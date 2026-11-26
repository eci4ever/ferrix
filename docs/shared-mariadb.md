# Shared MariaDB application databases

An organization owner/admin can deploy one MariaDB 11.4 Database service and
open its **Database** tab to provision a logical database for an Application
in the same environment. Existing single-database services and their configured
database/account continue to work. No MariaDB port is published to the host.

Each linked Application has one generated database name, SQL username and
cryptographically random password. Credentials are encrypted using Ferrix's
existing authenticated encryption with the logical database UUID as associated
identity. SQL names contain only letters and digits, avoiding SQL identifier
injection and wildcard database grants. SQL is sent over Docker exec stdin;
statements and passwords are not included in command arguments or error messages.

The account has explicit DDL/DML grants on its own database, without global
privileges, GRANT OPTION, FILE, PROCESS or server administration. The default
limit is 10 simultaneous connections per account. CPU, memory, disk capacity
and I/O remain shared: this is credential/data isolation, not a hard resource
quota per database. Organization resource accounting counts the physical MariaDB
service once; logical database count is the number of records in its Database
tab and does not allocate additional containers, CPU or memory reservations.

## Access and application configuration

Only organization managers provision, rotate, revoke, back up, restore or delete
logical databases. An authorized developer of the linked Application can read
its limited connection settings, including through a project-scoped API key
with `secrets:read`. Viewers cannot read credentials. Student accounts cannot
obtain the shared Database service's root configuration or terminal through a
grant to their Application.

**Preview connection settings** shows the resulting Application environment.
Applying it replaces `DB_HOST`, `DB_PORT`, `DB_DATABASE`, `DB_USERNAME` and
`DB_PASSWORD`, preserving other variables. It saves configuration without
redeploying. Deploy the Application to apply it. Map these generic keys to the
framework's configuration as needed; no framework-specific URL is inferred.

Rotation requires typing the exact database name. It invalidates the old SQL
password for this Application; apply the new settings and redeploy it explicitly.
Sibling accounts are unchanged. A pending encrypted secret is persisted before
the SQL operation, so an interrupted rotation can be resumed from the same
button. Already-open SQL sessions may remain connected after rotation/revocation.

## Lifecycle and recovery

Provisioning is idempotent per Application. Metadata and encrypted secrets are
saved before SQL changes. Retry verifies a generated database ownership marker
and the account's generated password hash and absence of global grants. It
never adopts an unrelated database/account on name collision. Partial failures
remain visible and retryable. Do not manually change managed names, SQL account
passwords, ownership comments or PostgreSQL metadata.

- Removing a Ferrix member or assignment blocks Ferrix API access, but does not
  revoke a SQL password already disclosed. Use **Revoke SQL account** separately.
- Revocation locks new SQL connections and retains data. Existing sessions may
  remain connected. It does not delete the Application.
- Removing an Application retains its database/account and clears the link.
  Its retained data remains visible to managers in the shared service.
- **Delete database and account** is a separate exact-name confirmation. It
  permanently removes only that owned SQL database/account. Backup history
  blocks deletion so recoverable backups keep their mapping; archive/purge
  of managed backup histories is not implemented in this first version.
- Shared service deletion is blocked while any managed database record remains.
- Moving/cloning a linked Application or shared service to another environment
  is blocked, preventing credentials being copied into an unrelated environment.

Logical backups reuse `database_jobs`, the existing worker, checksums, backup
volumes, S3 destination snapshot and retention cleanup. Retention is partitioned
by logical database so one student's backups cannot evict another's. The
Database tab shows logical jobs; the Backups tab shows configured-database jobs.
Logical restore imports into the same managed database and preserves the current
SQL account/mapping/password. Pause the Application and external writers first;
restore replaces tables from the dump and is not an atomic rollback if import
fails. Sibling databases are not imported. A logical dump cannot be restored via
the configured-database/new-service restore endpoint.

The existing MariaDB Backups tab and its schedule cover the service's original
**configured database only**, not all logical databases or MariaDB account tables.
Each logical database has an **Automatic backups** section in the Database tab.
Owners/admins can enable a five-field cron schedule with an IANA timezone and
its own successful-backup retention count (1–365). Schedules default to disabled.
The shared service's saved local/S3 destination is snapshotted when each job is
queued; changing storage does not change already-created backups. Manual logical
backups use the same per-database retention setting when one is saved, otherwise
they inherit the shared service's retention. No sibling database history is pruned.

Due databases on one service run sequentially. Busy jobs, deployments and project
maintenance defer a due run; missed runs coalesce after restart. A database that
is not ready or has no eligible deployed MariaDB version is skipped with a visible
message and retried at its next scheduled time. Demoting/removing/banning the
manager who configured a schedule pauses it. Queued jobs recheck permissions.
Removing the linked Application retains its database and schedule; explicitly
deleting the logical database removes its schedule. Backups do not stop student
applications; coordinate writers and pause them before restoring a database.

Full shared-service disaster recovery remains follow-up work. Project bundles are explicitly blocked
for projects containing managed databases until bundle mapping recovery exists;
they must not silently produce an incomplete classroom backup.

## Real local verification

Run `python3 scripts/tests/shared-mariadb-live.py` from the mac2 clone. It uses
the current user's PostgreSQL database with a disposable schema/API port and
Ferrix-owned containers/networks/volumes. It requires the official `mariadb:11.4`
and `caddy:2-alpine` images. It verifies three students first, then 30 databases
on one MariaDB container, actual SQL DDL/DML and forbidden cross-database/global
operations, cookie/API-key/terminal authorization, duplicate/partial-failure
retries, collision refusal, rotation and real Application deployment, logical
backup/restore, revocation, retention after app deletion, and explicit data
deletion. Cleanup checks existing metadata and running containers were preserved.

Run `python3 scripts/tests/logical-backup-schedules-live.py` for real cron-triggered
A/B backups, restore isolation, independent retention and permission regression
checks in disposable resources. The test waits for a real cron minute and leaves
existing demo applications and their data unchanged.
