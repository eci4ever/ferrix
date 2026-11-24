# Stateful Applications and background workers

Available from v0.2.1 with migration 0025. The v0.2.0 image does not include this feature.

In Application **Configuration**, enable **Persistent storage** before the first deployment and enter a dedicated container data directory such as `/opt/data`. Ferrix creates one local named volume, `ferrix-app-<service UUID without hyphens>`, with service, project and organization ownership labels. You cannot select a host path, another service's volume or a remote volume driver.

The same volume and numeric file ownership survive stop/start, redeploy, failed deployment recovery and image rollback. Ferrix does not recursively change UID/GID or reset data. Choose images whose runtime user can access the imported data. The mount directory cannot be added, removed or moved after a deployment has been queued; create a new service and import its data for such a change. Read-only mode can be changed through a new deployment.

All image-declared volumes must match the configured mount directory. Additional implicit anonymous volumes are rejected. This initial version supports one data volume per Application.

## Commands and workers

**Command arguments** is a JSON array, for example `["gateway", "run"]`. It replaces the image CMD while preserving ENTRYPOINT. `[]` retains the image default. Ferrix does not parse a shell command or expand variables. To run a shell intentionally, supply an explicit shell command in an image that provides it. Arguments are encrypted with the service configuration and snapshotted per deployment; use Environment for credentials instead of putting them into arguments or logs.

Enable **Background worker** for gateways, bots and queue consumers. Workers have no published host port, Caddy ingress connection or website domain. Readiness requires the process to remain running for two consecutive checks and any image-defined healthcheck to become healthy. It does not prove a Telegram/provider connection is healthy. This is a long-running service mode, not a one-shot job scheduler.

For Hermes, use its supported image (preferably a pinned digest), command `["gateway", "run"]`, `HERMES_HOME=/opt/data`, persistent storage at `/opt/data`, and Background worker mode. Keep tokens in imported data or encrypted environment variables.

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

Check the command succeeded and validate the archive before restarting. Store an encrypted copy off-host. Application volume backups use this offline workflow; scheduled PostgreSQL/MariaDB Backups and S3 policies do not back up Application volumes.

After import, start only the target service. Verify its identity, sessions, data permissions and connectivity without regenerating the Telegram token or pairing state. Retain the original backup until restart and redeploy have been verified.

## Deletion

Service deletion keeps its named volume by default. **Permanently delete data** requires the exact service name confirmation and deletes only volumes carrying that service's ownership labels, without force. Retained volumes are not automatically attached to newly created services; use the offline export/import workflow for recovery. Keep the volume name with your backups.
