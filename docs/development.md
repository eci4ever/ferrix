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

The `CI` GitHub Actions workflow runs on every pull request, including updates,
and can also be started manually. Its independent jobs check Rust formatting,
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
