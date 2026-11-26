# Classroom LAN: ferrix.pi.local

The mac2 installation stays on its existing loopback profile until the operator
chooses LAN exposure. These are explicit setup steps, not automatic DNS/CA setup.
Keep the current private `.env` and installation authentication key; do not replace
them with a fresh setup or copy another macOS user's credentials.

## Hostnames and ports

Use `ferrix.pi.local` for the dashboard and
`app-<application-UUID-without-hyphens>.ferrix.pi.local` for each application's
automatic URL. These are different hostnames. The existing mac2 dashboard port is
5174; private HTTPS ingress uses 9444. Other macOS users' ports remain unchanged.

On the server, use a stable LAN IP or DHCP reservation. The local verification
used `192.168.0.2`; verify this address before using it on another network.
Configure the school DNS to resolve these names to the actual server LAN IP,
including the application subdomains, or add explicit hosts-file entries on each
client. A base-host entry does not cover application subdomains; hosts files do
not support wildcard entries. No public DNS provider or Internet connection is
required to render already-deployed starter pages.

Example client hosts entries (replace the application hostname with the actual
one shown by Ferrix):

```text
192.168.0.2 ferrix.pi.local
192.168.0.2 app-<application-id>.ferrix.pi.local
```

## Opt-in mac2 server configuration

After setting client DNS/hosts, edit only mac2's ignored `.env`:

```dotenv
FERRIX_AUTH_ORIGIN=http://ferrix.pi.local:5174
FERRIX_AUTO_DOMAIN=private
FERRIX_INGRESS_IP=192.168.0.2
FERRIX_DOMAIN_SUFFIX=ferrix.pi.local
FERRIX_WEBSITE_HOST_BIND=192.168.0.2
FERRIX_WEBSITE_HTTPS=true
FERRIX_WEBSITE_HTTPS_PORT=9444
FERRIX_LOCAL_CADDYFILE=Caddyfile.local-https
```

Preserve `COMPOSE_PROJECT_NAME=ferrix-mac2`, existing database/ports,
`FERRIX_INGRESS_NETWORK` and the OrbStack Docker socket. Recreate only mac2's proxy
with `docker compose --profile websites up -d proxy`. Restart the API to reload
its configuration and start Vite with `npm run dev -- --host 192.168.0.2` from
`apps/web`. Open `http://ferrix.pi.local:5174` for the dashboard; authentication
cookies use that exact configured origin. Allow the chosen dashboard and ingress
ports only where intended on the LAN. Database ports remain private.

Changing private ingress changes automatic URLs. Existing localhost URLs are not
promised to remain available after opting in. Review Domains/Overview before
sharing student links. Existing applications already attached to this profile's
ingress need no source changes; older ones may need a redeploy to attach.

## Internal HTTPS trust

Use the public Caddy root certificate from the **same** mac2 proxy:

```sh
docker compose cp proxy:/data/caddy/pki/authorities/local/root.crt /tmp/ferrix-local-root.crt
```

Install/trust this root on each student device using its operating system's
certificate controls, then restart the browser. Never distribute `root.key`.
A different test proxy's CA does not establish trust for the normal installation.
The TLS hostname must match the selected application URL; bypassing certificate
validation is not a successful HTTPS check. Follow the macOS example in
[development.md](development.md#local-https) when testing on this Mac.

## Verification boundary

`python3 scripts/tests/php-lan-live.py --lan-ip 192.168.0.2` provisions a temporary
PHP/MariaDB application using a non-platform organization admin. It uses the real
LAN interface, an isolated Caddy proxy and the selected hostname suffix. It checks
that an untrusted CA is rejected, explicit CA trust validates the LAN response,
and an Internet-isolated Docker client loads both the PHP page and local CSS over
verified TLS. It does not edit system DNS/hosts/trust or the normal runtime profile.
Temporary schemas, applications, images, networks and proxy are cleaned afterward.

On the actual classroom network, additionally verify from a student device:

1. Dashboard and application hostnames resolve to the server's current LAN IP.
2. Login, own-service access and application HTTPS work without certificate errors.
3. The starter page/styles still work when the student's Internet is disconnected
   while the LAN connection remains available.

These device/network checks cannot be inferred from a server-side test. Initial
image pulls, Git downloads and Composer builds may still require Internet.
