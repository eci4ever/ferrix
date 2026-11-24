# Runs only in the pinned Docker CLI helper, not in the Ferrix container.
set -eu
umask 077
version=$1
project=$2
source_container=$3
old_ref=$4
job=$5
cd /opt/ferrix
[ -f .env ] && [ ! -L .env ] && [ -f compose.yaml ] && [ ! -L compose.yaml ]
[ ! -L updates ]
mkdir -p "updates/$job"
chmod 700 updates "updates/$job"
backup="/opt/ferrix/updates/$job"
exec 3>&1
exec >"$backup/update.log" 2>&1
stage() { printf 'FERRIX_UPDATE_STAGE=%s\n' "$1" >&3; }
compose() { docker compose --project-directory /opt/ferrix -p "$project" -f /opt/ferrix/compose.yaml "$@"; }
set_image() {
  # Never source .env or interpolate its contents into a command.
  awk -v image="$1" '/^FERRIX_IMAGE=/{count++; print "FERRIX_IMAGE=" image; next} {print} END {if(count!=1) exit 1}' .env > .env.update
  chmod 600 .env.update
  mv .env.update .env
}
stopped=0
changed=0
compatible=0
finish() {
  code=$?
  trap - EXIT INT TERM
  if [ "$code" -ne 0 ]; then
    if [ "$changed" -eq 0 ] && [ "$stopped" -eq 1 ]; then
      compose start ferrix || true
    elif [ "$changed" -eq 1 ] && [ "$compatible" -eq 1 ]; then
      stage rolling_back
      cp "$backup/env" .env.update
      chmod 600 .env.update
      mv .env.update .env
      set_image "$old_ref"
      if compose up -d --no-deps --pull never --wait --wait-timeout 180 ferrix; then
        stage rolled_back
        exit "$code"
      fi
    fi
    stage failed
  fi
  exit "$code"
}
trap finish EXIT
trap 'exit 1' INT TERM
stage preparing
[ "$(grep -c '^FERRIX_IMAGE=' .env)" = 1 ]
# Refuse altered/multi-service stacks; never update PostgreSQL, Caddy or user services.
[ "$(compose ps -q ferrix)" = "$source_container" ]
[ "$(compose config --services | sort | tr '\n' ' ')" = 'ferrix postgres proxy ' ]
cp .env "$backup/env"
cp compose.yaml "$backup/compose.yaml"
cp Caddyfile "$backup/Caddyfile"
chmod 600 "$backup/env" "$backup/compose.yaml" "$backup/Caddyfile"
stage pulling
image="ghcr.io/eci4ever/ferrix:$version"
docker pull "$image"
[ "$(docker image inspect -f '{{.Architecture}}' "$image")" = amd64 ]
[ "$(docker image inspect -f '{{index .Config.Labels "org.opencontainers.image.version"}}' "$image")" = "$version" ]
new_ref=$(docker image inspect -f '{{range .RepoDigests}}{{println .}}{{end}}' "$image" | grep '^ghcr.io/eci4ever/ferrix@sha256:[0-9a-f]\{64\}$' | head -1)
[ -n "$new_ref" ]
old_schema=$(docker exec "$source_container" /usr/local/bin/ferrix-api --schema-version)
new_schema=$(docker run --rm --network none --read-only --cap-drop ALL --security-opt no-new-privileges --entrypoint /usr/local/bin/ferrix-api "$new_ref" --schema-version)
case "$old_schema:$new_schema" in *[!0-9:]*|:*|*:) exit 1;; esac
[ "$new_schema" -ge "$old_schema" ]
if [ "$new_schema" = "$old_schema" ]; then compatible=1; fi
stage backing_up
# All managed lifecycle work was drained before the helper was started.
compose exec -T postgres pg_dump -U ferrix -d ferrix -Fc > "$backup/database.dump.part"
[ -s "$backup/database.dump.part" ]
compose exec -T postgres pg_restore --list < "$backup/database.dump.part"
mv "$backup/database.dump.part" "$backup/database.dump"
sha256sum "$backup/database.dump" > "$backup/database.sha256"
sync
stage updating
stopped=1
compose stop -t 120 ferrix
changed=1
set_image "$new_ref"
compose up -d --no-deps --pull never --wait --wait-timeout 180 ferrix
stage verifying
# Verify both the binary version and PostgreSQL-backed setup endpoint.
[ "$(compose exec -T ferrix /usr/local/bin/ferrix-api --version)" = "$version" ]
compose exec -T ferrix curl -fsS http://127.0.0.1:8080/api/v1/auth/setup
stage succeeded
