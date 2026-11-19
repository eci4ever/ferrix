# Production installation

Ferrix runs as three Compose services: Ferrix (Rust API and compiled React frontend), PostgreSQL 17, and Caddy for HTTPS. Only ports 80/443 are published. PostgreSQL data and Caddy certificates use persistent volumes. The API runs as a non-root user with the host Docker socket's group ID.

## Requirements

- Linux amd64 (Intel/AMD 64-bit) with a public IPv4 address. A custom DNS domain is optional.
- Inbound TCP 80 and 443 available, including the host/provider firewall.
- Root access, curl, openssl, util-linux and zoneinfo data.
- Docker Engine with Compose v2. Automatic Docker installation is limited to Debian/Ubuntu, via Docker's installation script. Existing Docker installations are retained.

The Docker socket grants Ferrix control of the host Docker daemon. Use a dedicated VPS. No development user folders, ports or credentials are used.

## Publish before installing

The installer URL and image are available only after these files are merged and an image is published. Create a version tag such as `v0.1.0` to trigger `.github/workflows/release.yaml`. Tags publish `ghcr.io/eci4ever/ferrix:0.1.0`, `latest` for stable releases, and a full commit SHA tag, for amd64 only. The workflow builds only `linux/amd64` on a native amd64 runner. ARM64 release images are not published. Manual workflow runs require an existing `release_tag` and rebuild exactly that tag's commit without moving it, publishing the same version and SHA tags. Ensure the GHCR package visibility is public so VPS hosts can pull without authentication. A version tag is not a GitHub Release by itself.

## Install

For a released version, download installer and configuration from the same tag:

```sh
curl -fsSL https://raw.githubusercontent.com/eci4ever/ferrix/v0.1.0/scripts/install.sh -o install-ferrix.sh
sudo env FERRIX_DOMAIN=deploy.example.com FERRIX_REF=v0.1.0 FERRIX_VERSION=0.1.0 FERRIX_TIMEZONE=Asia/Kuala_Lumpur bash install-ferrix.sh
```

After a stable image is published, the convenient interactive form uses main's installer/configuration and the latest image:

```sh
curl -fsSL https://raw.githubusercontent.com/eci4ever/ferrix/main/scripts/install.sh | sudo bash
```

The domain prompt reads `/dev/tty`, so piped installation works. Set `FERRIX_DOMAIN` through `sudo env` for unattended use. The timezone defaults to UTC; set `FERRIX_TIMEZONE` explicitly when desired.

Installation creates `/opt/ferrix/{.env,compose.yaml,Caddyfile}` with root-only access, generates independent database and auth secrets, and waits for Compose healthchecks. HTTPS certificate issuance additionally depends on correct DNS and network access; container startup alone does not confirm public HTTPS. Register the first admin immediately: the first successful signup becomes platform admin. Restrict HTTPS access on TCP 443 to your IP until registration is complete; leave TCP 80 available for public certificate validation.

The installer refuses an existing installation. If startup fails after files are written, inspect logs and rerun `docker compose up` instead of deleting volumes or secrets.

## Operate and update

Use a root shell (`sudo -i`) for the following commands; `/opt/ferrix` is root-only.

```sh
cd /opt/ferrix
sudo docker compose ps
sudo docker compose logs --tail=100 ferrix proxy
```

Before updates, make a backup:

```sh
sudo bash -c 'umask 077; cd /opt/ferrix; docker compose exec -T postgres pg_dump -U ferrix -d ferrix -Fc > ferrix.dump; tar -czf ferrix-backup.tar.gz .env compose.yaml Caddyfile ferrix.dump; rm ferrix.dump'
```

Keep backups off-host. The `.env` auth key is required to decrypt existing MFA secrets and saved service configuration. Do not regenerate the PostgreSQL password against an existing volume. Compose `down` preserves named volumes; `down -v` destroys them.

To update, first inspect the target release's migration/compatibility notes. Edit only `FERRIX_IMAGE` in `/opt/ferrix/.env` to a published version (or immutable image digest), then:

```sh
cd /opt/ferrix
sudo docker compose pull
sudo docker compose up -d --wait --wait-timeout 180
```

Migrations run on API startup. Rolling back an image does not roll back schema changes: restore the matching database backup when a migration is incompatible. Preserve the auth key and credentials. Configuration changes in a new release must be applied deliberately; the installer never overwrites existing configuration.

For recovery on a fresh installation with the original `.env` and Compose configuration, start only PostgreSQL, restore the dump with `pg_restore -U ferrix -d ferrix --clean --if-exists` through `docker compose exec -T postgres`, then start Ferrix and Caddy. Keep the original image version until the restore has been verified.

## Local image verification

```sh
docker build -t ferrix:local .
```

To test the production stack, copy `compose.production.yaml` and `deploy/Caddyfile` into a dedicated directory, create its own `.env` matching the required variables, and use a separate Compose project and free proxy ports. Never use the development database or run the VPS installer on a shared development host. `FERRIX_IMAGE=ferrix:local` allows testing a locally built image.

The image contains no Node runtime or Rust toolchain. `FERRIX_STATIC_DIR` enables static frontend serving; local development continues using Vite when it is unset. `/health` is process liveness, not a full database/Docker readiness check. The protected status endpoint reports those dependencies separately.

### Repeatable smoke checks

```sh
# Isolated installer execution with mocked Docker/download commands:
docker run --rm --entrypoint /bin/bash -e FERRIX_INSTALL_TEST=1 -v "$PWD:/src:ro" -v /var/run/docker.sock:/var/run/docker.sock:ro debian:bookworm-slim /src/scripts/tests/install.sh
# Real production stack with private test volumes and loopback HTTPS port 18443:
python3 scripts/tests/production.py ferrix:local
```

The smoke test uses Caddy's internal CA only for localhost verification, creates a disposable database, and removes only its own Compose project and volumes when finished. It requires Compose 2.24.4+ for its test override. Public production HTTPS continues using Caddy's automatic public certificates.

The runtime image includes Docker CLI and Compose for managed Compose deployments. Hosted service databases and Compose named volumes require separate data backups; the Ferrix metadata dump does not contain those volumes.

## Host a static website

1. Open a project environment and choose **Create Service → Static Website**.
2. Use the generated automatic domain, or have a system administrator with
   organization manager access assign a custom hostname. For a custom hostname,
   point its A/AAAA records at the server. Ensure TCP 80 and 443 are reachable.
3. Select your build output folder and publish it. The root must contain
   `index.html`. Each build supports up to 512 files and 8 MiB total. Each
   organization can keep up to 10 websites.
4. Open the HTTPS link. Caddy obtains and renews certificates on demand.
   Ferrix authorizes issuance only for a registered domain with a published
   build. If issuance fails, check DNS, firewall rules and Caddy logs.
5. Choose **Roll back** on a retained release to activate it again. Ferrix
   retains the five most recent uploads per website.

Static releases are part of the PostgreSQL backup. The website listener runs
on internal container port 8081 and must not be published directly. The
dashboard stays on port 8080 behind its own Caddy host rule. Static hosting serves files and directory indexes. HTML navigation to a missing
extensionless path falls back to root `index.html` for SPA routing. Missing
assets such as `.js` files still return 404.

For an existing installation, back up the database, then update the installed
Compose and Caddy configuration from the same release as the image. Preserve
the existing `.env` and named volumes, including Caddy certificate storage.

## Automatic domains with Caddy

Set `FERRIX_AUTO_DOMAIN=sslip.io` and `FERRIX_PUBLIC_IP` to the VPS public IPv4
address in `/opt/ferrix/.env`. The installer detects the address through
`https://api.ipify.org` unless explicitly supplied. Leaving `FERRIX_DOMAIN`
blank generates `ferrix.<ip-with-dashes>.sslip.io` for the dashboard too.

Published Static Websites receive `https://site-<uuid>.<ip-with-dashes>.sslip.io/`;
deployed Docker Applications receive `https://app-<uuid>.<ip-with-dashes>.sslip.io/`.
No DNS account or custom domain is required. Open the link on the service page.
URLs remain stable across redeploys while the configured public IP is unchanged.
IPv6-only automatic domains are not supported in this version.

Caddy joins the project-owned ingress network and routes Application traffic
to the active deployment's container port. New deployments attach automatically;
redeploy older Applications once after enabling the feature. Database and
Compose services stay private. Only registered published sites or deployed
Applications can authorize automatic certificate issuance. Public certificate
issuance depends on reachable ports 80/443, public DNS and CA availability.
The website listener on 8081 and Caddy's administration interface are not exposed.

Existing VPS installations must update the image, Compose file and Caddyfile
from the same release, preserve secrets/volumes, add the above settings, then
run `docker compose up -d`. The ingress name defaults to the Compose project
name plus `_ingress`; Ferrix and Caddy must use the same network. If the VPS IP
changes, update `FERRIX_PUBLIC_IP` and restart Ferrix; generated URLs will change.

References: [Caddy forward authentication](https://caddyserver.com/docs/caddyfile/directives/forward_auth)
and [sslip.io DNS and TLS](https://sslip.io/).
