# Organization API keys for deployment agents

Ferrix implements API keys natively in Rust/Axum and PostgreSQL. An API key
represents one user in one organization, independent of browser sessions or the
browser's currently selected organization. Create a separate key per AI agent.

## Create and use a key

Open **API keys** in the sidebar, choose a name, expiry and permissions, then
save the credential shown once. Creation and revocation require browser login
and identity verification within the last 15 minutes; use Security to verify
password and TOTP if prompted. Store the key in the agent's secret store as
`FERRIX_API_KEY`, and configure `FERRIX_URL` to your Ferrix origin. Use HTTPS on
remote servers; HTTP is for local loopback development only.

Each API call sends `Authorization: Bearer <key>`. Do not send session cookies,
put the key in a URL, commit it to source control, or print it in command logs.
Keys cannot create other keys, sign in as a browser, manage users/organizations,
or access Docker host administration. Even a platform admin's key is restricted
to the explicitly permitted deployment API routes.

## Permissions

| Scope | Allowed operations |
| --- | --- |
| `read` (required) | Projects/environments/services, deployment history, runtime, logs, backup/restore status, organization summary, agent discovery |
| `manage` | Create projects/environments/applications/databases; update service configuration and environment variables |
| `deploy` | Queue deployments, start/stop/reload services and create database backups |
| `delete` | Delete services and empty projects using their existing confirmation payloads |
| `secrets:read` | Read service configuration/environment, including database passwords and environment secrets |

**Manage/deploy can execute container code with saved service secrets.** The
`secrets:read` scope controls direct configuration reads, not isolation from code
a trusted deployment agent is allowed to run.

Database restore requires both `manage` and `deploy` because it creates a new Database service in the same environment. Use `POST /api/v1/services/{id}/backups/{backup_id}/restore` with `{ "name": "Restored database" }`, then poll `GET /api/v1/services/{id}/backups` until that operation succeeds. Backups can be queued with `POST /api/v1/services/{id}/backups`. Only PostgreSQL and MariaDB are supported. Keep the returned new service ID and wait for restore completion before connecting an application.

The default owner/admin key has `read`, `manage`, `deploy`; delete and secret
reads are opt-in. Organization members can create read-only keys. A platform
`user` who is an organization `owner/admin` can manage and deploy with a key;
platform role is not a substitute for organization permission. Current user
status, membership and organization role are checked on every request. A role
demotion immediately limits a previously issued key. Deployment/application logs
can themselves contain application secrets; treat read access accordingly.

## Agent workflow

Start with `GET /api/v1/agent`. Read `GET /api/v1/agent/docs` for this guide
as Markdown; it is bundled in the server and accessible to keys with read scope. It returns the key ID, effective `scopes` (and stored `granted_scopes`), organization and
current organization role, plus a deployment workflow. Stop if the role/scopes
do not authorize the requested work. The response never returns the credential.

1. `GET /api/v1/projects`, or `POST /api/v1/projects` with
   `{"name":"Agent project","description":"Created by my agent"}`.
2. `GET /api/v1/projects/{project_id}/environments` returns
   `{"project_name":"…","environments":[…]}`. Projects start with production.
   Add an environment with `POST` to the same URL and `{"name":"staging"}`.
3. `GET /api/v1/environments/{environment_id}/services` lists services. Create an
   application with `POST` and
   `{"kind":"application","name":"web","image":"nginx:alpine","container_port":80,"environment":""}`.
   For MariaDB use `{"kind":"database","name":"db","engine":"mariadb","database_name":"app","username":"app"}`.
   Database credentials are generated server-side; reading them needs `secrets:read`.
4. `POST /api/v1/services/{service_id}/deploy` with `{}` returns HTTP 202 and
   `{"id":"deployment-id","status":"queued"}`. Deploying preserves saved config.
5. Poll `GET /api/v1/services/{service_id}/deployments` every few seconds until
   that deployment is `succeeded` or `failed`. Read `/runtime` and the service
   record to obtain runtime state, host port and automatic domain.

Optional endpoints:

- `POST /services/{id}/control` with `{"operation":"start"}`, `stop` or `reload`
  needs `deploy`.
- `GET/POST /services/{id}/configuration` and `/environment` read/update saved
  settings. Reads require `secrets:read`, writes require `manage`. Preserve all
  fields/environment values you intend to keep; saving configuration is a
  replacement, not a patch. Refer to the current service form for engine fields.
- Application aliases under `/applications/{id}` remain supported where present.
- Deleting a service requires `delete` and its exact name:
  `DELETE /services/{id}` with `{"name":"web","remove_data":false}`.
  Removing persistent data requires an explicit user decision.
- Deleting an empty project requires `DELETE /projects/{id}` with
  `{"name":"Agent project"}` and `delete` permission.

All paths above are relative to `/api/v1`. Resource UUIDs are resolved within the
key's organization; changing a URL or browser organization cannot cross tenants.
Project listing currently returns the latest 100 projects; use an explicit known
project ID when working with older projects rather than creating a duplicate.

## Runnable example

After injecting `FERRIX_URL` and `FERRIX_API_KEY` into the agent's environment:

```sh
python3 scripts/agents/deploy-image.py \
  --project "Agent project" --application web --image nginx:alpine --port 80
```

The standard-library Python client finds/creates a project and application,
queues a deployment and waits for its result. It emits a JSON summary without
credentials or deployment logs. Use `--project-id` to disambiguate existing
projects. Existing application config is preserved: if image/port/kind differs,
the client stops and asks for an explicit configuration update. It does not
follow redirects, send cookies, automatically retry writes or delete resources.
A timeout does not cancel a queued/running deployment.

## Lifecycle, limits and audit

- Keys contain 256 random bits with an `frx_` prefix. Only a SHA-256 hash and a
  short display prefix are stored; the full credential is shown only on creation.
- Mandatory expiry: 1–365 days, UI default 90. At most 20 active keys per user
  across all organizations. UI lists the latest 100 with active keys first.
- Each key permits 120 requests per fixed UTC minute. HTTP 429 includes
  `Retry-After`; invalid credentials are separately throttled per direct peer IP.
  Behind a reverse proxy, configure an upstream client-IP limiter as appropriate.
- HTTP 401 means missing/invalid/expired/revoked credentials. Stop and request a
  valid key. HTTP 403 means insufficient scope/role or a prohibited endpoint;
  never try a browser-login fallback. HTTP 404 may indicate another organization's
  resource. Do not blindly retry 409 conflicts or failed writes.
- Revocation affects subsequent requests immediately, not work already queued.
  Logout does not revoke keys. Password changes/resets, bans and membership/user/
  organization removal invalidate them. Restoring access does not restore a
  revoked/deleted key. Platform admins can revoke all keys for a user from Users.
- Usage records contain key/user/org IDs, matched route template, resource UUID,
  method, response status, direct peer IP and timestamp. No request bodies, query
  strings, credentials or response contents are retained. Rate-limit overflow is
  recorded once per window rather than logging an unbounded flood.
- Usage is retained for 30 days and cleaned by the existing bounded housekeeping
  worker. The UI shows the latest 100 records per owned key. Audit changes to key
  creation/revocation are retained in the existing organization audit.
- Browser cookie authentication keeps exact-Origin CSRF protection. Valid bearer
  requests can omit Origin; if provided it must match the configured origin.
  Mixed key/session credentials and duplicate authorization headers are rejected.

Management endpoints are browser-session only: `GET/POST /auth/api-keys`,
`POST /auth/api-keys/{id}/revoke`, `GET /auth/api-keys/{id}/usage`, and the
platform-admin `POST /auth/admin/users/{id}/revoke-api-keys`.

References: [OWASP REST security](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html),
[Axum matched-path middleware](https://docs.rs/axum/latest/axum/extract/struct.MatchedPath.html).
