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

### Static website preview

The Static Website page enables **Preview** after a build is published. It opens
the active release in a new tab without a domain or `FERRIX_WEBSITE_BIND`.
Preview requires a valid session and membership in the website's organization.
HTML, inline styles and relative image/CSS assets are supported. Scripts, forms
and embedded frames are disabled by a CSP sandbox to isolate uploads from the
dashboard. Root-relative asset URLs and interactive builds require the website's
dedicated hostname instead. Preview follows the active release after rollback.

## Pull request CI

The `CI` GitHub Actions workflow runs on every pull request, including updates,
on pushes to `main`, and can also be started manually. Its independent jobs check Rust formatting,
Clippy with warnings denied, all Rust tests (including ignored PostgreSQL
integration tests), and the frontend production build. Rust uses the stable
toolchain; the frontend uses Node.js 24 and npm 12.2.2 with the committed lockfile.

Each Rust job gets a temporary PostgreSQL 17 service and its own `ferrix_ci`
database. No developer `.env`, external database or repository secrets are needed.
The integration test applies migrations in an isolated schema. New commits cancel
an older CI run for the same pull request.

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

### Home summary

Authenticated `GET /api/v1/home-summary` returns counts for the session's active
organization, including ordinary members. Projects and services cover all rows,
independent of the Projects list's 100-row limit. Service totals include
applications, databases and Compose services across every environment.
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

Static Website and Docker Application pages show an **Automatic domain**.
Use **Open website** to run JavaScript, modules, forms and root-relative assets
on a separate origin. Local URLs use `site-<uuid>.localhost:<port>` and
`app-<uuid>.localhost:<port>`. Port 80 is omitted from nmfairus URLs, so they open as `http://site-<uuid>.localhost/`
or `http://app-<uuid>.localhost/`. Port 80 is reserved for nmfairus; mac2 and
mac3 retain their separate ports to avoid conflicts.
UUIDs remain stable across publishes and redeploys.
Publish a static build or deploy an Application first. Applications deployed
before automatic domains were configured need one redeploy to join ingress.
Database and Compose services are not automatically exposed.

The ingress network and its ownership label are scoped to the Compose project.
Never reuse another user's network. The authenticated dashboard preview keeps
its script sandbox; use the automatic URL for interactive testing.

`python3 scripts/tests/automatic-domains.py` exercises real Caddy, Docker and
Playwright with disposable resources in the current user's database. It also
checks the production HTTPS routes with a test certificate authority; public
DNS and ACME issuance require a real VPS.

### Local HTTPS

The nmfairus profile uses HTTPS on port 443 and redirects HTTP on port 80.
Automatic URLs become `https://app-<uuid>.localhost/` and
`https://site-<uuid>.localhost/`. Other users retain their isolated HTTP ports.
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
query. This keeps cookies and form origins consistent for login and student
registration. API requests are not redirected; strict backend Origin checks
remain in place. Other users' ports and production domains are not affected.
