# Scheduled backups and S3 storage

Open a PostgreSQL or MariaDB service, choose **Backups → Schedule and storage**.
Save a five-field cron expression, an IANA timezone and the number of successful
backups to keep (1–365). For example, `0 2 * * *` with `Asia/Kuala_Lumpur` runs
at 02:00 Malaysia time. The next run and last scheduler message appear below the
form. Disable the checkbox to stop future scheduled runs; already queued jobs
continue. **Backup now** uses the saved destination even with scheduling disabled.

Schedules persist in PostgreSQL. The existing exclusive deployment/backup worker
claims due service rows, queues one job, and advances from the current time. A
restart combines missed slots into one run rather than replaying a backlog.
Busy deployments, backups and restores skip the current slot and show a reason.
Stopped databases produce the existing failed-job notification. A schedule is
paused if its configuring user is banned, removed, deleted or no longer an
owner/admin. Every queued job rechecks the current requester permissions.

## S3 destination

Select **Docker server and S3**, enter the existing bucket, region and optional
endpoint/prefix. Leave endpoint blank for AWS S3; S3-compatible services such as
R2, RustFS and MinIO use their HTTPS API endpoint. Path-style addressing is
available for compatible providers. Remote endpoints require HTTPS without URL
credentials, query strings or paths. HTTP loopback is allowed only when Ferrix
itself has a loopback development origin.

Enter an access key ID and secret key, with an optional session token. Use keys
restricted to this bucket/prefix and grant object read/write/delete plus multipart
upload operations. Ferrix does not create production buckets. Choose **Save
backup settings**, then **Test connection** to verify object write, read and
delete using a temporary probe. Changes must be saved before testing.

Credentials are encrypted with XChaCha20-Poly1305 under a separate authenticated
context tied to the service. They are never returned to the browser or included
in job history/logs. Leave the replacement checkbox off to preserve saved keys;
select it to rotate credentials. API keys cannot read or edit policy credentials.
An agent's existing deploy scope can still create a backup using the saved policy.

Each backup keeps an immutable encrypted destination snapshot and generated
object key under `<prefix>/ferrix/<service-id>/<backup-id>.dump`. Changing the
current destination or turning S3 off does not rewrite older backups. Keep old
credentials/buckets accessible while those backups are retained.

## Upload and restore

A consistent native dump is created on the dedicated Docker backup volume.
S3 upload starts only after the dump's size and SHA-256 are verified, and the
operation becomes successful only after upload completes. Successful S3 backups
keep their local copy. Upload/download use bounded streams and mode-0600
host temporary files, rather than buffering an entire database in RAM. Temporary
files are removed on completion/cancellation. Allow host temporary disk space for
one dump during upload and approximately two dumps while staging a restore.
Multipart upload is limited to one in-flight chunk to keep RAM use modest. Transfers use the existing twenty-minute database-operation timeout.

Restoring an S3 backup downloads from its original destination, verifies size and
SHA-256, stages it on Docker and uses the existing **new Database service** flow
with fresh credentials. It works even if the local backup volume has been lost.
The original database is not overwritten. Corrupt/missing objects fail before
importing into the target. Restores require the saved image to be available.
Dump/transfer helpers use the pinned database image, verified ownership labels,
restricted resources and no S3 credentials in container environment variables.

Back up Ferrix's own PostgreSQL metadata and `FERRIX_AUTH_KEY` off-host too: S3
contains the database dump, while its manifest, checksum and encrypted destination
live in Ferrix. Existing production backup instructions cover this metadata.
Deleting a service removes its history/policy but preserves S3 objects; use native
database tools for manual recovery when Ferrix metadata is no longer available.

## Retention and failures

After a new backup succeeds, retention marks older successful backups beyond
`keep_last` for removal. It never expires the only latest successful copies because
of a failed replacement. A queued/running restore protects its source backup.
Removal uses each backup's original S3 snapshot and its exact locally generated
filename, never a bucket-wide listing, prefix deletion or database-volume cleanup.

Pending removal cannot be restored. History shows **Removal pending**, **Removed
by retention**, or a sanitized retry error. Failed removals retry after five
minutes. Removal is complete only after remote and local cleanup succeeds. The
history row remains as a record. Existing backups have no automatic retention
until a policy is saved and a subsequent backup succeeds.

Interrupted jobs retain existing recovery behavior and emit failure notifications.
Incomplete multipart uploads or orphan objects from interrupted remote transfers
may require bucket lifecycle cleanup; configure an abort-incomplete-multipart
rule on the bucket. No remote credentials or provider error bodies are logged.

## API and verification

Browser owner/admin endpoints, scoped through the service's organization:

- `GET /api/v1/services/{id}/backup-policy`
- `POST /api/v1/services/{id}/backup-policy`
- `POST /api/v1/services/{id}/backup-policy/test`

Policy body: `enabled`, `cron`, `timezone`, `keep_last`, and `s3` (`null` for local).
S3 fields: `endpoint`, `region`, `bucket`, `prefix`, `path_style`, and optional
`credentials: { access_key, secret_key, session_token }`. Omit/null credentials to
reuse saved keys. Responses return only non-secret settings and
`credentials_saved`. Backup history adds storage, scheduled time and retention
state; the original backup/restore endpoints remain compatible.

`python3 scripts/tests/backup-s3.py` starts a disposable loopback RustFS instance
with random credentials, then runs real PostgreSQL and MariaDB S3 round trips.
Tests remove local backup volumes, change current policy, restore original data,
reject checksum tampering and verify remote retention deletion. Container removal
is restricted to that test's unique ownership label. Normal CI keeps the existing
local Docker backup tests and PostgreSQL scheduler/retention tests; live S3
coverage uses this separate disposable runner without cloud secrets.
