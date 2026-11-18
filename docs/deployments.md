# Docker image deployments

The first deployment flow follows Dokploy's resource hierarchy: organization → project → environment → service. Each new project receives `production`; the new migration also backfills existing projects. Managers can add environments such as `staging`, create applications, edit configuration and deploy. Members can view applications/history/logs but cannot read saved environment values or mutate deployments. All resource access is scoped to the active organization.

## Use

1. Open **Projects**, then select a project.
2. Select `production` or create another environment.
3. Select **Create Service** → **Application**, **Database**, or **Compose**. Applications accept a public Docker image such as `nginx:alpine` and container port `80`. Database starts with PostgreSQL 17. Compose accepts image-based YAML.
4. Open the service, save its configuration and select **Deploy**.
5. Follow queued → running → succeeded/failed in deployment history. Open the runtime address when running.

Configuration changes are saved separately and take effect only on the next deployment. Deployments snapshot their configuration when queued, so editing settings cannot change an already queued deployment. Redeploy pulls the tag again and pins the new container to the resulting immutable image ID.

## Database and Compose

PostgreSQL services use a persistent named volume, generated credentials and a private environment network. No database port is published on the host. Applications deployed in the same environment can connect using the connection URL shown to managers. Credentials become locked once a deployment is queued, so changing a password in configuration cannot silently diverge from the initialized database. Database redeployment briefly stops the old container before starting its replacement; failed candidates are removed before restarting the previous container. Back up service database volumes separately from Ferrix's metadata database.

Compose creation, configuration and deployment require both platform admin and organization manager permissions. The first version supports up to 20 public image-based services, scoped bridge networks and named volumes. Builds, external resources, host mounts, privileged containers and host namespaces are rejected. Published ports bind to loopback with automatically allocated ports. Each service has a stable Compose project, ownership labels and resource limits. Compose runs with a private configuration directory and does not inherit Ferrix secrets. Saved YAML and environment variables are encrypted, including deployment snapshots.

Compose updates its existing project using `up --wait`. Named volumes survive redeployment. Updates are not atomic: a failed update may leave some containers updated; inspect runtime/logs, fix configuration and redeploy. Removed YAML services are not automatically deleted. There is no automatic Compose rollback.

## Runtime behavior

A Tokio worker in the API process polls the PostgreSQL queue. HTTP handlers only persist jobs and return 202; they do not perform long Docker operations. A partial unique index permits at most one queued/running job per application. A schema-specific PostgreSQL session advisory lock serializes workers across API processes. A worker recovers interrupted running jobs as failed, preserving the application's last activated container. Queued jobs survive restart. Each deployment has a five-minute deadline.

The worker rechecks the requesting user's membership before starting a queued deployment. It pulls the image, creates a candidate, starts it, and checks that it remains running on successive inspections. Images with a healthcheck must become healthy. A running state without a healthcheck is not an HTTP readiness guarantee. Activation and success are committed together. For stateless applications, on failure the previous activated container stays active; before the first success there is no active container. Database and Compose behavior is described above.

Candidate/replaced containers are cleaned only when their Ferrix application/deployment labels match terminal records in this database and they are not the current active container. Other users' containers and development Compose resources are preserved. Anonymous volumes are never deleted by the worker. Labels include application, deployment, environment, project and organization UUIDs; container names include application and deployment IDs to avoid collisions across users.

Each container has 512 MiB memory, one CPU and 256 PIDs, no host mounts or privileged-mode configuration, `no-new-privileges`, an unless-stopped restart policy, and rotating Docker logs. An automatic loopback port is allocated by Docker, preventing fixed-port collisions between users. Its address belongs to the Docker host. On a local installation open `http://127.0.0.1:<port>`; on a VPS use an SSH tunnel for now. Every redeploy allocates a new port: there is no stable public domain or zero-downtime routing yet.

The Application service supports **stateless public Docker images**, one container per application on the connected Docker server. It does not implement private registry credentials, Swarm services, replicas, Git builds, persistent-volume configuration, domain/HTTPS routing or rollback. Images declaring persistent volumes are rejected before container creation, preventing accidental state loss during replacement. Application containers use the Docker server's native image architecture; the Ferrix release image remains amd64 only.

Environment values are encrypted at rest using the existing auth key with an application-specific authenticated context, including immutable deployment snapshots. Keep the auth key in backups. Values are passed directly to Docker without shell interpolation. Saved values are returned only to organization managers. Container output is read-only, bounded to the latest 100 lines/64 KiB, and can contain anything the application prints; avoid printing secrets. Deployment progress uses bounded phase messages and does not store environment values. Read polling authenticates without extending inactivity expiry.

## API

All paths are under `/api/v1`; authenticated unsafe requests also require the configured exact Origin.

| Method | Path | Purpose |
| --- | --- | --- |
| GET / POST | `/projects/{id}/environments` | List/create project environments |
| GET / POST | `/environments/{id}/services` | List/create all service types |
| GET | `/services/{id}` | Read service metadata |
| GET / POST | `/services/{id}/configuration` | Manager-only configuration; Compose also requires platform admin |
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

It covers creation, configuration and real Docker deployment of Application, PostgreSQL and Compose, plus desktop/mobile layouts. The Docker integration test additionally verifies PostgreSQL data survives redeployment and Compose environment values remain literal.

## Delete a service

Use the trash button on a project service card, type the exact service name and confirm. Deletion removes its containers, service configuration and deployment history. Database/Compose volumes are retained by default for manual recovery; choose **Permanently delete data** to remove only that service’s labeled volumes. Retained volumes are not automatically reused by newly created services. Shared environment networks, images and other services remain intact. Organization managers can delete Application/Database; Compose also requires platform admin. Pending deployments block deletion. Docker cleanup failures retain the record for retry, although some containers may already have been removed.

The authenticated `DELETE /api/v1/services/{id}` accepts `{ "name": "exact service name", "remove_data": false }` and returns 204 on success.
