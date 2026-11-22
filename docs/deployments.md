# Docker image deployments

The first deployment flow follows Dokploy's resource hierarchy: organization → project → environment → service. Each new project receives `production`; the new migration also backfills existing projects. Managers can add environments such as `staging`, create applications, edit configuration and deploy. Members can view applications/history/logs but cannot read saved environment values or mutate deployments. All resource access is scoped to the active organization.

## Use

1. Open **Projects**, then select a project.
2. Select `production` or create another environment.
3. Select **Create Service** → **Application** or **Database**. Applications accept a public Docker image such as `nginx:alpine` and container port `80`. Database opens a selector for PostgreSQL 17, MongoDB 8, MariaDB 11.4 or Redis 8.
4. Open the service, save its configuration and select **Deploy**.
5. Follow queued → running → succeeded/failed in deployment history. Open the runtime address when running.

Configuration changes are saved separately and take effect only on the next deployment. Deployments snapshot their configuration when queued, so editing settings cannot change an already queued deployment. Redeploy pulls the tag again and pins the new container to the resulting immutable image ID.

## Database

Database services use persistent named volumes, generated credentials and a private environment network. No database port is published on the host. Applications deployed in the same environment can connect using the connection URL shown to managers. Credentials become locked once a deployment is queued, so changing a password in configuration cannot silently diverge from the initialized database. Database redeployment briefly stops the old container before starting its replacement; failed candidates are removed before restarting the previous container. Back up service database volumes separately from Ferrix's metadata database.



## Runtime behavior

A Tokio worker in the API process polls the PostgreSQL queue. HTTP handlers only persist jobs and return 202; they do not perform long Docker operations. A partial unique index permits at most one queued/running job per application. A schema-specific PostgreSQL session advisory lock serializes workers across API processes. A worker recovers interrupted running jobs as failed, preserving the application's last activated container. Queued jobs survive restart. Each deployment has a five-minute deadline.

The worker rechecks the requesting user's membership before starting a queued deployment. It pulls the image, creates a candidate, starts it, and checks that it remains running on successive inspections. Images with a healthcheck must become healthy. A running state without a healthcheck is not an HTTP readiness guarantee. Activation and success are committed together. For stateless applications, on failure the previous activated container stays active; before the first success there is no active container. Database behavior is described above.

Candidate/replaced containers are cleaned only when their Ferrix application/deployment labels match terminal records in this database and they are not the current active container. Other users' containers and development Compose resources are preserved. Anonymous volumes are never deleted by the worker. Labels include application, deployment, environment, project and organization UUIDs; container names include application and deployment IDs to avoid collisions across users.

Each application container has 512 MiB memory, one CPU and 256 PIDs, no host mounts or privileged-mode configuration, `no-new-privileges`, an unless-stopped restart policy, and rotating Docker logs. An automatic loopback port is allocated by Docker, preventing fixed-port collisions between users. Its address belongs to the Docker host. On a local installation open `http://127.0.0.1:<port>`; use the automatic domain shown on the Application page for a stable URL through Caddy, locally or on a VPS (see production.md). Existing Applications need one redeploy after enabling automatic domains. The loopback diagnostic port changes on redeploy. Deployments still have a brief handover and do not guarantee zero downtime.

The Application service supports **stateless public Docker images**, one container per application on the connected Docker server. It does not implement private registry credentials, Swarm services, replicas, Git builds, persistent-volume configuration. Images declaring persistent volumes are rejected before container creation, preventing accidental state loss during replacement. Application containers use the Docker server's native image architecture; the Ferrix release image remains amd64 only.

Environment values are encrypted at rest using the existing auth key with an application-specific authenticated context, including immutable deployment snapshots. Keep the auth key in backups. Values are passed directly to Docker without shell interpolation. Saved values are returned only to organization managers. Container output is read-only, bounded to the latest 100 lines/64 KiB, and can contain anything the application prints; avoid printing secrets. Deployment progress uses bounded phase messages and does not store environment values. Read polling authenticates without extending inactivity expiry.

## API

All paths are under `/api/v1`; authenticated unsafe requests also require the configured exact Origin.

| Method | Path | Purpose |
| --- | --- | --- |
| GET / POST | `/projects/{id}/environments` | List/create project environments |
| GET / POST | `/environments/{id}/services` | List/create all service types |
| GET | `/services/{id}` | Read service metadata |
| GET / POST | `/services/{id}/configuration` | Manager-only configuration |
| POST | `/services/{id}/deploy` | Queue immutable deployment snapshot |
| GET | `/services/{id}/runtime`, `/deployments`, `/logs` | Service runtime, history and output |
| GET / POST | `/environments/{id}/applications` | List/create applications |
| GET | `/applications/{id}` | Read application metadata |
| GET / POST | `/applications/{id}/configuration` | Manager-only configuration, including environment values |
| POST | `/applications/{id}/deploy` | Snapshot configuration and queue deployment; 409 if already pending |
| GET | `/applications/{id}/deployments` | Latest 30 deployment records/progress |
| GET | `/applications/{id}/runtime` | Live Docker state and loopback port |
| GET | `/applications/{id}/logs` | Bounded current-container output |

## Tests

Run Rust formatting, Clippy and all PostgreSQL/Docker integration tests against your own development database. `deployment.rs` creates a unique schema and labeled disposable containers, covers tenant isolation and permissions, encryption, immutable queued configuration, worker concurrency, successful Nginx HTTP responses, failure preservation, interrupted-job recovery and redeployment cleanup, and cleans its own resources. It needs Docker and curl.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
FERRIX_TEST_DATABASE_URL=<your-development-database-url> cargo test --workspace -- --include-ignored
cd apps/web
npm run build
```

For browser checks, start the API on port 8088 using a fresh temporary PostgreSQL schema and `FERRIX_AUTH_ORIGIN=http://127.0.0.1:5180`; start Vite with `FERRIX_BIND=127.0.0.1:8088 FERRIX_WEB_PORT=5180`. Install Chromium with `npx playwright install chromium`, then:

```sh
FERRIX_E2E_URL=http://127.0.0.1:5180 FERRIX_E2E_DISPOSABLE=1 npm run test:e2e
```

Never point browser registration tests at a user database. The test verifies fresh setup before creating a disposable account/project, tests environment selection, configuration, deploy, HTTP access and mobile overflow, and writes desktop/mobile screenshots under `/private/tmp`. Stop the preview API before removing its labeled test containers and dropping only its temporary schema. Preserve the main development services and data.

The repeatable browser runner allocates preview ports and a disposable schema in the current user's database, then removes only its own test resources:

```sh
python3 scripts/tests/services-ui.py
```

It covers creation, configuration and real Docker deployment of Application and PostgreSQL, plus desktop/mobile layouts. The Docker integration test additionally verifies PostgreSQL data survives redeployment.

## Delete a service

Use the trash button on a project service card, type the exact service name and confirm. Deletion removes its containers, service configuration and deployment history. Database volumes are retained by default for manual recovery; choose **Permanently delete data** to remove only that service’s labeled volumes. Retained volumes are not automatically reused by newly created services. Shared environment networks, images and other services remain intact. Organization managers can delete Application/Database. Pending deployments block deletion. Docker cleanup failures retain the record for retry, although some containers may already have been removed.

The authenticated `DELETE /api/v1/services/{id}` accepts `{ "name": "exact service name", "remove_data": false }` and returns 204 on success.

## Service controls and live status

Project cards poll one environment-scoped Docker inventory every five seconds. A green LED means all tracked containers are running, amber means starting/restarting/partially running, red means unhealthy or missing, and gray means stopped, undeployed or unavailable. The text label also communicates status. These are container/healthcheck states, not HTTP availability checks. Failed polling clears the previous green indication; opening a detail page polls runtime every three seconds without renewing session inactivity expiry.

All service types share **General**, **Environment** and **Logs** tabs. **Deploy** applies saved configuration and pulls the image. **Reload** restarts the existing deployed containers with no image pull or configuration changes. **Stop** requires confirmation and retains data; **Start** resumes those containers. Deploy is required to recreate containers.

Runtime operations require organization manager access. The service row lock serializes operations with deploy and deletion, and queued/running deployments block lifecycle controls. Only verified owned containers are affected.

The Environment tab loads manager-only encrypted values on demand. Save changes and then Deploy to apply them; Reload keeps deployed values. PostgreSQL reserves `POSTGRES_*` and `PG*`; MongoDB reserves `MONGO*`; MariaDB reserves `MARIADB_*`/`MYSQL_*`; Redis reserves `REDIS*` for managed credentials and initialization behavior. Other variables such as `TZ` are supported. Database credentials remain locked after the first deployment. Internal credentials have masked passwords, show/hide and copy controls; Only MariaDB has a separate root-password field.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/environments/{id}/service-status` | Scoped batch runtime states; all organization members |
| POST | `/api/v1/services/{id}/control` | `{ "operation": "start" }` (also `stop` or `reload`); manager only |
| GET / POST | `/api/v1/services/{id}/environment` | Read/save `{ "environment": "KEY=value" }`; manager only |

Terminal, per-service monitoring and database backup/restore remain outside this step.

## Database engines

The Database dialog offers exactly PostgreSQL, MongoDB, MariaDB and Redis. MySQL and libSQL are not available. `POST /environments/{id}/services` accepts `kind: "database"` and `engine: "postgres" | "mongodb" | "mariadb" | "redis"`. Omitted engine defaults to PostgreSQL for compatibility. Engine is immutable, including before the first deploy; create a separate service to change engines. Existing encrypted PostgreSQL configurations deserialize as PostgreSQL without a migration. Service metadata exposes the engine without credentials.

| Engine | Image | Internal port | Persistent storage |
| --- | --- | --- | --- |
| PostgreSQL | `postgres:17-alpine` | 5432 | `/var/lib/postgresql/data` |
| MongoDB | `mongo:8` | 27017 | `/data/db` and `/data/configdb` |
| MariaDB | `mariadb:11.4` | 3306 | `/var/lib/mysql` |
| Redis | `redis:8-alpine` | 6379 | `/data`, AOF enabled |

All engines require authentication and remain private to the environment network. MongoDB initializes an administrative user in `admin`; its connection URL includes `authSource=admin` and the selected database is created on first write. MariaDB generates independent application and root passwords; its connection URL uses the compatible `mysql://` protocol. Redis uses the default authenticated user and database 0, 384 MiB maximum dataset memory with no eviction, and AOF persistence (not a backup or a guarantee against all crash data loss).

MongoDB uses a 1 GiB container limit with a 256 MiB WiredTiger cache. The other database containers retain the 512 MiB limit. Healthchecks use each engine's native authenticated probe, except PostgreSQL's readiness probe. Database replacement stops the previous writer before starting its successor; persistent volumes survive Stop, Reload and redeploy. Credential/root-password changes lock after the first deployment is queued.

Integration tests authenticate, write data, reject anonymous reads, replace containers, read the original data and delete only owned volumes for all four engines.

Earlier Ferrix binaries only understand PostgreSQL service configuration; do not use an older worker to deploy newly created non-PostgreSQL database services.

## Static websites through Application

Bundle website files into an Nginx or Caddy Docker image, publish the image to a
public registry, then create an **Application** using that image and container
port 80. Its automatic domain supports JavaScript, assets and HTTPS just like
any other Application. Configure SPA fallback in the image's web server when
client-side routes need it. There is no separate Static Website upload service.

```dockerfile
FROM nginx:alpine
COPY dist/ /usr/share/nginx/html/
```

Application and Database are the only accepted service types. Historical
migrations remain unchanged; migration 0010 removes retired Compose service
records, Static Website releases/files and old Hosting accounts/metadata.

## Application rollback

Organization managers can select Rollback on a successful Application deployment in Deployment History. Confirmation queues a new deployment with the source snapshot’s port and encrypted environment, and its immutable Docker image ID. Saved service settings remain available for a subsequent regular deploy. Pending deployments block another deployment or rollback.

`POST /api/v1/services/{id}/rollback` accepts `{ "deployment_id": "uuid" }` and returns HTTP 202 with the queued deployment ID. API keys require the deploy scope. Database services and unsuccessful or foreign source deployments are rejected. History exposes `rollback_of` for rollback jobs.

The original image must remain on the connected Docker server. If it has been removed, rollback fails without pulling a mutable tag. The current Application stays active on failure. Rollback does not restore database or external data, and old environment secrets may no longer be valid.

## Custom domains and HTTPS

Application services have a Domains tab for up to ten custom hostnames. Organization managers can add, check and remove domains. Members can view their organization's domain records. Removing a service cascades its domain records, and removing a domain stops routing even while Caddy retains its cached certificate.

On a VPS, point the hostname's A record to `FERRIX_PUBLIC_IP` and add the displayed `_ferrix-challenge.<hostname>` TXT value to prove ownership. Use DNS-only mode during verification and remove conflicting AAAA records. The check requires all resolved addresses to match the configured server. Hostnames become routable after successful DNS proof. A verified hostname can belong to only one Application. Dashboard and generated automatic hostnames are reserved.

Locally, use a name such as `myapp.localhost`. Local domain checks activate it without public DNS records and use the existing local Caddy CA. Trust that root in the operating system to verify HTTPS. Local profiles with HTTPS disabled continue using HTTP.

Caddy's existing on-demand TLS authorization checks verified domains in PostgreSQL without DNS or outbound calls. Public HTTPS issuance requires reachable ports 80/443 and the hostname pointing directly to the VPS. The Domains check establishes a trusted HTTPS connection to the configured ingress IP, using the hostname for SNI, and reports certificate expiry. It never follows redirects or probes DNS-supplied addresses. Checks are explicit, limited to once per minute per domain, with a last-checked timestamp. Caddy handles renewals independently. An unavailable check can reflect DNS propagation, firewall, CA issuance restrictions, untrusted local roots or a server unable to reach its own public address. A successful certificate check does not imply application health.

API routes are `GET/POST /api/v1/services/{id}/domains`, `POST /api/v1/services/{id}/domains/{domain_id}/check` and `DELETE /api/v1/services/{id}/domains/{domain_id}`. API keys require read, manage and delete scopes respectively. Database services reject domain management. Public CA issuance requires a real controlled domain/VPS; the automated Caddy test uses an isolated local CA.

## PostgreSQL and MariaDB backups

The Backups tab queues manual backups and restores into a **new Database service in the same environment**. Enter a unique service name in the restore dialog. Follow queued/running/succeeded/failed status before connecting an application. Failed restores can leave an empty or partially restored new service for inspection or deletion. The source database remains available. New service credentials are generated afresh; its database name and username match the snapshot.

Backups cover the configured database, including tables and schema. PostgreSQL uses a custom-format dump and a single-transaction restore without original owners/ACLs. MariaDB includes routines, events and triggers with a single-transaction dump; avoid concurrent DDL and use transactional tables for consistency. Server-wide users/roles, other databases and external files are outside these backups.

Dumps are streamed directly by a short-lived, resource-limited helper using the deployed database's immutable image ID and environment network. Dumps have restrictive file permissions in a dedicated named volume `ferrix-backups-<service UUID>`. They remain on the Docker host and are not encrypted by Ferrix. Keep Docker storage protected, preserve Ferrix metadata/auth key, monitor disk space and maintain an independent off-server backup for host/disk failure. No retention policy automatically removes these backups. Service deletion with permanent data removal also removes its backup volume; keeping data retains the volume but removes service/history records.

Backup files are atomically finalized and checksummed. The next backup clears stale partial dump files left by interrupted work. Restore verifies SHA-256 before importing and uses the same database version. The original image and backup volume must remain available on the connected Docker server. Imported routines/functions can execute database code and should come from trusted sources. Backup contents and credentials are excluded from job messages and API responses.

Jobs persist in PostgreSQL and share the existing serialized deployment worker. The API returns HTTP 202 immediately. Runtime controls, deployment and deletion of affected services are blocked while operations are queued/running. The worker rechecks membership and ban status, verifies Docker ownership, enforces a twenty-minute operation deadline and stops helper containers before releasing the service. Interrupted work is marked failed after helper cleanup; an interrupted restore requires reviewing the new service, as completion may be unconfirmed.

Endpoints are `GET/POST /api/v1/services/{id}/backups` and `POST /api/v1/services/{id}/backups/{backup_id}/restore` with `{ "name": "Restored database" }`. Restore returns the operation and new Application IDs. API keys need read for history, deploy for backup, and both manage/deploy for restore. Organization members can view status; owners/admins can queue work. History includes the latest 100 source/target operations. MongoDB and Redis backups are outside this feature.

## Service logs and resource monitoring

Every Application and Database service has Logs and Monitoring tabs available to organization members. Log search filters the currently displayed buffer without sending the query to the server. Choose 100/500/1000 lines, refresh manually or enable Live tail for two-second snapshots. Pause stops live reads; auto-scroll is optional and search keeps the viewport stable. The viewer replaces its buffer when the active container changes and renders output as plain text. Hidden pages stop polling; switching tabs closes the active viewer. Lost access clears displayed telemetry.

`GET /api/v1/services/{id}/logs` and the legacy Application alias accept `tail` from 1–1000 and `timestamps=true/false`. Defaults remain 100 lines without timestamps. The response retains `logs` and adds container ID, checked timestamp and truncation metadata. Only the newest 64 KiB of text is retained. High-volume logs can exceed the visible window between snapshots. Older rotated logs and historical full-text search are outside this buffer.

`GET /api/v1/services/{id}/metrics` reads only the verified active container, with CPU usage in vCPU, configured CPU limit, RAM usage/limit and percentages of those limits. CPU uses Docker counter deltas; one vCPU is one CPU core. CPU quota and cpuset restrictions determine the configured limit. RAM follows Docker CLI working-set accounting by excluding reclaimable inactive file cache; raw usage is also returned. Missing counters, stopped containers and absent limits produce unavailable values rather than fabricated zero usage. The runtime diagnostic port is read from current Docker port bindings, including after stop/start changes an automatically allocated port.

Monitoring refreshes every five seconds while visible. Docker samples are bounded by a seven-second request deadline and coalesced in a bounded per-service cache for three seconds. Each request checks organization access and container ownership before cache use. Container replacement/restart invalidates the cached generation. No background collector samples inactive services, and no database history is written. API keys require read scope for logs and metrics. Read polling does not renew browser session inactivity expiry. Container logs may contain application secrets.
