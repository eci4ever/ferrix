#!/usr/bin/env bash
# Runs ONLY inside an ephemeral test container; never run on a VPS.
set -Eeuo pipefail
[[ -e /.dockerenv && ${FERRIX_INSTALL_TEST:-} == 1 ]] || exit 1
mkdir /mocks
cat > /mocks/uname <<'MOCK'
#!/bin/bash
if [[ ${1:-} == -m ]]; then echo "${TEST_MACHINE_ARCH:-x86_64}"; else /usr/bin/uname "$@"; fi
MOCK
cat > /mocks/docker <<'MOCK'
#!/bin/bash
set -eu
printf '%s\n' "$*" >> /tmp/docker-calls
if [[ $1 == pull && ${FAIL_PULL:-0} == 1 ]]; then exit 1; fi
MOCK
cat > /mocks/curl <<'MOCK'
#!/bin/bash
set -eu
source_file= destination=
while (($#)); do
  case $1 in
    *compose.production.yaml) source_file=/src/compose.production.yaml ;;
    *deploy/Caddyfile) source_file=/src/deploy/Caddyfile ;;
    -o) shift; destination=$1 ;;
  esac
  shift
done
[[ -n $source_file && -n $destination ]]
cp "$source_file" "$destination"
MOCK
cat > /mocks/openssl <<'MOCK'
#!/bin/bash
od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
printf '\n'
MOCK
chmod +x /mocks/*
export FERRIX_PUBLIC_IP=203.0.113.10
export PATH=/mocks:$PATH FERRIX_DOMAIN=deploy.example.com FERRIX_TIMEZONE=UTC
# Unsupported ARM hosts fail before installation.
if TEST_MACHINE_ARCH=aarch64 bash /src/scripts/install.sh >/tmp/result 2>&1; then exit 1; fi
[[ ! -e /opt/ferrix/.env ]]
# Pull failures leave no installation credentials.
if FAIL_PULL=1 bash /src/scripts/install.sh >/tmp/result 2>&1; then exit 1; fi
[[ ! -e /opt/ferrix/.env ]]
# Successful installation creates root-only credentials, and starts Compose.
bash /src/scripts/install.sh >/tmp/result 2>&1
[[ $(stat -c %a /opt/ferrix/.env) == 600 ]]
[[ $(stat -c %a /opt/ferrix) == 700 ]]
grep -q 'FERRIX_AUTH_KEY=' /opt/ferrix/.env
grep -q 'up -d --wait' /tmp/docker-calls
before=$(sha256sum /opt/ferrix/.env)
if bash /src/scripts/install.sh >/tmp/result 2>&1; then exit 1; fi
[[ $before == "$(sha256sum /opt/ferrix/.env)" ]]
# Hostname injection/invalid labels fail before Docker is called.
for invalid in 'https://deploy.example.com' 'a.-bad.com' '127.0.0.1' 'a..com'; do
  if FERRIX_DOMAIN="$invalid" bash /src/scripts/install.sh >/tmp/result 2>&1; then exit 1; fi
done
echo 'Installer tests passed: pull failure, fresh install, permissions, existing install preservation, invalid hostnames.'

# Only this ephemeral container's installation fixture is removed.
rm /opt/ferrix/.env /opt/ferrix/compose.yaml /opt/ferrix/Caddyfile
FERRIX_DOMAIN= bash /src/scripts/install.sh </dev/null >/tmp/result 2>&1
grep -q '^FERRIX_DOMAIN=ferrix.203-0-113-10.sslip.io$' /opt/ferrix/.env
grep -q '^FERRIX_AUTO_DOMAIN=sslip.io$' /opt/ferrix/.env
for invalid in '127.0.0.1' '192.168.1.1' '172.16.0.1' '999.1.2.3' '1.2.3.04'; do
  if FERRIX_PUBLIC_IP="$invalid" bash /src/scripts/install.sh >/tmp/result 2>&1; then exit 1; fi
done
echo 'Automatic domain installer tests passed.'
