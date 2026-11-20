# Ferrix authentication

This is a native Rust implementation of the self-hosted account, organization and session workflows used in Dokploy. It does not run Better Auth or implement its wire protocol. The React client calls the Ferrix API directly.

## Local setup

Fresh `scripts/setup-dev.py` environments include unique secrets. For an existing development environment, run:

```sh
python3 scripts/setup-auth-env.py
```

Keep `.env` private. `FERRIX_AUTH_ORIGIN` must be the exact browser origin, without a trailing slash, e.g. `http://127.0.0.1:5173`. For deployment, configure an HTTPS origin, preserve a random 32-byte hex `FERRIX_AUTH_KEY`. The API refuses missing/invalid configuration. Back up the encryption key with the database; losing it makes enrolled TOTP secrets unreadable. Do not rotate the key without migrating encrypted secrets.

Start PostgreSQL and the API from the clone root, then the web app as described in development.md. API startup applies the new migrations. Open the configured origin. The first-run screen asks only for a name, email and password. The first successful registration automatically becomes the platform administrator. A transaction with an installation lock permits exactly one first administrator and creates `My Organization` with an owner membership. Every project requires an organization; legacy unassigned projects must receive explicit ownership before migration 0011. Public sign-up is then closed.

No installation token is required. Existing FERRIX_SETUP_TOKEN entries in older .env files are ignored and can be removed. No default user or password is installed.

## Roles and organizations

System roles and organization roles have separate meanings:

| Role | Access |
| --- | --- |
| System admin | Installation user list, ban/unban ordinary users, revoke user sessions, issue password reset tokens, create organizations, Docker host inventory |
| Organization owner | Manage members and their roles, invite admins/members, read/write projects, view organization audit |
| Organization admin | Invite/remove members, read/write projects, view organization audit; cannot appoint admins/owners |
| Organization member | Read organization projects and members, manage own account/security |

The initial administrator is a system admin and an organization owner. Organization owners cannot be banned through the admin endpoint, and a system admin cannot ban itself or another system admin. The last owner cannot be removed or demoted. A system admin is not automatically a member of every organization. API authorization reads memberships and banned status on every request; organization IDs and roles supplied by the client are never accepted as authority.

Projects are filtered and inserted using the session's active organization. Members cannot write projects. Docker inventory is host-wide, so it is restricted to system admins. Organization switching verifies membership first. Member removal invalidates sessions active in that organization. Other memberships remain intact.

Invitations are bound to normalized email, expire after two days, and work once. A new invitation expires pending invitations for the same email/organization. Existing accounts must verify their current password and MFA before acceptance. Accepting an invitation cannot overwrite an existing membership role. The issuer receives the raw token once and shares it privately; mail delivery is not configured in this release.

## Passwords, sessions and security

- Argon2id password hashes with random per-password salt; 12-character minimum, 1024-byte maximum. Hashing runs in bounded blocking workers instead of Tokio request threads.
- Random 256-bit session tokens are stored only as SHA-256 hashes. Cookies are HttpOnly, SameSite=Lax and host-only. HTTPS uses Secure and a `__Host-` prefix. Development cookie names include the origin port so the three users' sessions do not collide on one host.
- Sessions expire after three days. Authenticated requests renew them after one day; renewal does not count as password reauthentication. An additional 30-day absolute lifetime bounds renewal.
- Security settings list live sessions and allow individual revocation, revocation of other sessions and logout. Revocation and bans are effective on the next request, with no cached identity/JWT fallback.
- Sensitive organization/admin actions require password verification within 15 minutes. Reauthenticate from Security when prompted. Password verification also checks MFA when enabled.
- Password change revokes every session and outstanding password reset tokens. Sign in again afterward.
- Password recovery is admin-assisted: a fresh system admin issues a 15-minute, single-use reset token. The recipient uses the sign-in screen's reset flow. It preserves MFA requirements, does not unban the account, and revokes all sessions after use. Self-service email recovery is not enabled.
- TOTP enrollment lasts 10 minutes and requires a password. The authenticator secret is encrypted with XChaCha20-Poly1305, random nonce, and the user ID as associated data. Enrollment becomes active only after verifying a code. Accepted time steps are recorded under a database lock to reject replay; one 30-second time step of clock skew is accepted.
- Eight random recovery codes are shown once, stored as hashes and consumed transactionally. MFA enable/disable revokes other sessions. Disabling requires the password plus a valid TOTP/recovery code. Keep recovery codes offline; no MFA-bypass endpoint exists.
- Unsafe API requests require an exact trusted Origin. No cross-origin credential sharing is enabled. Auth bodies are limited to 16 KiB and responses are `no-store`. Browser forms use JSON requests; tokens are not put in URL query strings or localStorage.
- Persistent throttles limit account credential attempts and peer-IP auth operations. Forwarded IP headers are ignored. Behind a reverse proxy, the peer limit applies to the proxy's IP; configure an upstream per-client limiter before public deployment rather than trusting arbitrary forwarding headers.
- Security activity is recorded for login/logout, organization and membership changes, invitations, password changes/resets, MFA changes and administrative operations.

Expired records are rejected at read time. Operators may periodically delete expired sessions, invitations, reset tokens, pending MFA records, expired throttle rows, and audit rows according to their retention policy. Such cleanup must target this installation's database only.

## API

All paths below are under `/api/v1/auth`. Unsafe methods are POST and require the configured `Origin` header, including CLI clients. Protected requests also need the session cookie. `/health` remains public; application API routes require authentication.

| Public endpoints | Purpose |
| --- | --- |
| GET `/setup` | Whether initial setup is required |
| POST `/setup` | First registration automatically creates the platform administrator and organization |
| POST `/sign-in` | Email/password and optional MFA code |
| POST `/accept-invitation` | Invitation token, name/email/password and optional MFA code |
| POST `/reset-password` | Reset token, new_password and optional MFA code |

| Protected endpoints | Purpose |
| --- | --- |
| GET `/session`, POST `/sign-out` | Current identity and logout |
| GET `/sessions`, POST `/sessions/{id}/revoke`, `/sessions/revoke-others` | Session management |
| POST `/reauthenticate`, `/change-password` | Verify identity and change credentials |
| GET `/security`, `/audit` | Account security and organization activity |
| POST `/mfa/enroll`, `/mfa/confirm`, `/mfa/disable` | TOTP lifecycle |
| GET/POST `/organizations`, POST `/organizations/switch` | Organization management |
| GET `/members`, POST `/members/{id}/role`, `/members/{id}/remove` | Membership management |
| GET/POST `/invitations`, POST `/invitations/{id}/revoke` | Invitation management |
| GET `/admin/users` | System user list |
| POST `/admin/users/{id}/ban`, `/revoke-sessions`, `/password-reset` | System administration |

POST bodies and role constraints are defined by the corresponding request types in `apps/api/src/auth`. Password confirmation/reauthentication bodies use `password` and optional `code`.

## Verification

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Use the DATABASE_URL from your own .env:
FERRIX_TEST_DATABASE_URL=postgres://ferrix:ferrix@127.0.0.1:5433/ferrix_nmfairus cargo test --workspace -- --include-ignored
cd apps/web && npm run build
```

The ignored integration test creates and removes an isolated schema in the specified development database. It exercises setup races, session cookies, password hashing, origin rejection, organization isolation, roles, invitations, bans, password changes, reset expiry/replay, MFA/recovery-code replay, session renewal/expiry and throttling. No production account is created by tests.

## Scope compared with Dokploy

Implemented: first owner, invitation-only additional accounts, system administration, fixed owner/admin/member organization roles, active organization, password login, TOTP/recovery codes, session and credential management.

Not included: OAuth/social login, passkeys/WebAuthn, SSO/SCIM, API keys, custom dynamic roles, automatic email delivery/verification, impersonation, organization deletion and account deletion. These require separate workflows; no stub or bypass endpoint pretends to support them.

Reference configuration reviewed: [Dokploy auth.ts on canary](https://github.com/Dokploy/dokploy/blob/canary/packages/server/src/lib/auth.ts), [Better Auth organization plugin](https://better-auth.com/docs/plugins/organization), [admin plugin](https://better-auth.com/docs/plugins/admin), and [OWASP session management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html). Dokploy uses bcrypt; Ferrix uses Argon2id. Dokploy self-hosted configuration disables Secure cookies; Ferrix instead requires them for HTTPS origins. The 30-day absolute session cap is an additional Ferrix control.

## Expired-record housekeeping

The API cleans expired sessions (either inactivity or absolute deadline),
throttle buckets, invitations, password-reset tokens and unconfirmed expired MFA
enrollments once at startup and every 15 minutes. Authorization still rejects
expired records immediately; cleanup timing never extends token validity.
Enabled MFA, recovery codes, users, memberships, audit and deployment history
are retained. Accepted invitations are removed only after their expiry time.

Each target is processed in batches of 1,000, at most 10 batches per sweep.
Each batch commits separately, skips locked rows and has a five-second statement
limit plus a one-second lock timeout. Active token consumption/renewal is not
blocked by housekeeping; concurrent cleaners can safely process separate rows.
Idle cleanup does not reserve a pool connection, and API shutdown stops the worker.
Large backlogs drain over subsequent sweeps. Errors are logged and retried on the
next interval; no cleanup tables or external scheduler are required.
