# PHP classroom template (php-v1)

## Lecturer guide

1. Use an organization owner/admin account. Platform administrator access is not
   required. Add each student as an ordinary organization member.
2. In Team, enable assignment-only resource access. Remove broad student project
   assignments from the classroom project: those grants deliberately cover every
   application in that project and defeat individual classroom isolation. The
   template refuses creation until these prerequisites are satisfied.
3. Deploy a shared MariaDB service in the class environment. Keep its root and
   original configured-database credentials private. Its port remains unpublished.
4. Open the project/environment and choose **Add student PHP application**.
   Select the student and existing MariaDB service, and review CPU/RAM limits.
   The template creates an individual developer service assignment and uses the
   existing organization quota checks. Persistent storage is mounted at
   `/opt/data`; Docker local-volume byte quotas remain advisory.
5. Expand the generated-file preview or download the starter before creation.
   After creation, keep the starter or use the existing Source controls for
   GitHub or ZIP/tar upload. Choose **Use PHP v1 template (public/)** as the build
   method, or explicitly use your own Dockerfile. Upload preview lists generated,
   included and excluded paths before selecting a revision.
6. Choose **Provision database and connection settings**. This calls the #128
   workflow: one logical database/account per application, encrypted storage and
   five server-side `DB_*` values. No root password is copied. Unrelated
   environment values are preserved. Then choose **Deploy PHP application** and
   open the service's deployment progress.
7. If a step fails, retry the same step. Creation uses a request UUID and compares
   the original parameters; retries cannot create another application or change
   resource limits. Database provisioning reuses its mapping/account. A failed
   build retains database rows and the existing application volume. After closing
   the wizard, open the application's Database tab to resume provisioning.

The built-in runtime is the official `php:8.5-apache-bookworm` image, with
`mysqli` and `pdo_mysql`. PHP 8.5 is explicitly selected rather than `latest`;
its tag follows patch updates. Deployments record the resulting immutable image
and source revision. See the [PHP support policy](https://www.php.net/supported-versions.php)
and [official image documentation](https://github.com/docker-library/docs/blob/master/php/README.md).

The document root is **`public/` only** (`/var/www/app/public`). Project-root
document roots, framework installers and automatic legacy-code rewrites are not
included. Move public entry points/assets into `public/`, retaining private PHP
code, configuration and dependencies outside it. Dotfiles, Git metadata and
directory listings are not served. For the template build, `.env`, `.env.*`,
`.git`, `.svn` and `.hg` are excluded even if ignore rules try to re-include them.
Do not upload other secret files, SQL dumps or persistent data in source archives.

Generated files live under `.ferrix/php-v1/`. The existing Dockerfile and other
user files are never replaced. Conflicting files at those reserved paths cause
a preview/build failure; remove them before choosing the template, or select
your own Dockerfile explicitly. Choosing your own Dockerfile also makes its
document-root, extension and secret-exclusion behavior your responsibility.

Composer 2 is available at build time. If `composer.json` exists, commit
`composer.lock`; the build runs `composer install --no-dev --no-interaction
--prefer-dist --no-progress --no-scripts`. Dependencies are not downloaded during
web requests. Review Composer plugins and `allow-plugins` settings before building.
Additional extensions or Composer scripts needed by a framework require a reviewed
custom Dockerfile. No database credentials are build arguments.

### URLs and offline labs

The template reuses the application's existing Overview and Domains controls;
it does not create a second ingress service. Public direct/tunnel mappings and
LAN mappings retain their existing route-kind labels. A missing mapping is not
an available URL. The host-port address is a server-local diagnostic address,
not an automatically reachable school LAN hostname.

For LAN HTTPS, configure Ferrix private ingress, map each hostname in the
school's DNS (or client hosts file) to the server's reachable LAN address, allow
the configured ingress ports through the firewall, and install/trust the internal
CA on each client. Follow the existing Domains/private-ingress export instructions.
A generated hostname does not configure a router, DNS, firewall or trust store.
Use public HTTPS/domain verification for Internet access. Students should use the
configured hostname rather than their own computer's `localhost`.

The starter bundles its stylesheet and uses system fonts. After images and code
are built, rendering and the MariaDB connection need no external CDN or Internet.
Initial image pulls, Git downloads and Composer dependencies may require Internet
during build. School clients still need a path to the Ferrix server and DNS/CA
prerequisites; an offline page cannot fix network configuration.

### Data and access

Students receive developer access to their own application: source updates,
deployments, logs, limited connection settings and its own container terminal.
Changing CPU, RAM, storage or runtime commands remains manager-only on every
configuration route. Shared CPU/I/O contention in MariaDB is not a hard per-schema
resource guarantee. Application websites may be public; service/API permissions
and SQL database privileges are separate boundaries.

Removing a Ferrix assignment/member does not revoke a disclosed SQL password.
Revoke it explicitly through the MariaDB Database tab. Removing an Application
retains its logical database and, unless explicitly selected otherwise, its owned
volume. Exact-name logical database deletion is a separate manager operation.
Managed database/credential backup recovery follows
[the shared MariaDB guide](shared-mariadb.md): logical backups are manual,
restore is manager-only and affects the chosen database in place. Application
volume backups protect `/opt/data` separately. Project bundles remain blocked
until managed database mapping recovery exists. Do not describe configured-
database backups as backups of the entire class.

## Student guide

- Open your assigned application. To update code, use Source → GitHub repository
  or Source archive and choose PHP v1 build. Review the upload preview, select the
  revision, then Deploy. Keep `public/index.php` in the selected build context.
- Read injected values with `getenv('DB_HOST')`, `getenv('DB_PORT')`,
  `getenv('DB_DATABASE')`, `getenv('DB_USERNAME')` and `getenv('DB_PASSWORD')`.
  Use PDO prepared statements or mysqli parameter binding. The starter's
  `public/index.php` demonstrates PDO setup without embedding secrets.
- Existing applications that read another configuration format must be adapted;
  Ferrix does not rewrite database credentials into PHP files or `.env` archives.
  Never print credentials, commit them, place them in JavaScript, use `phpinfo()`
  on a public page or return raw database exceptions to visitors.
- Store persistent files in `/opt/data/uploads`; PHP session files use
  `/opt/data/sessions`. Other files in the image/container are not durable.
  These directories are outside the public document root. Write application code
  to authorize and serve downloads rather than exposing the directory directly.
- Use Deployment progress for build failures, Logs for runtime messages and
  Terminal for commands inside your own container. You cannot manage the shared
  MariaDB container or another student's application. Database access uses only
  your own restricted account; external MariaDB ports are not opened.
- A missing `public/index.php`, a conflicting reserved template file or a missing
  Composer lockfile is a source/build problem. A generic “Database unavailable”
  means the lecturer should check the shared service, provisioning and current
  connection settings. After password rotation, apply the new settings and
  redeploy. Ask the lecturer to perform backups/restores or change resource limits.

## Local verification

Build `apps/web`, then run `python3 scripts/tests/php-template-live.py` from the
current user's clone. The runner uses an isolated PostgreSQL schema/API port
and only its own labeled Docker resources. It deploys three actual PHP 8.5
applications against one MariaDB service, exercises SQL from inside PHP A's
container, verifies assignments/terminal/resource ceilings, source preview,
student ZIP updates, data persistence and failure recovery. A pinned, reviewed
GitHub PHP fixture exercises the real Git download/build/deployment path. The
browser test uses the actual wizard and blocks all external browser requests
while checking the starter and its stylesheet. The flow then provisions 30
assigned application drafts/logical databases on one shared service; it does not
claim 30 running PHP containers. Cleanup checks existing projects and running
containers were preserved. Actual school LAN DNS/internal-CA connectivity must
be verified on the target network separately.
