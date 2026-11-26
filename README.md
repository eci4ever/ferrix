# Ferrix

## About

Ferrix is a self-hosted deployment platform for Docker applications and databases. Manage deployments, domains, backups and team access from one dashboard, built with Rust, React and Caddy.

Website: [ferrix.nimfi.dev](https://ferrix.nimfi.dev) · License: [AGPL-3.0-only](LICENSE)

## Features

- Organize applications and databases into projects and environments.
- Deploy Docker images, GitHub repositories or uploaded source archives, with deployment history and rollback.
- Run PostgreSQL, MariaDB, MongoDB and Redis, with persistent application storage.
- Configure automatic or custom domains with HTTPS and private LAN access.
- Monitor resources, view logs and open service terminals.
- Back up and restore supported services, with schedules and S3-compatible storage.
- Manage organizations, resource assignments, API keys and two-factor authentication.
- Provision student PHP applications with isolated databases on shared MariaDB.

## Getting Started

Use an Ubuntu/Debian **amd64** VPS with a public IPv4 address, sudo access and ports **80/443** available. Install `curl`, `openssl`, `util-linux` and `tzdata`, then run:

```sh
curl -fsSL https://raw.githubusercontent.com/eci4ever/ferrix/main/scripts/install.sh | sudo bash
```

Follow the installer prompts, open the displayed URL and create the first administrator account. Press Enter at the domain prompt to use an automatic domain, or provide your own domain pointing to the server.

See the [production guide](docs/production.md) for installation and updates, or the [development guide](docs/development.md) to run Ferrix locally.
