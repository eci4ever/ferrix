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
| POST `/sign-in` | Email/password; returns a separate MFA challenge when enabled |
| POST `/sign-in/verify`, `/sign-in/cancel` | Complete or cancel the browser-bound MFA challenge |
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
| GET/POST `/organizations/current`, POST `/organizations/transfer`, `/leave`, `/delete` | Organization profile, ownership and membership lifecycle |
| GET `/members`, POST `/members/{id}/role`, `/members/{id}/remove` | Membership management |
| GET/POST `/invitations`, POST `/invitations/{id}/revoke`, `/invitations/{id}/renew` | Invitation management |
| GET `/admin/users`, `/admin/users/{id}` | Paginated user search and account details |
| POST `/admin/users/{id}/ban`, `/revoke-sessions`, `/password-reset`, `/role`, `/delete` | System administration |

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

Not included: OAuth/social login, passkeys/WebAuthn, SSO/SCIM, custom dynamic roles, automatic email delivery/verification, impersonation, self-service account deletion and custom roles. These require separate workflows; no stub or bypass endpoint pretends to support them.

Reference configuration reviewed: [Dokploy auth.ts on canary](https://github.com/Dokploy/dokploy/blob/canary/packages/server/src/lib/auth.ts), [Better Auth organization plugin](https://better-auth.com/docs/plugins/organization), [admin plugin](https://better-auth.com/docs/plugins/admin), and [OWASP session management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html). Dokploy uses bcrypt; Ferrix uses Argon2id. Dokploy self-hosted configuration disables Secure cookies; Ferrix instead requires them for HTTPS origins. The 30-day absolute session cap is an additional Ferrix control.

## Expired-record housekeeping

The API cleans expired sessions (either inactivity or absolute deadline),
throttle buckets, invitations, password-reset tokens, expired login challenges and unconfirmed expired MFA
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

## Profile and current-session protection

Profile updates accept only name, avatar and profile image URL. The sign-in
email is read-only in the UI; including an email change in a profile request is
rejected. The Sessions table marks the current session as Protected and offers
Revoke only for other sessions. The API also rejects revoking the current
session through the session-management endpoint. Use Sign out to end it;
password changes and security revocations retain their existing behavior.

## Authenticator QR and two-step sign-in

In **Security → Two-factor authentication → Set up authenticator**, a dialog
walks through password confirmation, scanning the TOTP QR, entering its six-digit
code and saving recovery codes. The recovery step stays open until acknowledged. The QR is generated
locally in the browser from the enrollment URI, without an external QR service.
A manual setup key remains available. Enrollment expires in ten minutes and MFA
is enabled only after a valid code. Save the eight recovery codes shown once;
each is single-use and only its hash is stored. Enrollment secrets stay encrypted.

`POST /api/v1/auth/sign-in` accepts email/password. With MFA disabled it issues a
session. With MFA enabled it returns `two_factor_required: true` and
`expires_in_seconds: 300`, setting a separate HttpOnly, host-only, SameSite cookie
(Secure on HTTPS). It does not issue an authenticated session. Submit `{code}`
to `POST /api/v1/auth/sign-in/verify`, using that browser cookie, with a fresh
TOTP or recovery code. Success consumes the challenge, issues a new session and
clears the temporary cookie. `POST /api/v1/auth/sign-in/cancel` consumes the
browser's pending challenge and clears the cookie.

Proof tokens are random, stored hashed, expire after five minutes and allow
five attempts each. An additional account-wide MFA limit permits ten requests
per fifteen-minute bucket; password and IP limits also apply. Password or MFA
secret changes and bans invalidate pending proofs. Duplicate/forged cookies,
expired proofs, code replay and concurrent reuse are rejected. A 410 response
returns the UI to password entry. Passwords and proof tokens are never persisted
in browser storage. Reset/invitation acceptance still requires MFA when enabled.

Reference: [OWASP multifactor authentication guidance](https://cheatsheetseries.owasp.org/cheatsheets/Multifactor_Authentication_Cheat_Sheet.html).
The disposable browser suite decodes the actual rendered QR and tests TOTP and
recovery-code sign-in. Run `python3 scripts/tests/services-ui.py` with the local
Node runtime and Docker environment described in development.md.

## Users and organization management

**Users** uses server-side literal substring search (`q`), platform-role and
active/banned filters, and zero-based pagination (`page`, `limit`, default 10,
maximum 50). `GET /auth/admin/users` now returns `{items,total,page,limit}` rather
than a capped array. User details include organizations and up to 100 active
sessions, without passwords, tokens or MFA secrets. The UI uses TanStack Table
with manual server pagination for Users and local pagination for smaller lists.

Fresh platform admins can change `admin/user` platform roles, issue reset tokens,
revoke sessions, ban/unban and delete accounts. Platform roles are separate from
organization `owner/admin/member` roles. Bans require a reason up to 500 characters
and may have a future expiry within one year. Sessions are revoked immediately;
expired temporary bans allow login immediately without waiting for housekeeping.
Admin accounts and organization owners cannot be banned. The current session is
excluded from admin session revocation too.

A role change cannot demote the last active administrator with organization
access, and banned or organization-less accounts cannot become platform admins.
Deletion requires the exact email, cannot target the acting admin, and blocks
organization owners until ownership is transferred. Deletion removes memberships,
sessions and invitations issued by that user; existing audit rows remain with
nullable references. Profile email remains immutable.

**Organization** provides name/HTTPS logo editing for owners/admins; searchable
members and invitations; invitation cancellation and regeneration (old token
invalidated); explicit ownership transfer; leaving; and deleting empty
organizations with exact-name confirmation. Transfer promotes an active existing
member to owner and demotes the acting owner to organization admin. Last owners
cannot leave; platform administrators must retain membership in another
organization before leaving/deleting their last one. Deletion never cascades
projects: all projects must be removed first. Sessions active in a departed or
deleted organization become invalid; sign in again to access other memberships.
Logo URLs are loaded by the browser, not fetched by the API.

Mutations require fresh verification (15 minutes). They share a transaction-scoped
management lock and recheck current authority under that lock, protecting
concurrent owner/admin changes. The coarse lock covers only low-volume account,
membership and invitation transitions, not application deployment traffic.
Administrative and membership changes produce audit events. API tests cover
concurrent admin demotions, last-owner protection, tenant boundaries, temporary
ban expiry, token rotation and deletion constraints. The disposable browser suite
exercises the user and organization management flows against a real isolated API.

The installation's default organization is permanently marked by `is_default`.
Its Delete action is disabled and the API rejects deletion even after renaming
or joining another organization. Migration 0016 identifies existing default
organizations through the installation audit event, falling back to the earliest
surviving organization. New organizations remain deletable under the usual checks.

## Agent API keys

Organization-bound API keys now support deployment automation. Keys have explicit
read/manage/deploy/delete/secrets-read scopes, mandatory expiry, live membership
and role checks, rate limits and 30-day usage metadata. Browser-only API key
management lives at `/api-keys`; platform admins can revoke all user keys from
Users. Password changes/resets and bans revoke keys; logout does not. See
[API key and agent guide](api-keys.md) for the complete API contract and runnable
Python deployment example.
