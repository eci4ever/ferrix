# Ferrix

A self-hosted deployment platform for managing Docker applications and databases on a single server. Built with Rust, a React dashboard and Caddy for HTTPS.

## Features

- Organize Application and Database services into projects and environments.
- Deploy Docker images or public/private GitHub repositories using Dockerfiles, with push-triggered deployments through webhooks.
- Track deployment progress, commits and initiators; roll back to a successful version.
- Attach persistent Application storage, override command arguments and run background workers with single-writer replacement.
- Use automatic domains or custom domains with Caddy HTTPS.
- Run PostgreSQL, MariaDB, MongoDB and Redis. Back up and restore PostgreSQL and MariaDB, with schedules and S3-compatible storage.
- Search live logs, monitor CPU/RAM, open service terminals and receive notifications.
- Manage users and organizations, enable TOTP 2FA and create organization/project-scoped API keys for deployment agents.

Deploy static websites as Applications. Shared hosting, Compose services and a separate Static Website service are not supported.

## Install on a VPS

Requires an Ubuntu/Debian **amd64** VPS (Intel/AMD 64-bit), a public IPv4 address, sudo access and available TCP ports **80/443**, open in your firewall. Install `curl`, `openssl`, `util-linux` and `tzdata` first. The installer sets up Docker if needed.

```sh
curl -fsSL https://raw.githubusercontent.com/eci4ever/ferrix/main/scripts/install.sh | sudo bash
```

This installs the **latest stable image** (`ghcr.io/eci4ever/ferrix:latest`) using the installer and configuration from `main`. ARM64 images are not published.

**Press Enter at the domain prompt** to use an automatic `sslip.io` domain based on your public IP. Alternatively, enter a custom domain with an A record pointing to your VPS. HTTPS requires working DNS and public access to ports 80/443.

Open the displayed URL and register the first account immediately; it becomes the platform administrator. Restrict port 443 to your IP until registration is complete, while leaving port 80 available for certificate validation.

The installer creates Ferrix, PostgreSQL and Caddy under `/opt/ferrix` and refuses to overwrite an existing installation. The timezone defaults to UTC; see the [production guide](docs/production.md) for configuration and installation of a specific version.

To check status and logs:

```sh
sudo -i
cd /opt/ferrix
docker compose ps
docker compose logs --tail=100 ferrix proxy
```

Before upgrading, back up the database and `.env`, review the [release notes](https://github.com/eci4ever/ferrix/releases/latest), then follow the [upgrade guide](docs/production.md#operate-and-update). Preserve `FERRIX_AUTH_KEY` to decrypt saved configuration. Database migrations run on startup; rolling back an image does not reverse schema changes. Existing installations are not updated automatically.

## Stack

| Component | Technology |
| --- | --- |
| Backend | Rust, Axum, Tokio, SQLx, PostgreSQL |
| Docker | Bollard, BuildKit |
| Frontend | React, TypeScript, Vite |
| UI | shadcn/ui, Tailwind CSS, TanStack Router and Table |
| Reverse proxy | Caddy |

## Documentation

- [VPS installation, operation and upgrades](docs/production.md)
- [Development setup and per-user ports](docs/development.md)
- [Services and deployments](docs/deployments.md)
- [Application storage and background workers](docs/application-storage.md)
- [GitHub, Dockerfiles and webhooks](docs/github-deployments.md)
- [Authentication and security](docs/auth.md)
- [API keys for agents](docs/api-keys.md)

Ferrix controls the host Docker daemon through its socket. Use a dedicated VPS and grant administrative access only to trusted users.
