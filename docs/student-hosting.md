# Hosting

Hosting uses the existing Caddy proxy, **one shared PHP-FPM runtime** and one
shared MariaDB server. Adding students does not create additional containers.
Each approved student receives a persistent PHP directory, a dedicated Unix
user and FPM pool, a private MariaDB database/user, and an automatic domain.

## Administrator and student flow

1. Open **Hosting** and enable the shared runtime. Registration defaults to closed
   on new installations until the administrator enables it.
2. Share `/student-register`. Students register with their name, email and a
   password of at least 12 characters. They can sign in to see pending status.
3. Verify the student before choosing **Approve**. Email ownership is not verified
   automatically. Existing platform accounts are not converted by registration.
4. Approval creates a database/user and configures a PHP-FPM pool and directory.
   Students can upload their project folder and open the automatic HTTPS URL.
5. **Suspend** removes the pool, terminates its PHP processes, locks its database
   account and disables public routing. **Resume** restores the same user, files
   and database. **Reject** applies only to pending registrations. Closing
   registration leaves active hosting enabled.

Uploads allow 256 files and 8 MiB per request, using relative ASCII paths of up
 to 100 bytes. Hidden files, traversal and duplicate paths are rejected.
Matching files are replaced atomically; other files remain. There is no shell,
SFTP, Composer build, phpMyAdmin or automatic backup service in this version.

## File Manager and database transfer

Active students can use **File Manager** to browse folders, create files/folders,
edit UTF-8 text, upload/download files, rename and delete items. The displayed
`public_html` is their existing jail's `/public` directory; no additional
container is created. Only empty folders can be deleted. The text editor accepts
up to 1 MiB; file read/write and downloads accept up to 8 MiB. Directory listings
are bounded to 1,024 entries. Paths retain the upload rules above. Symlinks and
special files are never listed or followed. Every path component is opened
relative to an already-open directory with `O_NOFOLLOW`. Renames cannot overwrite
another item. Writes replace file contents atomically; keep a copy before editing.

**Export SQL** downloads tables and data from the student's own database, up to
16 MiB. InnoDB data is read in one repeatable-read transaction. Binary data and
generated columns are supported; views, routines and triggers are excluded.
**Import SQL** accepts UTF-8 SQL up to 8 MiB, with confirmation before running.
Export a backup first: DDL and earlier successful statements cannot be rolled
back if a later statement fails. The importer accepts SQL through PDO, not mysql
client commands such as `source`, `system` or `DELIMITER`.

Every file/database request checks the authenticated identity against the
hosting owner and requires active hosting. Platform administrators can manage
any student's hosting; students receive 404 for another student's ID, including
write/import requests. Client requests cannot choose a database, user, UID or
container. SQL operations drop to the student's Unix UID/GID, clear supplementary
groups and connect with only that student's MariaDB credentials. No root SQL
account is used for import/export; MariaDB denies cross-database SQL, global
grants and server file access, and PDO local infile is disabled. SQL, passwords
and file contents are not included in diagnostic logs. Database transfers are
rate-limited and time-bounded. Runtime upgrades must include both the API and
the updated PHP-FPM image containing `hosting-database.php`.

## PHP connection

Each pool receives only its own credentials. Student PHP can use:

```php
$pdo = new PDO(
    'mysql:host=' . getenv('DB_HOST') .
    ';dbname=' . getenv('DB_NAME') . ';charset=utf8mb4',
    getenv('DB_USER'), getenv('DB_PASSWORD'),
    [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]
);
```

`DB_HOST` is the private alias `mariadb`. No database port is published. Each user
can manage tables in its own generated database, with no global privileges or
access to another student's database. Passwords are encrypted in Ferrix metadata;
FPM configuration contains plaintext credentials in a separate root-only volume
that is never mounted into Caddy or exposed inside a student's jail.

## Shared runtime and file isolation

The `php-fpm` container is scoped to the Ferrix installation. Pools use stable,
distinct Unix IDs allocated by migration 0008. Each has a filesystem chroot with
its own `/public`, `/tmp` and minimal DNS configuration. Pool workers drop to
the student's uid/gid. Private root-owned directories, Unix permissions and
`open_basedir` prevent cross-student file access. The jail has no `/proc`, runtime
configuration or other students' directories. Shell execution, dynamic extension
loading, FFI, hard links and symbolic links are disabled in student PHP.

Caddy mounts two project-scoped volumes read-only:

- `hosting_files` at `/srv/hosting`: student jails and protected published views.
- `hosting_sockets` at `/run/hosting`: root-only FastCGI sockets for each pool.

A privileged control helper, never student PHP, publishes a **root-owned static
view** for each domain. It copies only regular files through `O_NOFOLLOW` file
operations and ignores symlinks. Students cannot write this view. PHP files are
represented by empty placeholders for Caddy's file matcher, so PHP source is
never copied into the static view. Caddy sends script requests to the student's
own Unix socket, with `/public` as the FastCGI document root inside the chroot.
Private suffixes and noncanonical PHP extension casing are blocked.

Uploaded assets are published immediately. Files generated later by PHP are
published by reconciliation, normally within a few seconds. Static publication
is bounded to 1,024 files and 32 MiB per student. Over-limit or unsafe source files
leave the last valid static view intact. Published directory switches are atomic.
These publishing limits are not hard persistent-disk quotas: PHP and SQL can
still create additional data. Monitor storage and back it up for classroom use.

The FPM master uses `pm=ondemand`: each pool has up to two children, with idle
processes removed after ten seconds. The shared runtime has a global process
limit of 16, 1 GiB memory and one CPU. PHP scripts have a 64 MiB memory limit and
30-second execution/termination limit; OPcache is disabled to avoid shared cache
ambiguity across chroots. The master has only the capabilities required to
create users, assign ownership, chroot, drop worker identities and terminate
pool processes. Student workers do not retain those capabilities. This is
shared process isolation, not a separate kernel/container boundary per student.

MariaDB retains its existing volume, one CPU, 1 GiB limit and ten connections per
student account. The displayed services are `mariadb` and `php-fpm` within the
installation group in OrbStack. Ferrix retains their Docker lifecycle ownership.
Automatic domains retain the same student UUID and existing Caddy HTTPS behavior.
The gateway verifies active status, account bans and shared-runtime ownership;
clients cannot supply a filesystem root or choose another student's socket.

## Local and production setup

For local development:

```sh
docker build -t ferrix-php-fpm:local deploy/student-php
```

Set `FERRIX_HOSTING_PHP_IMAGE=ferrix-php-fpm:local` in the private `.env`, recreate
only Caddy with the updated Compose mounts, then restart the API. The FPM runtime
is created on demand when Hosting is enabled. Its three persistent volumes are
`<project>_hosting_files`, `<project>_hosting_sockets` and
`<project>_hosting_runtime`. Keep the original `ferrix-students-db-<instance UUID>`
MariaDB volume. Do not remove any volumes during the update.

For production, release the matching image
`ghcr.io/eci4ever/ferrix-php-fpm:<version>`, make the package public, and update the
Ferrix image, Caddyfile and Compose configuration together. The new installer
sets `FERRIX_HOSTING_PHP_IMAGE`; the old Apache image setting
`FERRIX_STUDENT_PHP_IMAGE` is no longer used by shared FPM. Caddy must mount the
same files/sockets volumes as FPM. The API accesses them through its Docker
control plane; no student filesystem or socket is published on the host.

Existing per-student containers are never silently deleted or overwritten.
Legacy active websites retain their old HTTP routing until explicitly migrated.
For installations with real legacy data, export those files first and arrange a
controlled import into shared hosting; legacy upload/resume does not implicitly
convert or discard the old volume. The current nmfairus installation's test
students were explicitly removed before switching to shared FPM.

Back up PostgreSQL metadata, the Ferrix `.env` auth key, the MariaDB volume and
both Hosting files/runtime volumes. The metadata sequence and runtime user IDs
must remain consistent when restored. Socket files can be regenerated. There
are no automatic backups, disk quotas or hosting deletion UI yet.

## Verification

```sh
docker build -t ferrix-student-php:fpm-test deploy/student-php
python3 scripts/tests/student-hosting.py
```

This test uses a disposable PostgreSQL schema and owned Docker resources. It
checks that two approved students create only one shared PHP-FPM and MariaDB,
that Caddy serves PHP/assets, distinct Unix identities and chroot boundaries,
blocked cross-database access, hidden/source file protection, upload path checks,
suspend/resume, container recreation, trusted local HTTPS and browser flows.
All test students and test resources are removed afterwards.

References: [Caddy PHP FastCGI](https://caddyserver.com/docs/caddyfile/directives/php_fastcgi),
[Caddy file server security](https://caddyserver.com/docs/caddyfile/directives/file_server),
[PHP-FPM configuration](https://www.php.net/manual/en/install.fpm.configuration.php).
