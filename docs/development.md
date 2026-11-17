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

The dashboard reads authenticated `GET /api/v1/server-time` for the server timestamp, IANA timezone and current UTC offset. It ticks locally between synchronizations every 60 seconds and synchronizes again when the tab becomes visible. Clock requests validate the session without extending its inactivity expiry. The timezone follows the server OS, falling back to UTC if unavailable.
