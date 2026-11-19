# Hosting

Hosting runs one shared MariaDB 11.4 server per Ferrix installation and
one PHP 8.4/Apache container per approved student. Each student receives exactly
one generated MariaDB database and non-root user. Students can create tables and
manage data inside that database, but cannot create arbitrary databases or read
another student's database. Database names and usernames are stable identifiers;
display names and emails never become SQL identifiers.

## Administrator flow

1. Open **Hosting** in the sidebar and choose **Enable Hosting**.
   This starts the shared server and opens student registration. Registration is
   closed by default on fresh installations until the administrator enables it.
2. Share `/student-register`. Students enter their name, email and a password of
   at least 12 characters. Their hosting remains **pending** until approved.
   They can sign in immediately to view their pending status. Platform signup
   remains restricted to the first administrator or existing invitations.
3. Verify the student's identity before choosing **Approve**. Email ownership is
   not verified automatically; there is no email delivery service in this module.
4. Ferrix queues provisioning and creates a dedicated database/user, persistent
   web files volume, and PHP container with its automatic domain. **Retry** repairs
   a failed provisioning attempt using the same identities and volumes.
5. **Suspend** stops the PHP container, disables its public route and locks its
   database account. **Resume** unlocks it and restarts the same web container;
   files and database contents are preserved. **Reject** is available only for
   pending registrations. Closing registration does not suspend existing sites.

The PHP image must be available before approval. For local development:

```sh
docker build -t ferrix-student-php:local deploy/student-php
```

Add `FERRIX_STUDENT_PHP_IMAGE=ferrix-student-php:local` to the private root `.env`
and restart the API. The local nmfairus setup has been configured this way.
On a VPS use the matching released image:
`ghcr.io/eci4ever/ferrix-student-php:<version>`. The release workflow publishes
this image for amd64 along with Ferrix; make the new GHCR package public before
using it on a VPS. A release containing this module must be published first.
The production Compose file and new installer accept `FERRIX_STUDENT_PHP_IMAGE`.

## Student flow

After signing in, students go directly to **My hosting**, where they can:

- Check approval/provisioning status and open their website.
- Read their own database connection details, including a password reveal button.
- Upload a project folder containing `index.php`, PHP scripts and assets.
  Each upload permits up to 256 files and 8 MiB total, using relative filenames
  of up to 100 ASCII bytes. Hidden files, traversal and duplicate paths are
  rejected. Matching paths are replaced; files omitted from the upload remain.

PHP can connect without copying a password into source code:

```php
$pdo = new PDO(
    'mysql:host=' . getenv('DB_HOST') .
    ';dbname=' . getenv('DB_NAME') . ';charset=utf8mb4',
    getenv('DB_USER'),
    getenv('DB_PASSWORD'),
    [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]
);
```

Students cannot access platform Projects, Docker inventory, monitoring, member
administration or another student's hosting API. Each student has a private
organization membership, and server-side middleware restricts their sessions
regardless of what URL they request. Existing platform users are not converted
by submitting their email to student registration. Password hashing, session
expiry, origin checks, MFA and account bans reuse Ferrix authentication.

## Runtime and persistence

Names and labels are scoped to a persistent installation UUID stored in
`student_hosting_settings`, so changing the dashboard origin does not create a
new database server. The shared MariaDB has no host-published port. Web containers
join the private database network and Caddy ingress. The gateway authorizes
only active, non-banned students and verifies container ownership before routing.
Local domains use `student-<user UUID>.localhost`; VPS domains use
`student-<user UUID>.<IPv4-with-dashes>.sslip.io`. HTTPS follows existing Caddy
configuration, including trust of the local root certificate.

Each PHP container runs as uid 33, with a read-only root filesystem, no host
mounts, dropped capabilities, 256 MiB memory, 0.5 CPU, 64 PIDs and a 64 MiB
scratch directory. PHP's execution limit is 30 seconds and memory limit 128 MiB.
The shared MariaDB has 1 GiB memory, one CPU and a limit of 10 simultaneous
connections per student user. Credentials are encrypted with the Ferrix auth
key. Neither root credentials nor other students' credentials are returned.

There is **no hard persistent-disk quota** in this first version. Upload limits
apply per request; PHP code and SQL can create additional data. The database
server and Docker host remain shared resources. This module does not include
SFTP, Composer builds, phpMyAdmin, student deletion or automatic backup/restore.
Use trusted classroom workloads and monitor disk usage before increasing class
size. Websites are public independently of dashboard sign-in; students must
implement application login themselves if their sites need it.

Back up three things together: Ferrix PostgreSQL metadata and its `.env` auth
key, the shared MariaDB volume, and student web file volumes. Removing containers
alone preserves volumes. Do not prune volumes or rerun initialization against
missing metadata. The administrator remains responsible for database dumps
and restore procedures until backup/restore is implemented.

## Verification

Build the local PHP image, then run:

```sh
python3 scripts/tests/student-hosting.py
```

The test uses the current developer's PostgreSQL with a disposable schema, its
own MariaDB/web containers, owned volumes, ingress network and Caddy proxy. It
checks signup/approval, two students on one server, real PDO database isolation,
blocked peer/platform API requests, PHP upload, suspend/resume persistence and
browser flows. Only resources labeled with the disposable installation UUID
are removed. Credentials are generated for the test and are not written to the
repository. Browser screenshots are written under `/tmp/ferrix-student-hosting-*`.

References: [MariaDB GRANT](https://mariadb.com/docs/server/reference/sql-statements/account-management-sql-statements/grant),
[account locking](https://mariadb.com/docs/server/security/user-account-management/account-locking)
and [PDO MySQL](https://www.php.net/ref.pdo-mysql).

## Container grouping

The shared database appears as `mariadb` inside the installation's Compose
project in OrbStack. Its Docker name is `<project>-mariadb-1`, while PHP uses
the short private hostname `mariadb`. The existing UUID-based volume name and
legacy DNS alias are retained, preserving data and existing PHP connections.
`FERRIX_HOSTING_COMPOSE_PROJECT` selects the display group; local development
falls back to `COMPOSE_PROJECT_NAME`. Production Compose passes this explicitly.

Ferrix still manages this database through its Docker API and ownership labels.
It deliberately does not set a Compose config-hash, so ordinary Compose commands
do not adopt it as a configured YAML service. Hosting reconciliation starts and
checks it separately. Existing ungrouped containers require a controlled
replacement with the same volume; never start both against that volume at once.
