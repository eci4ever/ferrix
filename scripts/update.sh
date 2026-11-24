#!/usr/bin/env bash
# Upgrade a stock single-host installation; never run the fresh installer over existing data.
set -Eeuo pipefail
umask 077
fail() { echo "Error: $*" >&2; exit 1; }
[[ $EUID -eq 0 ]] || fail 'Run as root (sudo bash).'
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || fail 'Requires a Linux amd64 VPS.'
for command in curl docker flock timeout awk sed cmp sha256sum; do
  command -v "$command" >/dev/null || fail "Missing required command: $command"
done
[[ ${DOCKER_HOST:-unix:///var/run/docker.sock} == unix:///var/run/docker.sock && -S /var/run/docker.sock ]] || fail 'Requires the local system Docker socket.'
# Preserve the requested version, but load runtime settings only from the installation .env.
requested_version=${FERRIX_VERSION:-latest}
while IFS= read -r variable; do
  case $variable in FERRIX_*|COMPOSE_*|POSTGRES_PASSWORD|DOCKER_GID|TZ) unset "$variable";; esac
done < <(compgen -e)
unset DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH DOCKER_DEFAULT_PLATFORM
export DOCKER_HOST=unix:///var/run/docker.sock
exec 9>/run/ferrix-install.lock
flock -n 9 || fail 'Another Ferrix install or command update is running.'
cd /opt/ferrix
for file in .env compose.yaml Caddyfile; do
  [[ -f $file && ! -L $file ]] || fail "Expected a regular /opt/ferrix/$file. Use the manual upgrade guide."
done
[[ ! -L updates ]] || fail 'The updates directory must not be a symlink.'
[[ $(grep -c '^FERRIX_IMAGE=' .env) == 1 ]] || fail '.env must contain exactly one FERRIX_IMAGE entry.'

get() { curl -fsSL --proto '=https' --proto-redir '=https' --connect-timeout 10 --max-time 60 "$@"; }
stable() { [[ $1 =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; }
version=$requested_version
if [[ $version == latest ]]; then
  latest=$(get --max-redirs 5 -o /dev/null -w '%{url_effective}' https://github.com/eci4ever/ferrix/releases/latest)
  [[ $latest == https://github.com/eci4ever/ferrix/releases/tag/v* ]] || fail 'Could not resolve the latest stable Ferrix release.'
  version=${latest##*/}; version=${version#v}
else
  version=${version#v}
fi
stable "$version" || fail 'Use a stable version such as 0.2.4 or latest.'
[[ $(printf '%s\n' 0.2.4 "$version" | sort -V | head -1) == 0.2.4 ]] || fail 'This updater requires v0.2.4 or newer.'
tag="v$version"
base="https://raw.githubusercontent.com/eci4ever/ferrix/$tag"

# Derive the existing project before replacing any configuration.
source_container=$(docker compose --project-directory /opt/ferrix -f /opt/ferrix/compose.yaml ps -q ferrix)
[[ $source_container =~ ^[a-f0-9]{64}$ ]] || fail 'Expected exactly one running Ferrix container.'
project=$(docker inspect -f '{{index .Config.Labels "com.docker.compose.project"}}' "$source_container")
[[ $project =~ ^[a-z0-9][a-z0-9_-]{0,62}$ ]] || fail 'Invalid Compose project identity.'
compose() { docker compose --project-directory /opt/ferrix --env-file /opt/ferrix/.env -p "$project" -f /opt/ferrix/compose.yaml "$@"; }
[[ $(docker inspect -f '{{index .Config.Labels "com.docker.compose.project.working_dir"}}' "$source_container") == /opt/ferrix ]] || fail 'Use the manual guide for custom installation paths.'
[[ $(docker inspect -f '{{index .Config.Labels "com.docker.compose.project.config_files"}}' "$source_container") == /opt/ferrix/compose.yaml ]] || fail 'Use the manual guide for Compose overrides.'
source_image=$(docker inspect -f '{{.Image}}' "$source_container")
current=$(docker image inspect -f '{{index .Config.Labels "org.opencontainers.image.version"}}' "$source_image")
stable "$current" || fail 'Running image has no stable Ferrix release metadata.'
[[ $(printf '%s\n' "$current" "$version" | sort -V | head -1) == "$current" ]] || fail 'Downgrades require the manual recovery guide.'
old_ref=$(docker image inspect -f '{{range .RepoDigests}}{{println .}}{{end}}' "$source_image" | grep '^ghcr.io/eci4ever/ferrix@sha256:[0-9a-f]\{64\}$' | head -1)
[[ -n $old_ref ]] || fail 'Could not pin the existing official image for recovery.'
helper="$project-updater"
if helper_state=$(docker inspect -f '{{.State.Running}}' "$helper" 2>/dev/null); then
  [[ $helper_state == false ]] || fail 'A Ferrix UI or command update is already running.'
  helper_status=$(docker inspect -f '{{.State.Status}}' "$helper")
  [[ $helper_status == exited || $helper_status == dead ]] || fail 'Another updater is being prepared. Retry after it finishes.'
  [[ $(docker inspect -f '{{index .Config.Labels "io.ferrix.updater"}}' "$helper") == "$project" ]] || fail 'Updater name is occupied by another container.'
  docker rm "$helper" >/dev/null
fi

staging=$(mktemp -d)
backup=
reserved=0
stopped=0
changed=0
compatible=0
attempted=0
phase() { if [[ -n $backup ]]; then printf '%s\n' "$1" > "$backup/phase"; fi; echo "Ferrix update: $1"; }
finish() {
  code=$?
  trap - EXIT INT TERM
  if ((code != 0)); then
    if ((changed && (compatible || !attempted))); then
      echo 'Attempting recovery to the previous image and configuration.' >&2
      cp "$backup/env" .env.recovery
      sed "s|^FERRIX_IMAGE=.*|FERRIX_IMAGE=$old_ref|" .env.recovery > .env.recovery.pinned
      chmod 600 .env.recovery.pinned
      mv .env.recovery.pinned .env
      rm -f .env.recovery
      cp "$backup/compose.yaml" compose.yaml.recovery
      chmod 600 compose.yaml.recovery
      mv compose.yaml.recovery compose.yaml
      if timeout 240 docker compose --project-directory /opt/ferrix --env-file .env -p "$project" -f compose.yaml up -d --no-deps --pull never --wait --wait-timeout 180 ferrix; then
        phase rolled_back
      else phase failed; fi
    elif ((stopped && !changed)); then
      compose start ferrix || true
      phase failed
    else phase failed; fi
    echo "Update failed. Backup and recovery guide: ${backup:-not created}; https://github.com/eci4ever/ferrix/blob/$tag/docs/production.md#operate-and-update" >&2
  fi
  if ((reserved)); then
    # The marker remains until terminal status has been recorded in Docker logs.
    if ! timeout 10 docker wait "$helper" >/dev/null 2>&1; then docker stop -t 2 "$helper" >/dev/null 2>&1 || true; fi
  fi
  rm -rf "$staging"
  exit "$code"
}
trap finish EXIT
trap 'exit 1' INT TERM

get "$base/compose.production.yaml" -o "$staging/compose.yaml"
get "https://raw.githubusercontent.com/eci4ever/ferrix/v$current/compose.production.yaml" -o "$staging/previous-stock.yaml"
# Refuse to discard local customization, including edits to the Ferrix service.
if ! cmp -s compose.yaml "$staging/previous-stock.yaml" && ! cmp -s compose.yaml "$staging/compose.yaml"; then
  fail 'Compose is customized. Merge the release Compose changes manually; no configuration was changed.'
fi
# Infrastructure changes require deliberate migration, even between stock releases.
without_ferrix() { awk '/^  ferrix:$/ {skip=1;next} skip && (/^[^ ]/ || /^  [a-zA-Z0-9_-]+:/) {skip=0} !skip {print}' "$1"; }
without_ferrix compose.yaml > "$staging/infra-before"
without_ferrix "$staging/compose.yaml" > "$staging/infra-after"
cmp -s "$staging/infra-before" "$staging/infra-after" || fail 'This release changes infrastructure. Follow the manual upgrade guide.'

job=$(cat /proc/sys/kernel/random/uuid)
[[ $job =~ ^[a-f0-9-]{36}$ ]] || fail 'Could not create an update identity.'
backup="/opt/ferrix/updates/$job"
mkdir -p "$backup"
chmod 700 /opt/ferrix/updates "$backup"
cp .env "$backup/env"
cp compose.yaml "$backup/compose.yaml"
cp Caddyfile "$backup/Caddyfile"
chmod 600 "$backup/env" "$backup/compose.yaml" "$backup/Caddyfile"
echo "Target: $tag"
echo "Release notes: https://github.com/eci4ever/ferrix/releases/tag/$tag"
echo "Backup: $backup"
phase preparing
# Reserve the same running container name used by UI updates. It holds no socket or secrets.
# Unique Docker names prevent a simultaneous UI updater from starting.
docker create --name "$helper" --log-driver json-file --log-opt max-size=64k --log-opt max-file=1 --network none --read-only --cap-drop ALL --security-opt no-new-privileges --memory 16m --pids-limit 16 --user 0:0 \
  --label "io.ferrix.updater=$project" --label "io.ferrix.update-id=$job" --label "io.ferrix.target-version=$version" --label io.ferrix.command-update=true \
  --mount "type=bind,src=$backup/phase,dst=/progress-phase,readonly" --entrypoint /bin/sh "$source_image" -c '
    last=
    while :; do
      stage=$(cat /progress-phase)
      if [ "$stage" != "$last" ]; then printf "FERRIX_UPDATE_STAGE=%s\n" "$stage"; last=$stage; fi
      case "$stage" in succeeded) exit 0;; failed|rolled_back) exit 1;; esac
      sleep 1
    done' >/dev/null
reserved=1
docker start "$helper" >/dev/null

phase pulling
image="ghcr.io/eci4ever/ferrix:$version"
timeout 900 docker pull "$image"
[[ $(docker image inspect -f '{{.Architecture}}' "$image") == amd64 ]] || fail 'Target image is not amd64.'
[[ $(docker image inspect -f '{{index .Config.Labels "org.opencontainers.image.version"}}' "$image") == "$version" ]] || fail 'Target image version does not match the release.'
new_ref=$(docker image inspect -f '{{range .RepoDigests}}{{println .}}{{end}}' "$image" | grep '^ghcr.io/eci4ever/ferrix@sha256:[0-9a-f]\{64\}$' | head -1)
[[ -n $new_ref ]] || fail 'Could not pin the target image digest.'
old_schema=$(compose exec -T postgres psql -U ferrix -d ferrix -Atqc 'SELECT CASE WHEN EXISTS(SELECT 1 FROM _sqlx_migrations WHERE NOT success) THEN -1 ELSE COALESCE(max(version),0) END FROM _sqlx_migrations WHERE success')
new_schema=$(timeout 60 docker run --rm --network none --read-only --cap-drop ALL --security-opt no-new-privileges --entrypoint /usr/local/bin/ferrix-api "$new_ref" --schema-version)
[[ $old_schema =~ ^[0-9]+$ && $new_schema =~ ^[0-9]+$ && $new_schema -ge $old_schema ]] || fail 'Target migration version is incompatible.'
[[ $new_schema != "$old_schema" ]] || compatible=1
# Do not interrupt known deployment, backup, recovery or terminal work.
compose exec -T postgres psql -U ferrix -d ferrix -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
DO $$
DECLARE item record; busy boolean;
BEGIN
 FOR item IN SELECT * FROM (VALUES
 ('deployments','status IN (''queued'',''running'')'),
 ('database_jobs','status IN (''queued'',''running'')'),
 ('project_bundle_jobs','status IN (''queued'',''running'',''resuming'',''cutover_queued'',''cutting_over'')'),
 ('project_maintenance','true'),
 ('terminal_connections','closed_at IS NULL AND expires_at>now()')
 ) AS checks(name,condition) LOOP
  IF to_regclass(item.name) IS NOT NULL THEN
   EXECUTE format('SELECT EXISTS(SELECT 1 FROM %I WHERE %s)',item.name,item.condition) INTO busy;
   IF busy THEN RAISE EXCEPTION 'Finish pending deployments, backups, recovery jobs and terminals before updating.'; END IF;
  END IF;
 END LOOP;
END $$;
SQL
# Stop the old API before backing up when bootstrapping versions without the UI maintenance gate.
stopped=1
compose stop -t 120 ferrix
phase backing_up
timeout 600 docker compose --project-directory /opt/ferrix --env-file .env -p "$project" -f compose.yaml exec -T postgres pg_dump -U ferrix -d ferrix -Fc > "$backup/database.dump.part"
[[ -s $backup/database.dump.part ]] || fail 'Database backup is empty.'
compose exec -T postgres pg_restore --list < "$backup/database.dump.part" >/dev/null
mv "$backup/database.dump.part" "$backup/database.dump"
sha256sum "$backup/database.dump" > "$backup/database.sha256"

sed -e "s|^FERRIX_IMAGE=.*|FERRIX_IMAGE=$new_ref|" -e '/^FERRIX_SELF_UPDATE=/d' -e '/^COMPOSE_PROJECT_NAME=/d' .env > "$staging/env"
printf '\nCOMPOSE_PROJECT_NAME=%s\nFERRIX_SELF_UPDATE=true\n' "$project" >> "$staging/env"
docker compose --project-directory /opt/ferrix --env-file "$staging/env" -p "$project" -f "$staging/compose.yaml" config --quiet
phase updating
# From this point a new schema may be applied; recovery depends on schema compatibility.
changed=1
install -m 600 "$staging/env" .env.update
mv .env.update .env
install -m 600 "$staging/compose.yaml" compose.yaml.update
mv compose.yaml.update compose.yaml
sync
attempted=1
timeout 240 docker compose --project-directory /opt/ferrix --env-file .env -p "$project" -f compose.yaml up -d --no-deps --pull never --wait --wait-timeout 180 ferrix
phase verifying
[[ $(compose exec -T ferrix /usr/local/bin/ferrix-api --version) == "$version" ]] || fail 'Running version does not match target.'
[[ $(compose exec -T ferrix printenv FERRIX_SELF_UPDATE) == true ]] || fail 'UI updates were not enabled.'
compose exec -T ferrix curl -fsS http://127.0.0.1:8080/api/v1/auth/setup >/dev/null
phase succeeded
echo "Ferrix $tag is ready. Refresh your browser. Future releases can be installed from the admin sidebar."
