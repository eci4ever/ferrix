# Stateful Applications and background workers

Available from v0.2.1 with migration 0025. The v0.2.0 image does not include this feature.

In Application **Configuration**, enable **Persistent storage** before the first deployment and enter a dedicated container data directory such as `/opt/data`. Ferrix creates one local named volume, `ferrix-app-<service UUID without hyphens>`, with service, project and organization ownership labels. You cannot select a host path, another service's volume or a remote volume driver.

The same volume and numeric file ownership survive stop/start, redeploy, failed deployment recovery and image rollback. Ferrix does not recursively change UID/GID or reset data. Choose images whose runtime user can access the imported data. The mount directory cannot be added, removed or moved after a deployment has been queued; create a new service and import its data for such a change. Read-only mode can be changed through a new deployment.

All image-declared volumes must match the configured mount directory. Additional implicit anonymous volumes are rejected. This initial version supports one data volume per Application.

## Commands and workers

**Command arguments** is a JSON array, for example `["gateway", "run"]`. It replaces the image CMD while preserving ENTRYPOINT. `[]` retains the image default. Ferrix does not parse a shell command or expand variables. To run a shell intentionally, supply an explicit shell command in an image that provides it. Arguments are encrypted with the service configuration and snapshotted per deployment; use Environment for credentials instead of putting them into arguments or logs.

Enable **Background worker** for gateways, bots and queue consumers. Workers have no published host port, Caddy ingress connection or website domain. Readiness requires the process to remain running for two consecutive checks and any image-defined healthcheck to become healthy. It does not prove a Telegram/provider connection is healthy. This is a long-running service mode, not a one-shot job scheduler.

For Hermes, use its supported image (preferably a pinned digest), command `["gateway", "run"]`, `HERMES_HOME=/opt/data`, persistent storage at `/opt/data`, and Background worker mode. Keep tokens in imported data or encrypted environment variables.

## CPU and memory limits

Configurable limits require a source build after v0.2.1. The v0.2.1 image retains the fixed Application defaults.

Application Configuration includes **Memory limit (MiB)** and **CPU limit (cores)** for web applications and workers. Defaults remain **512 MiB and 1 CPU**, including older configurations/deployment snapshots without these fields. Database service limits are unchanged.

Memory is a whole number from 6 to 1,048,576 MiB; 1 MiB is 1,048,576 bytes. CPU is from 0.01 to 1024 cores in increments of 0.01. Zero, null and unlimited limits are not supported. These are hard Docker limits, not reservations or guarantees that capacity is available on the host. Docker may reject limits unsupported by the connected server.

The configuration API accepts `memory_mib` and `cpus`, for example `"memory_mib": 768, "cpus": 0.75`. Omitted fields use the defaults when saving a full configuration. Values are encrypted alongside the runtime configuration and snapshotted at queue time for registry and GitHub deployments. Changes require **Deploy**; Start/Reload preserve the existing container's deployed limits. Rollback uses the selected deployment's limits while saved configuration remains available for the next regular deployment. Monitoring reads the actual deployed Docker limits.

## Single-writer replacement and recovery

Stateful Applications and workers use stop-first replacement, with downtime:

1. Pull/build and prepare the candidate without starting its process.
2. Persist recovery information, disable the old container's automatic restart policy and stop it.
3. Verify no other running container uses the volume (or this worker service), then start the candidate.
4. Activate only after readiness succeeds and remove the old container.
5. If startup fails or Ferrix restarts mid-deployment, remove the failed candidate before restoring the old instance. An intentionally stopped old instance stays stopped on failure.

Lifecycle controls reject changes while a deployment is pending. The deployment worker is serialized with a PostgreSQL advisory lock. Unexpected containers attached to the data volume block deployment/recovery rather than permitting a second writer. These guarantees cover Ferrix operations; a Docker administrator can bypass them by manually starting containers. Keep the source gateway stopped during migration and never run a second poller against the same Telegram bot identity.

Rollback restores the old image, command, worker mode and mount settings, **not the volume contents**. Take a backup before an image that migrates its data; older software may not understand newer files or schemas.

## Import, export and backup

These operations are performed by a trusted Docker host administrator. They are not exposed through a file-browsing API. Use Ferrix Stop and wait until the service is stopped, with no pending deployment, before accessing its data. Disable GitHub auto-deploy temporarily. Stop the source Dokploy/Hermes service before exporting, and keep it stopped until the migration is validated.

For SQLite, prefer the application's supported backup command. Otherwise stop all writers cleanly and archive the **entire directory**, including any `-wal` and `-shm` files. Copying a live `.db` file alone can lose data. Archive permissions, numeric UID/GID, dotfiles, sessions and configuration together. Protect backup files because they can contain bot tokens and provider credentials.

For an existing bind mount, on the source host:

```sh
sudo sh -c 'umask 077; tar --numeric-owner -czpf /root/hermes-data.tar.gz -C /srv/hermes/data .'
```

Create the Ferrix Application with its storage and worker settings, use a temporary idle command supported by that image and deploy once to create the owned volume, then stop it. Do not start the gateway against empty data. Restore its actual command after import. Transfer the backup privately to the target host. Obtain the exact volume name from the configuration API's `storage_volume` field (or the service UUID); never reuse another service's volume. For a blank/new volume only, import a trusted archive:

```sh
# Replace YOUR_SERVICE_UUID_WITHOUT_HYPHENS with this service's UUID.
volume=ferrix-app-YOUR_SERVICE_UUID_WITHOUT_HYPHENS
docker volume inspect "$volume"
docker run --rm --network none --mount "type=volume,src=$volume,dst=/data" \
  --mount type=bind,src=/root/hermes-data.tar.gz,dst=/backup.tar.gz,readonly \
  debian:bookworm-slim tar --numeric-owner -xzpf /backup.tar.gz -C /data
```

Do not overlay an archive onto unrelated existing data: extraction does not remove files absent from the archive. For a clean restore, create a **new** Ferrix Application and import into its volume. Validate the archive source and paths first; importing untrusted archives as root is unsafe. The commands preserve ownership and do not print data or credentials.

Export a stopped service's volume to a private file:

```sh
umask 077
docker run --rm --network none --mount "type=volume,src=$volume,dst=/data,readonly" \
  debian:bookworm-slim tar --numeric-owner -czpf - -C /data . > application-data.tar.gz
```

Check the command succeeded and validate the archive before restarting. Store an encrypted copy off-host. The v0.2.1 image uses this administrator workflow. Source builds after v0.2.1 also provide managed offline Application backups, schedules and S3 copies as described below.

After import, start only the target service. Verify its identity, sessions, data permissions and connectivity without regenerating the Telegram token or pairing state. Retain the original backup until restart and redeploy have been verified.

## Deletion

Service deletion keeps its named volume by default. **Permanently delete data** requires the exact service name confirmation and deletes only volumes carrying that service's ownership labels, without force. Retained volumes are not automatically attached to newly created services; use the offline export/import workflow for recovery. Keep the volume name with your backups.

## Managed offline backups (source builds after v0.2.1)

Open an Application's **Backups** tab after configuring persistent storage and deploying it. **Backup now** requires explicit downtime confirmation. For schedules, acknowledge the offline policy, choose a five-field cron expression and IANA timezone, set successful-backup retention, then save. Manual backups use the saved destination even if scheduling is disabled. PostgreSQL/MariaDB retain their existing database-specific backup behavior.

Ferrix records the deployed image/runtime/environment snapshot and original container state/restart policy before stopping the Application. It stops all managed processes before reading the complete owned volume, including dotfiles and any SQLite WAL/SHM files still present. A fixed official Python helper runs with no network access and never executes the Application image. Archives preserve numeric UID/GID, modes, timestamps, files and directories. This version rejects symbolic links and special files; hard-linked regular files are copied as file data. It supports at most 100,000 entries and 20 GiB of uncompressed file data per archive. ACLs and extended attributes are not preserved.

The archive is finalized by atomic rename and directory fsync. History/API report its size and SHA-256. Deploy, rollback, Start/Reload and deletion remain blocked while a backup/restore is pending. Once archiving finishes, helpers are removed and the source resumes only if it was originally running; a stopped service stays stopped. Source downtime ends before S3 upload. A failure or restart removes the helper before recovering the original runtime state and policy. If safe recovery cannot be confirmed, the job remains pending and controls stay blocked until recovery succeeds.

Local archives live in a separate owned `ferrix-backups-<service UUID>` volume. Optional S3 copies use the existing encrypted destination configuration, HTTPS for remote endpoints and per-service/job object keys. A backup is successful only after the required upload completes. Failed/interrupted upload objects are queued for cleanup at their original destination. Retention removes eligible archives from their original local/S3 locations and protects backups with a pending restore; it never prunes live Application data.

Archives themselves are **not encrypted by Ferrix** and may contain tokens/provider credentials. Use a dedicated Docker host, private buckets, server-side encryption, least-privilege storage credentials and an encrypted off-host backup. S3 availability does not guarantee your bucket's encryption or access policy. S3 transfers stage the compressed archive in Ferrix's temporary directory; provide enough temporary capacity, especially when the production `/tmp` mount uses RAM-backed storage. No archive contents, credentials or filenames are included in job output.

### Restore into a new stopped service

Choose a completed, unpruned backup and select **Restore**. Ferrix creates a new Application in the same environment from the saved image, port, environment, commands, resource limits and storage settings. It creates the container without executing its entrypoint, disables automatic restart and restores only into its empty owned volume. S3 restores can recover after the local backup volume has been lost. Checksums and every archive entry are verified before extraction; absolute/traversal paths, links, devices and duplicate normalized paths are rejected.

The original immutable image must still be available on the Docker server; data archives do not include container images. Preserve locally built images separately, or recover the exact original registry image before restoring. Back up Ferrix metadata and `FERRIX_AUTH_KEY` separately so encrypted backup/runtime settings remain usable after host loss.

The target remains stopped after success. Check its files, numeric ownership and SQLite integrity before starting it. WAL recovery/integrity checking can require write access to the **offline target volume**; use a trusted inspection helper rather than starting the gateway. A failed/interrupted restore remains blocked from Start, Reload, Deploy and rollback; delete that target and restore again into a new service. The original service/data remain intact.

Restored services share a lineage with their source. Concurrent Start/Deploy and recovery operations are serialized for that lineage. Ferrix rejects Start/Deploy while another related service is running or is paused for backup and may resume. Stop the source first, then start only the selected target. Docker administrators can bypass these controls manually; do not run a second Telegram poller outside Ferrix. A GitHub-derived restored service uses its pinned saved image initially; reconnect its Source configuration deliberately for future GitHub builds. Schedules, webhook automation and S3 credentials are not automatically enabled on the new service.

API operations use existing scoped service routes. Application manual backup requires `POST /services/{id}/backups` with `{"offline":true}`. Application schedules require `application_offline:true` when enabling `POST /services/{id}/backup-policy`. Restore uses `POST /services/{id}/backups/{backup_id}/restore` with a new service name. Organization/project authorization and execution-time membership checks apply; backup contents are not exposed through a download or file-browsing API.
