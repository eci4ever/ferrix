# Classroom resource assignments

Platform administrators provision the classroom organization, invite the lecturer
as an organization admin (or transfer ownership), then keep production in a separate
organization. Lecturers remain ordinary platform users. No new student platform
role or global administration is required.

## Rollout and inheritance

Existing organizations default to legacy read-only member visibility. An organization
owner/admin can open Organization → Member resource visibility and enable
**Restrict members to assigned projects and services**. Enable this before inviting
students into a shared class project. Without restriction, unassigned members can
still read organization resources as before; assignments never remove inherited
or legacy access.

Use **Resource access → Manage access** on a project or service. Only existing
members of the same organization can be assigned. Select Viewer, Developer, or
No direct assignment to revoke. Owners/admins always retain management access.
Project grants cover all services in that project. Service grants cover only
that service and the parent project/environment metadata needed to navigate.
The strongest direct or inherited grant wins. In a shared class project, assign
students to individual applications, not to the project.

Assignments govern the Ferrix management API and UI. Access to a published
website still follows its public/LAN domain configuration and application login.

| Capability | Viewer | Developer | Organization manager |
| --- | --- | --- | --- |
| Permitted overview, runtime, metrics, history, logs, notes | Yes | Yes | Yes |
| Application environment and configuration secrets | No | Own assigned application | Yes |
| Application source, deploy/redeploy, start/stop/reload, notes edits | No | Own assigned application | Yes |
| Application browser terminal | No | Own assigned application | Yes |
| Resource limits, command, worker mode, storage settings | No | No | Yes |
| New projects/environments/services, move/clone/delete/rollback | No | No | Yes |
| Database credentials, environment, deployment or terminal administration | No | No | Yes |
| Domain and backup/restore administration | No | No | Yes |
| Assignments, organization settings and quotas | No | No | Yes |

Logs are permitted application output and may contain secrets printed by the
application itself. Keep credentials out of logs. Developer configuration edits
preserve the runtime/resource settings approved by the lecturer, including across
deployments. A developer changing Git repositories must provide their own token
or clear the saved token; they cannot reuse a saved token to fetch a different
repository. GitHub webhook secrets remain manager-only.

## APIs and enforcement

- `GET/POST /api/v1/auth/organizations/access`: `{ "restricted": true }`.
- `GET/POST /api/v1/projects/{id}/assignments`.
- `GET/POST /api/v1/services/{id}/assignments`.
- Assignment POST: `{ "user_id": "UUID", "permission": "viewer" }` or
  `"developer"`; `null` revokes that direct grant.
- `GET /api/v1/services/{id}/access` reports effective permission, `can_develop`
  and `can_admin` for the UI.

Grant/revoke and visibility changes require a recently verified manager browser
session and are audited. Resource routes authenticate first, then apply a central
member capability allowlist before reading bodies or performing mutations.
Lists, service counts, dashboard summaries and notifications use the same live
database policy. Resource-specific handlers also check access; unsupported member
operations are denied. Cross-organization grants are rejected at the API and
database constraint/trigger boundaries.

API-key scopes and project restrictions intersect with current resource grants.
Assigned application developers can request read/manage/deploy/secrets:read
scopes; delete is manager-only. Existing cookies and keys do not cache grants.
Removing membership cascades assignment records; re-inviting the user does not
restore those grants. Terminal connections recheck permission before accepting
input and every ten seconds while idle. Polling/log requests authorize each new
snapshot. Queued deployment and GitHub webhook workers recheck the initiating
user's current application permission before starting work.

Project restore/clone creates new resource IDs and does not copy assignments.
Resource grants are not restored from application/database backup artifacts.
Member removal and grant revocation do not erase application data or rotate an
already-disclosed application password. Shared MariaDB per-app accounts and their
credential lifecycle are tracked separately in issue #128.

## Verification

Run `cargo test -p ferrix-api --test resource_permissions -- --include-ignored`
with `FERRIX_TEST_DATABASE_URL` set to the current user's local database. The test
creates and drops an isolated schema; it does not change existing organizations.
It exercises real HTTP handlers and PostgreSQL for a non-platform-admin lecturer,
students A/B, restricted lists/counts/notifications, cookie and key revocation,
configuration ceilings, inherited grants, database administration denial and
cross-organization/forged assignments. Deployment requests are queued; this test
does not start Docker application containers or a PTY.

Frontend coverage is in `apps/web/e2e/resource-access.spec.ts` with API mocks.
It verifies manager assignment controls and developer configuration restrictions.

For real Docker and terminal verification, run
`python3 scripts/tests/resource-permissions-live.py` from this clone. It uses
the clone's own database in a disposable schema and starts a separate API on a
free loopback port. The already-installed `caddy:2-alpine` image is used for two
student applications. Both students deploy through the real worker and receive
HTTP 200 from their containers. A real WebSocket/PTY session executes shell
commands in A; B's shell verifies its own environment and separate filesystem.
A's requests to B's configuration, logs, deployment/control APIs and terminal
upgrade are rejected. Revoking A's assignment closes the existing shell and
blocks further input and reconnects without stopping either application.

The runner removes only its schema and UUID-labeled containers, networks and
volumes. It verifies that existing organization/project/service/assignment
metadata and previously running Docker containers are preserved. Temporary
credentials remain in memory; it does not use or change the user's browser login.
