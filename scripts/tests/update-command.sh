#!/usr/bin/env bash
# ONLY inside a disposable container; all Docker/download operations are mocked.
set -Eeuo pipefail
[[ -e /.dockerenv && ${FERRIX_COMMAND_UPDATE_TEST:-} == 1 ]] || exit 1
mkdir -p /mocks /tmp/update-fixtures
awk '/^  ferrix:$/ {ferrix=1} ferrix && /^  [a-zA-Z0-9_-]+:/ && !/^  ferrix:$/ {ferrix=0} ferrix && (/FERRIX_SELF_UPDATE:/ || /^    labels:$/ || /io.ferrix.self-update:/) {next} {print}' /src/compose.production.yaml > /tmp/update-fixtures/old.yaml
cat > /mocks/uname <<'MOCK'
#!/bin/bash
if [[ $1 == -m ]]; then echo "${TEST_ARCH:-x86_64}"; else echo Linux; fi
MOCK
cat > /mocks/curl <<'MOCK'
#!/bin/bash
set -eu
url= destination= effective=0
while (($#)); do
 case $1 in
  https:*) url=$1;;
  -o) shift;destination=$1;;
  -w) shift;effective=1;;
 esac
 shift
done
[[ ${TEST_SCENARIO:-} != download_failure ]] || exit 22
if ((effective)); then echo -n https://github.com/eci4ever/ferrix/releases/tag/v0.2.4; exit; fi
case $url in
 */v0.2.3/compose.production.yaml) cp /tmp/update-fixtures/old.yaml "$destination";;
 */v0.2.4/compose.production.yaml)
  cp /src/compose.production.yaml "$destination"
  if [[ ${TEST_SCENARIO:-} == infra_change ]]; then sed -i 's/caddy:2-alpine/caddy:future/' "$destination"; fi;;
 *) exit 22;;
esac
MOCK
cat > /mocks/docker <<'MOCK'
#!/bin/bash
set -eu
# Runtime values from an invoking shell must never override the saved .env.
[[ ! -v FERRIX_AUTH_KEY && ! -v POSTGRES_PASSWORD && ! -v COMPOSE_PROJECT_NAME ]]
printf '%q ' "$@" >> /tmp/update-calls; printf '\n' >> /tmp/update-calls
scenario=${TEST_SCENARIO:-success}
source_id=$(printf 'a%.0s' {1..64})
source_image=$(printf 'c%.0s' {1..64})
old="ghcr.io/eci4ever/ferrix@sha256:$(printf 'd%.0s' {1..64})"
new="ghcr.io/eci4ever/ferrix@sha256:$(printf 'b%.0s' {1..64})"
case $1 in
 inspect)
  format=$3; object=$4
  case $format in
   *State.Running*) [[ $scenario == ui_busy ]] && { echo true;exit; }; [[ $scenario == ui_created ]] && { echo false;exit; };exit 1;;
   *State.Status*) echo created;;
   *project.working_dir*) echo /opt/ferrix;;
   *project.config_files*) echo /opt/ferrix/compose.yaml;;
   *com.docker.compose.project*) echo campus-ferrix;;
   *io.ferrix.updater*) echo campus-ferrix;;
   *Image*) echo "$source_image";;
   *) exit 1;;
  esac;;
 image)
  format=$4; object=$5
  case $format in
   *Architecture*) echo amd64;;
   *Labels*) [[ $object == "$source_image" ]] && echo 0.2.3 || echo 0.2.4;;
   *RepoDigests*) [[ $object == "$source_image" ]] && echo "$old" || echo "$new";;
   *) exit 1;;
  esac;;
 create) echo reserved;;
 start|wait|rm) echo 0;;
 pull) [[ $scenario != pull_failure ]];;
 run) [[ $scenario == schema_failure ]] && echo 32 || echo 31;;
 compose)
  shift
  while (($#)); do
   case $1 in --project-directory|--env-file|-p|-f) shift 2;;*)break;;esac
  done
  case $1 in
   ps) echo "$source_id";;
   config) :;;
   stop) :;;
   start) :;;
   up)
    count=$(cat /tmp/up-count 2>/dev/null || echo 0); count=$((count+1));echo "$count" > /tmp/up-count
    if [[ $count == 1 && ( $scenario == health_failure || $scenario == schema_failure ) ]]; then exit 1;fi;;
   exec)
    if [[ $4 == psql ]]; then
     if [[ $* == *-Atqc* ]]; then echo 31;else cat >/tmp/checked-work;[[ $scenario != pending_work ]];fi
    elif [[ $4 == pg_dump ]]; then echo 'verified database archive';[[ $scenario != backup_failure ]]
    elif [[ $4 == pg_restore ]]; then read -r archive;[[ $archive == 'verified database archive' ]]
    elif [[ $4 == /usr/local/bin/ferrix-api ]];then echo 0.2.4
    elif [[ $4 == printenv ]];then echo true
    elif [[ $4 == curl ]];then echo '{"setup_required":false}'
    else exit 1;fi;;
   *)exit 1;;
  esac;;
 *)exit 1;;
esac
MOCK
chmod +x /mocks/*
export PATH=/mocks:$PATH
setup() {
 rm -rf /opt/ferrix
 mkdir -p /opt/ferrix
 chmod 700 /opt/ferrix
 cp /tmp/update-fixtures/old.yaml /opt/ferrix/compose.yaml
 cp /src/deploy/Caddyfile /opt/ferrix/Caddyfile
 printf '%s\n' FERRIX_IMAGE=ghcr.io/eci4ever/ferrix:0.2.3 FERRIX_AUTH_KEY=preserved-auth-key POSTGRES_PASSWORD=preserved-password FERRIX_DOMAIN=preserved.example.com FERRIX_SELF_UPDATE=false > /opt/ferrix/.env
 chmod 600 /opt/ferrix/.env /opt/ferrix/compose.yaml /opt/ferrix/Caddyfile
 rm -f /tmp/update-calls /tmp/up-count
}
run_update() {
 # Deliberately hostile shell overrides: all must be ignored except FERRIX_VERSION.
 FERRIX_AUTH_KEY=wrong POSTGRES_PASSWORD=wrong COMPOSE_PROJECT_NAME=wrong bash /src/scripts/update.sh >/tmp/update-output 2>&1
}
for scenario in download_failure pull_failure ui_busy ui_created infra_change pending_work backup_failure health_failure schema_failure custom success; do
 setup
 export TEST_SCENARIO=$scenario
 if [[ $scenario == custom ]];then echo '# user customization' >> /opt/ferrix/compose.yaml;fi
 before_env=$(sha256sum /opt/ferrix/.env)
 before_compose=$(sha256sum /opt/ferrix/compose.yaml)
 if run_update; then code=0;else code=$?;fi
 if [[ $scenario == success ]];then
  [[ $code == 0 ]] || { cat /tmp/update-output;exit 1; }
  grep -q '^FERRIX_AUTH_KEY=preserved-auth-key$' /opt/ferrix/.env
  grep -q '^POSTGRES_PASSWORD=preserved-password$' /opt/ferrix/.env
  grep -q '^FERRIX_DOMAIN=preserved.example.com$' /opt/ferrix/.env
  grep -q '^COMPOSE_PROJECT_NAME=campus-ferrix$' /opt/ferrix/.env
  grep -q '^FERRIX_SELF_UPDATE=true$' /opt/ferrix/.env
  grep -q '^FERRIX_IMAGE=ghcr.io/eci4ever/ferrix@sha256:' /opt/ferrix/.env
  cmp /src/compose.production.yaml /opt/ferrix/compose.yaml
  [[ $(stat -c %a /opt/ferrix/.env) == 600 ]]
  backup=$(find /opt/ferrix/updates -name database.dump -print -quit)
  [[ -s $backup && $(stat -c %a "${backup%/*}/env") == 600 ]]
  grep -q 'up -d --no-deps --pull never --wait' /tmp/update-calls
 elif [[ $scenario == health_failure ]];then
  [[ $code != 0 && $(cat /tmp/up-count) == 2 ]]
  grep -q '^FERRIX_IMAGE=ghcr.io/eci4ever/ferrix@sha256:dddd' /opt/ferrix/.env
  cmp /tmp/update-fixtures/old.yaml /opt/ferrix/compose.yaml
 elif [[ $scenario == schema_failure ]];then
  [[ $code != 0 && $(cat /tmp/up-count) == 1 ]]
  [[ -n $(find /opt/ferrix/updates -name database.dump -print -quit) ]]
 else
  [[ $code != 0 ]] || { echo "Unexpected success: $scenario";exit 1; }
  [[ $before_env == "$(sha256sum /opt/ferrix/.env)" && $before_compose == "$(sha256sum /opt/ferrix/compose.yaml)" ]]
  if [[ $scenario == backup_failure ]];then grep -q 'start ferrix' /tmp/update-calls;fi
 fi
 if grep -q 'preserved-auth-key\|preserved-password' /tmp/update-output;then echo 'Secret leaked';exit 1;fi
 echo "Command updater: $scenario passed"
done
setup
if FERRIX_VERSION=../../bad run_update;then exit 1;fi
if TEST_ARCH=aarch64 run_update;then exit 1;fi
# An explicit stable version uses matching assets as well.
TEST_SCENARIO=success FERRIX_VERSION=0.2.4 run_update
