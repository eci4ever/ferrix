# Docker image deployments

The first deployment flow follows Dokploy's resource hierarchy: organization → project → environment → application. Each new project receives `production`; the new migration also backfills existing projects. Managers can add environments such as `staging`, create applications, edit configuration and deploy. Members can view applications/history/logs but cannot read saved environment values or mutate deployments. All resource access is scoped to the active organization.

## Use

1. Open **Projects**, then select a project.
2. Select `production` or create another environment.
3. Create an **Application**, starting with `nginx:alpine`, container port `80`, and optional literal `KEY=value` environment variables.
4. Open the application and select **Deploy**.
5. Follow queued → running → succeeded/failed in deployment history. Open the runtime address when running.

Configuration changes are saved separately and take effect only on the next deployment. Deployments snapshot their configuration when queued, so editing settings cannot change an already queued deployment. Redeploy pulls the tag again and pins the new container to the resulting immutable image ID.

## Runtime behavior

A Tokio worker in the API process polls the PostgreSQL queue. HTTP handlers only persist jobs and return 202; they do not perform long Docker operations. A partial unique index permits at most one queued/running job per application. A schema-specific PostgreSQL session advisory lock serializes workers across API processes. A worker recovers interrupted running jobs as failed, preserving the application's last activated container. Queued jobs survive restart. Each deployment has a five-minute deadline.

The worker rechecks the requesting user's membership before starting a queued deployment. It pulls the image, creates a candidate, starts it, and checks that it remains running on successive inspections. Images with a healthcheck must become healthy. A running state without a healthcheck is not an HTTP readiness guarantee. Activation and success are committed together. On failure, the previous activated container stays active; before the first success there is no active container.

Candidate/replaced containers are cleaned only when their Ferrix application/deployment labels match terminal records in this database and they are not the current active container. Other users' containers and development Compose resources are preserved. Anonymous volumes are never deleted by the worker. Labels include application, deployment, environment, project and organization UUIDs; container names include application and deployment IDs to avoid collisions across users.

Each container has 512 MiB memory, one CPU and 256 PIDs, no host mounts or privileged-mode configuration, `no-new-privileges`, an unless-stopped restart policy, and rotating Docker logs. An automatic loopback port is allocated by Docker, preventing fixed-port collisions between users. Its address belongs to the Docker host. On a local installation open `http://127.0.0.1:<port>`; on a VPS use an SSH tunnel for now. Every redeploy allocates a new port: there is no stable public domain or zero-downtime routing yet.

This MVP supports **stateless public Docker images**, one container per application on the connected Docker server. It does not implement private registry credentials, Swarm services, replicas, Git builds, persistent-volume configuration, domain/HTTPS routing or rollback. Images declaring persistent volumes are rejected before container creation, preventing accidental state loss during replacement. Application containers use the Docker server's native image architecture; the Ferrix release image remains amd64 only.

Environment values are encrypted at rest using the existing auth key with an application-specific authenticated context, including immutable deployment snapshots. Keep the auth key in backups. Values are passed directly to Docker without shell interpolation. Saved values are returned only to organization managers. Container output is read-only, bounded to the latest 100 lines/64 KiB, and can contain anything the application prints; avoid printing secrets. Deployment progress uses bounded phase messages and does not store environment values. Read polling authenticates without extending inactivity expiry.

## API

All paths are under `/api/v1`; authenticated unsafe requests also require the configured exact Origin.

| Method | Path | Purpose |
| --- | --- | --- |
| GET / POST | `/projects/{id}/environments` | List/create project environments |
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
