# Development with three macOS users

Each user uses their own clone at `/Users/Shared/Workspace/TS/<user>/ferrix`, where `<user>` is `nmfairus`, `mac2` or `mac3`. The original `/Users/Shared/Workspace/TS/ferrix` is retained as a legacy setup checkout; do not run it alongside the new nmfairus clone because they share a port profile. Use each user's existing GitHub authentication and Git identity; credentials are not copied between users.

| User | API | Web | PostgreSQL | Database | Compose project |
| --- | --- | --- | --- | --- | --- |
| nmfairus | 8080 | 5173 | 5433 | ferrix_nmfairus | ferrix-nmfairus |
| mac2 | 8081 | 5174 | 5434 | ferrix_mac2 | ferrix-mac2 |
| mac3 | 8082 | 5175 | 5435 | ferrix_mac3 | ferrix-mac3 |

From the clone root, run once:

```sh
python3 scripts/setup-dev.py
```

The script detects the current user and creates a private, ignored `.env`. It preserves an existing `.env` by refusing to overwrite it. Vite reads that root file for its port and API proxy; Axum reads it when started from the root. Compose uses it for project identity, PostgreSQL port and database. Volumes and networks are scoped by the Compose project.

```sh
docker compose up -d postgres --wait
cargo run -p ferrix-api
```

In another terminal:

```sh
cd apps/web
npm ci
npm run dev
```

For PostgreSQL integration tests, set `FERRIX_TEST_DATABASE_URL` to the `DATABASE_URL` in your own `.env`. Never point tests at another user's database.

Stop only your own services with `docker compose stop` from your clone. Do not use `down -v` unless intentionally deleting your own development data. The previous project named `ferrix` and its volume are legacy data; leave them intact until nmfairus chooses to migrate or remove them.

The credentials in Compose are local development credentials. Each user must have Docker access in their own login session. On macOS, availability of Docker/OrbStack across separate login sessions depends on the installed runtime; separate Compose projects isolate resource names, not access to the daemon. Container inventory can still show other users' containers on a shared daemon.

Create one task branch per change, push it, and open a PR to main. Coordinate overlapping work before editing. Only one agent may write to a given checkout at a time.

## Git access in the shared folders

The initial clones were created by nmfairus and grant the named user inherited filesystem access through macOS ACLs. mac2 and mac3 should each run this once in their own login session to mark only their assigned clone as trusted:

```sh
git config --global --add safe.directory "/Users/Shared/Workspace/TS/$(id -un)/ferrix"
```

Do not use a wildcard safe.directory. Existing GitHub credentials and commit identity remain unchanged. Install frontend dependencies using `npm ci` in `apps/web` from each user's login. Setup changes are present locally in each clone and have not been committed or pushed; nmfairus should integrate the canonical setup once, then the other users can reconcile their identical setup changes before starting feature branches.

### Dashboard server clock

The dashboard reads authenticated `GET /api/v1/server-time` for the server timestamp, IANA timezone and current UTC offset. It ticks locally between synchronizations every 60 seconds and synchronizes again when the tab becomes visible. Clock requests validate the session without extending its inactivity expiry. The timezone follows a valid `TZ` environment override (for containers), otherwise the server OS, falling back to UTC if unavailable.

## Pull request CI

### Release versions

The sidebar shows the running API version from `workspace.package.version` in
the root `Cargo.toml`. Before tagging a release, update that version and the
`ferrix-api` entry in `Cargo.lock` together, merge the PR, and tag that commit
with the matching `v` prefix (for example `0.3.0` → `v0.3.0`). The API inherits
the workspace version; the private frontend package version is not displayed.
Release CI checks the exact tagged source and lockfile before building or
publishing an image, including manual release runs. A mismatched tag fails
rather than publishing an image whose sidebar reports another version.
Existing published tags are immutable; this change does not rewrite v0.2.0
or replace an already published image. Source builds need an API rebuild/restart
after a version change; installed releases need the corrected image.

### Checks

The `CI` GitHub Actions workflow runs on every pull request, including updates,
on pushes to `main`, and can also be started manually. A small change-detection
job selects the existing build jobs from the complete Git diff:

| Changed files | Checks selected |
| --- | --- |
| `apps/web/**` (including npm lockfile, vendor and assets) | Frontend |
| `apps/api/**`, `Cargo.toml`, `Cargo.lock`, `.cargo/**`, Rust toolchain/lint config | Rust and PostgreSQL |
| Workflow files, scripts, Dockerfile/Compose configuration or unrecognized shared paths | Both |
| Only `docs/**`, root Markdown files or license files | Neither build job |
| Manual run, unavailable diff or failed detection | Both |

PRs use the merge-base (three-dot) diff; pushes compare the complete before/after
range. Renames include both source and destination, and deletions are included.
The workflow itself always starts and preserves the required check names:
`Frontend build` and `Rust checks and PostgreSQL tests`. Unneeded jobs are skipped
at job level, avoiding pending checks caused by workflow-level path filters.
Detection tests run on every invocation; a failed detector cannot skip builds.

Selected jobs check Rust formatting, Clippy with warnings denied, all Rust tests
(including ignored PostgreSQL integration tests), and the frontend production
build. Rust uses the stable toolchain; the frontend uses Node.js 24 and npm
12.2.2 with the committed lockfile. To test the CI scripts locally (Python 3.11 or newer):

```sh
python3 -m unittest discover -s scripts/ci -p 'test_*.py' -v
```

Each Rust job gets a temporary PostgreSQL 17 service and its own `ferrix_ci`
database. No developer `.env`, external database or repository secrets are needed.
The integration test applies migrations in an isolated schema. New commits cancel
an older CI run for the same pull request.

### Docker Swarm inventory

The administrator-only Docker → Swarm page has Overview, Nodes and Services views.
`GET /api/v1/docker/swarm` reads Docker info and lists nodes only when the
connected daemon is an active Swarm manager. Ready nodes are counted separately
from scheduling availability (active/pause/drain). CPU and RAM are node capacity,
not current utilization. Node responses include only display fields; labels,
certificates and join tokens are excluded. Inactive, transitional and worker
states display an explanation with unavailable counts rather than invented zeros.
Refresh swarm fetches a new snapshot; this page never initializes Swarm or changes cluster membership.
Services are read from the manager with `status=true` for Docker's running,
desired and completed task counts. The searchable table shows deployment mode,
image, published ports and update state; missing counts stay unknown. Completed
job tasks are displayed separately from running tasks. No service environment,
secret/config references, mounts or labels are returned. An empty manager
inventory is distinguished from unavailable inventory or missing manager access.
Refreshing preserves the selected Swarm view.

System administrators can scale ordinary Replicated services from the Services
table. The confirmation dialog accepts 0–100 replicas and warns that zero stops
all tasks. Global and job modes cannot be scaled here. The POST endpoint at
`/api/v1/docker/swarm/services/{id}/scale` checks the active-manager state and
the inspected service version before updating only the replica count in the
existing spec. Docker receives the same version for its concurrent-write guard
and retains registry authorization from the current spec. Stale versions require
refreshing the inventory. Successful requests trigger a refresh; task convergence
is asynchronous, so refresh again to check the running/desired counts. Timeouts
may occur after Docker accepts an update; check the inventory before retrying.

Restart uses a separate confirmation dialog for ordinary Replicated and Global
services. `POST /api/v1/docker/swarm/services/{id}/restart` requires the same
system-admin, active-manager and optimistic-version checks. It increments
`TaskTemplate.ForceUpdate` without changing replicas, task configuration or the
service update policy. Docker replaces tasks asynchronously; existing connections
may be interrupted. Job modes are excluded to avoid accidentally rerunning work.
The UI reports the request as accepted and refreshes inventory; refresh again
until running/desired counts recover. A service with zero desired replicas stays
at zero after restart. Refresh before retrying failed or timed-out requests.

View logs opens a shadcn dialog with an on-demand snapshot from
`GET /api/v1/docker/swarm/services/{id}/logs`. System-admin and active-manager
checks apply to every read. Docker is asked for stdout/stderr with timestamps,
200 tail lines per task and no follow stream. The read has a five-second timeout,
a 1 MiB received-data budget and a 64 KiB display cap with a truncation notice.
Logs stay plain text (ANSI styling is stripped in the UI); credentials logged by
applications are not automatically redacted. Responses use `Cache-Control:
no-store`. Missing services and empty logs have explicit displays; read errors
include guidance about Docker connectivity and logging drivers. Refresh logs fetches a new snapshot;
closing the dialog aborts the browser request. No tasks are restarted or scaled.

Swarm log dialogs auto-refresh by default, scheduling the next snapshot three
seconds after the previous request completes (no overlapping reads). Pause
auto-refresh cancels the current browser request and retains the last snapshot;
manual Refresh logs still works while paused. Resume fetches immediately.
Hidden tabs do not start new reads, and visible tabs resume polling. Closing
the dialog aborts requests and removes timers/listeners. Transient errors retain
the last successful snapshot with an error notice; 401/403/404 responses clear
it and pause polling. Snapshot timestamps show when data was last received.
Log polling validates sessions without extending their inactivity expiry.

### Host monitoring

System administrators can open **Monitoring** (`/monitoring`). `GET /api/v1/monitoring`
is session protected and checks the system administrator role; organization owners
without that role cannot access host data.

One background OS collector samples CPU, memory, root filesystem capacity, mounted
block-device throughput and non-loopback network throughput every five seconds.
The cache keeps 180 samples (15 minutes) in memory and resets on API restart.
Requests read the cache without running probes. OS collection runs on one dedicated
thread so a slow probe cannot block HTTP requests or create an unbounded queue.
Docker storage uses the configured Docker connection, a three-second timeout and
a separate 60-second cache. Failed probes retain the last reading with an error
and timestamp. The page flags host readings older than 20 seconds.

CPU and memory are the API's OS view, not cgroup limit accounting. Filesystem
capacity is the root mount's total minus available bytes; on APFS this reflects
shared capacity consumption rather than just the read-only system snapshot.
Block I/O deduplicates mounted devices by name; virtual interfaces and layered
block devices can still overlap. The first CPU/rate sample and reset counters
have gaps. Docker storage includes all objects on the connected daemon, including
other projects. It is separate from API-host storage and does not measure Docker
VM disk allocation. Image layers are deduplicated by Docker; build cache remains
separate because it can share image layers. Unsupported volume sizes stay unknown.

Docker storage reads the raw `/system/df` response with a dedicated HTTP client,
leaving the existing Bollard dependency and deployment code unchanged. Bollard
0.20's generated storage model discards the legacy fields still returned by Docker.
The probe honors `DOCKER_HOST` Unix sockets and HTTP/TCP endpoints (defaulting to
`unix:///var/run/docker.sock` like the API). Proxies and redirects are disabled;
responses are capped at 8 MiB and three seconds. Unsupported transports report
unavailable. An HTTP fixture test protects legacy storage-field compatibility.
No database migration is required.

### Delete projects

Organization owners/admins can delete projects from either the card or table
view. The dialog requires the exact project name and blocks projects that still
contain services. `DELETE /api/v1/projects/{id}` enforces the same checks on the
server, scopes access to the active organization and deletes only an empty
project and its environments. The transaction locks the project and environment
rows before checking for services, preventing concurrent service/environment
creation from bypassing the check. Remove services explicitly first; retained
Docker volumes are not removed by deleting the project.

### Project runtime summaries

Project cards poll authenticated `GET /api/v1/projects/service-status` every ten
seconds while the tab is visible. The endpoint covers the same 100 latest projects
as the project list and all their environments. It uses one organization-filtered
Docker inventory with a five-second timeout and verifies active container IDs and
ownership labels. Polling does not renew session inactivity expiry.

Running means an active running container without an unhealthy/starting check.
Stopped includes exited/created containers. Failed includes dead/unhealthy or
missing activated containers and failed first deployments. A failed redeployment
does not count as failed while the previous active container is running. Other
includes undeployed, starting, restarting, paused and unknown states. If Docker is
unavailable, affected projects show unavailable instead of zero counts. These are
service counts, not deployment counts; no schema migration is needed.

### Home summary

Authenticated `GET /api/v1/home-summary` returns counts for the session's active
organization, including ordinary members. Projects and services cover all rows,
independent of the Projects list's 100-row limit. Service totals include
applications and databases across every environment.
Deployments count requests created in the last 24 hours, broken down into
succeeded, failed and queued/running. Projects created in the last seven days
are counted separately.

Needs attention counts services whose latest deployment (creation time, then ID)
failed, regardless of age; a newer successful or pending deployment clears that
service from this list. Undeployed services are not failures. Up to five affected
services are returned with project IDs for navigation; the count is uncapped.
This is deployment history, not live container health. No Docker probe or
host-wide data is used. A single SQL statement gives all counts one snapshot.

### Data tables and service cards

Containers, Users, Sessions and Deployment History share a TanStack Table v9 adapter rendered with shadcn Table, InputGroup and Button components. It provides case-insensitive search over accessor values, sortable headers with `aria-sort`, ten-row pagination, stable record IDs and empty results. Sorting/search reset to page one; data refreshes preserve the current page where possible and clamp it after removals. Actions keep the original row ID after sorting/filtering. These operations apply to the API records already loaded, not server-side pagination; deployment history remains limited to the latest 30 records. Tables scroll horizontally inside their container on narrow screens.

Project services remain cards. Their title anchor covers the card for pointer/keyboard navigation and browser open-in-new-tab behavior; Delete remains a separate button above that link. There is no Open button. Browser tests exercise table sorting/search/pagination, correctly targeted actions, card navigation/deletion and mobile overflow.

### Cargo cache in GitHub Actions

The Rust job restores Cargo registry/git dependencies and compiled dependency artifacts through `Swatinem/rust-cache`, pinned to a reviewed commit. The action runs after installing stable Rust and uses that same explicit toolchain for its cache key. Compiler version, Cargo manifests/lockfile and Rust build environment contribute to cache invalidation.

Successful `main` runs save the reusable cache; PRs restore it without saving their own branch copies. A push to `main` warms the default-branch cache for future PRs. The first run or a toolchain/dependency change may still require compilation. Formatting, Clippy and all integration tests always run; Docker/database startup and test workloads are not cached. Installed Cargo binaries and the project’s own compiled crates are excluded by the action settings/defaults; compiled dependency artifacts remain cached.

This cache is stored in GitHub Actions, not developers’ local clones. Local `.env`, auth keys, PostgreSQL/Docker data and volumes are not cache paths. No local development configuration changes are needed.

## Automatic website domains

For an existing clone, run `python3 scripts/setup-domains.py` once. Fresh
`setup-dev.py` setups include these settings. Start the private public-site
listener by restarting the API, then start Caddy:

```sh
docker compose --profile websites up -d proxy
```

| User | Private website listener | Local Caddy HTTP |
| --- | --- | --- |
| nmfairus | 8090 | 80 |
| mac2 | 8091 | 9081 |
| mac3 | 8092 | 9082 |

Docker Application pages show an **Automatic domain**.
Use **Open website** to run JavaScript, modules, forms and root-relative assets
on a separate origin. Local URLs use `app-<uuid>.localhost:<port>`. Port 80 is omitted from nmfairus URLs, so they open as `http://app-<uuid>.localhost/`
Port 80 is reserved for nmfairus; mac2 and
mac3 retain their separate ports to avoid conflicts.
UUIDs remain stable across redeploys.
Deploy an Application first. Applications deployed
before automatic domains were configured need one redeploy to join ingress.
Database services are not automatically exposed.

The ingress network and its ownership label are scoped to the Compose project.
Never reuse another user's network. Use the Application's automatic URL for
interactive testing.

`python3 scripts/tests/automatic-domains.py` exercises real Caddy, Docker and
Playwright with disposable resources in the current user's database. It also
checks the production HTTPS routes with a test certificate authority; public
DNS and ACME issuance require a real VPS.

### Local HTTPS

The nmfairus profile uses HTTPS on port 443 and redirects HTTP on port 80.
Automatic URLs become `https://app-<uuid>.localhost/` and
`https://app-<uuid>.localhost/`. Other users retain their isolated HTTP ports.
`FERRIX_WEBSITE_HTTPS=true`, `FERRIX_WEBSITE_HTTPS_PORT=443` and
`FERRIX_LOCAL_CADDYFILE=Caddyfile.local-https` select this mode.
Caddy stores its local certificate authority in the project-scoped
`ferrix_caddy_data` volume. Preserve this volume so the trusted root stays stable.

Copy only the public root certificate (never its private key) from the proxy:

```sh
docker compose cp proxy:/data/caddy/pki/authorities/local/root.crt /tmp/ferrix-local-root.crt
security add-trusted-cert -r trustRoot -k "$HOME/Library/Keychains/login.keychain-db" /tmp/ferrix-local-root.crt
```

macOS may request permission to trust the certificate. Restart the browser
after installing it. Firefox with a separate trust store may need its own
certificate import. Trust applies to this user's local Caddy authority.
See [Caddy local HTTPS](https://caddyserver.com/docs/automatic-https#local-https).

### Local authentication address

Open the dashboard at the exact `FERRIX_AUTH_ORIGIN` from your `.env`. Vite
redirects document visits from equivalent loopback names (`localhost`,
`127.0.0.1`, `::1`) to that address on the same port, preserving the path and
query. This keeps cookies and form origins consistent for login and account
setup. API requests are not redirected; strict backend Origin checks
remain in place. Other users' ports and production domains are not affected.

### Docker-only scope and retired data

Ferrix supports Application and Database services. Static websites run inside
Applications using Nginx/Caddy images; automatic domains and HTTPS remain.
Shared/student Hosting, Compose services and the dedicated Static Website
service have been removed from the UI, API, runtime and release workflow.

Historical migrations remain unchanged for existing installations. Migration
0009 deactivates publicly registered student accounts; migration 0010 removes
those old accounts/metadata, Compose service records and Static Website stored
releases/files. Administrators, ordinary team accounts, Application/Database
services and their projects remain intact. Cleanup is permanent.

For existing Docker resources, scope cleanup to the current installation's old
Hosting instance label or retired Compose application UUIDs. Remove retired
containers before their labeled volumes/networks. Never use a global prune or
remove another user's resources. Keep PostgreSQL, Caddy certificate storage,
ingress and active Application/Database volumes.

### Database integrity and indexes

Migration 0011 requires every project's `organization_id` and adds a deferred
composite foreign key from `(applications.id, active_deployment_id)` to
`deployments(application_id, id)`. A service cannot activate another service's
deployment or delete its current deployment independently. Activation, redeploy
and cascading service/project deletion remain transactional. Existing orphan
ownership or invalid active pointers stop migration without silently reassigning
or deleting data; fix those records explicitly before retrying startup.

Membership lookups use `(user_id, organization_id)` and organization audit queries
use `(organization_id, created_at DESC)`. Additional expiry indexes support bounded
auth housekeeping, including `LEAST(expires_at, absolute_expires_at)` for sessions
and a partial index on unconfirmed MFA enrollments. Existing tenant/project,
deployment queue and history indexes remain in place.

PostgreSQL regression tests cover invalid ownership/pointers, deferred activation,
cascading deletes, failed-migration recovery, expiry preservation, locked renewals,
bounded cleanup, concurrent cleaners and worker startup/shutdown. Run against your
own test database with `cargo test --test database_integrity -- --include-ignored`.

### Organization deletion and sessions

Deleting an empty, non-default organization moves only the requesting browser
session to an organization the user already belongs to. The default organization
is preferred, then the oldest remaining organization (with its ID as a stable
tie-breaker). Session expiry and recent password verification are unchanged.
Other sessions still using the deleted organization are revoked by the database
cascade. If the requesting user has no remaining membership, their session is
also revoked. The reassignment and deletion commit in one transaction; rejected
deletions leave the session unchanged. Leaving an organization still revokes
sessions using it.

### Service notes

Application and Database workspaces include a Notes tab. Notes are plain text,
shared with members of the service's organization, with a 32 KiB UTF-8 byte limit.
Owners/admins can save or clear notes; members can read them. Do not put secrets
in notes. Updates take effect immediately and do not change deployment settings
or deployment snapshots. Drafts survive switching workspace tabs, but are not
saved until **Save notes** is used.

`GET /api/v1/services/{id}/notes` returns `{ notes, revision }`.
`POST` to the same route requires both fields and the current revision. A stale
revision returns 409 without overwriting saved text. The UI retains the local
draft so the user can copy it before loading the latest saved notes. API keys
require `read` for GET or `manage` for POST; organization role and project scope
remain enforced. Migration 0032 initializes existing services with empty notes.


### Moving and cloning services

Service Overview includes **Move service** and **Clone to environment** dialogs
for organization owners/admins. Destinations are other environments in the same
project. API keys need `manage`; project and organization boundaries still apply.
Both operations reject pending deployments, backup/restore and project maintenance.

Move preserves service identity, notes, domains, history and named data volumes.
Stop the service first. Move removes its stopped active container without deleting
volumes, then clears the active runtime pointer. Deploy again to attach to the new
environment network and create correct Docker labels. Other services, connection
strings and environment variables are not moved or rewritten automatically.

Clone creates an undeployed service from saved configuration, including CPU/RAM,
command, worker mode, storage settings, notes and encrypted environment variables.
GitHub build configuration/token is re-encrypted for the new service; auto-deploy
and webhook credentials/deliveries are reset. Uploaded source archives are copied
to an independently owned upload. Database passwords are regenerated. Runtime
containers, volume contents, domain mappings, history, schedules and restore
lineage are not copied. Review environment-specific connection strings before
deploying. Each clone's storage volume is identified by its new service UUID.

`POST /api/v1/services/{id}/move` accepts `{ "environment_id": "UUID" }`.
`POST /api/v1/services/{id}/clone` accepts the same destination plus optional
`name`. Duplicate destination names return 409; a different project or unknown
environment is rejected. No operation automatically deploys the destination.
