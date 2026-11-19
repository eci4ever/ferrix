#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
trap 'echo "Installation failed. Existing data is preserved; inspect /opt/ferrix and Docker logs." >&2' ERR

fail() { echo "Error: $*" >&2; exit 1; }
[[ ${EUID} -eq 0 ]] || fail 'Run as root (sudo bash).'
[[ $(uname -s) == Linux ]] || fail 'Only Linux VPS hosts are supported.'
case $(uname -m) in x86_64) ;; *) fail 'Requires an amd64 (Intel/AMD 64-bit) VPS.' ;; esac

# Set FERRIX_REF to a release tag; all installation files come from that revision.
ref=${FERRIX_REF:-main}
[[ $ref =~ ^[a-zA-Z0-9._/-]+$ && $ref != *..* ]] || fail 'Invalid FERRIX_REF.'
base="https://raw.githubusercontent.com/eci4ever/ferrix/${ref}"
version=${FERRIX_VERSION:-latest}
[[ $version =~ ^[a-zA-Z0-9_][a-zA-Z0-9_.-]*$ ]] || fail 'Invalid image version.'
command -v curl >/dev/null || fail 'Install curl first.'
public_ip=${FERRIX_PUBLIC_IP:-}
if [[ -z $public_ip ]]; then
  public_ip=$(curl -4 -fsS --connect-timeout 5 --max-time 10 https://api.ipify.org) || fail 'Could not detect public IPv4; set FERRIX_PUBLIC_IP explicitly.'
fi
[[ $public_ip =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail 'FERRIX_PUBLIC_IP must be an IPv4 address.'
IFS='.' read -r -a ip_parts <<< "$public_ip"
for part in "${ip_parts[@]}"; do
  [[ ${#part} -le 3 && ( $part == 0 || $part != 0* ) ]] && ((10#$part <= 255)) || fail 'Invalid IPv4 octet.'
done
((ip_parts[0] > 0 && ip_parts[0] < 224 && ip_parts[0] != 10 && ip_parts[0] != 127)) || fail 'Use a public IPv4 address.'
[[ $public_ip != 192.168.* && $public_ip != 169.254.* ]] || fail 'Use a public IPv4 address.'
! ((ip_parts[0] == 172 && ip_parts[1] >= 16 && ip_parts[1] <= 31)) || fail 'Use a public IPv4 address.'
domain=${FERRIX_DOMAIN:-}
if [[ -z $domain && -r /dev/tty ]]; then
  read -r -p 'Ferrix domain (leave blank for automatic sslip.io domain): ' domain </dev/tty || true
fi
domain=${domain:-ferrix.${public_ip//./-}.sslip.io}
[[ ${#domain} -le 253 && $domain == *.* && $domain =~ ^[a-zA-Z0-9]([a-zA-Z0-9.-]*[a-zA-Z0-9])?$ && $domain != *..* ]] || fail 'Set FERRIX_DOMAIN to a DNS hostname, without scheme or port.'
IFS='.' read -r -a labels <<< "$domain"
for label in "${labels[@]}"; do
  [[ ${#label} -le 63 && $label =~ ^[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?$ ]] || fail 'Invalid DNS label.'
done
[[ ! $domain =~ ^[0-9.]+$ ]] || fail 'Use a DNS hostname, not an IP address.'
command -v curl >/dev/null || fail 'Install curl first.'
command -v openssl >/dev/null || fail 'Install openssl first.'
command -v flock >/dev/null || fail 'Install util-linux (flock) first.'
exec 9>/run/ferrix-install.lock
flock -n 9 || fail 'Another Ferrix installation is running.'

if ! command -v docker >/dev/null; then
  # shellcheck source=/dev/null
  source /etc/os-release
  case ${ID:-} in ubuntu|debian) ;; *) fail 'Automatic Docker setup supports Ubuntu/Debian; install Docker Engine and Compose first.' ;; esac
  docker_script=$(mktemp)
  curl -fsSL --proto '=https' --tlsv1.2 https://get.docker.com -o "$docker_script"
  sh "$docker_script"
  rm -f "$docker_script"
fi
docker info >/dev/null
docker compose version >/dev/null || fail 'Install Docker Compose v2.'
[[ -S /var/run/docker.sock ]] || fail 'Requires the system Docker socket /var/run/docker.sock.'

install -d -m 700 /opt/ferrix
cd /opt/ferrix
[[ ! -e .env && ! -e compose.yaml && ! -e Caddyfile ]] || fail 'An installation exists. Use the documented update procedure; installer never overwrites it.'
# Pull first: a missing/private release must fail before creating installation credentials.
image="ghcr.io/eci4ever/ferrix:${version}"
docker pull "$image"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
curl -fsSL --proto '=https' --tlsv1.2 "$base/compose.production.yaml" -o "$staging/compose.yaml"
curl -fsSL --proto '=https' --tlsv1.2 "$base/deploy/Caddyfile" -o "$staging/Caddyfile"
password=$(openssl rand -hex 32)
key=$(openssl rand -hex 32)
socket_gid=$(stat -c '%g' /var/run/docker.sock)
timezone=${FERRIX_TIMEZONE:-UTC}
[[ $timezone =~ ^[a-zA-Z0-9_+/-]+$ && -f /usr/share/zoneinfo/$timezone && $timezone != *..* ]] || fail 'Invalid FERRIX_TIMEZONE.'
cat > "$staging/.env" <<ENV
FERRIX_IMAGE=$image
FERRIX_HOSTING_PHP_IMAGE=ghcr.io/eci4ever/ferrix-php-fpm:${version}
FERRIX_DOMAIN=$domain
FERRIX_PUBLIC_IP=$public_ip
FERRIX_AUTO_DOMAIN=sslip.io
POSTGRES_PASSWORD=$password
FERRIX_AUTH_KEY=$key
DOCKER_GID=$socket_gid
TZ=$timezone
ENV
docker compose --env-file "$staging/.env" -f "$staging/compose.yaml" config --quiet
install -m 600 "$staging/.env" .env
install -m 600 "$staging/compose.yaml" compose.yaml
install -m 600 "$staging/Caddyfile" Caddyfile
docker compose pull
docker compose up -d --wait --wait-timeout 180
echo "Ferrix started: https://$domain"
echo 'Ensure DNS points here and inbound TCP 80/443 is open. Register the first admin immediately.'
echo 'Back up /opt/ferrix/.env together with PostgreSQL; the auth key is needed to decrypt MFA secrets.'
