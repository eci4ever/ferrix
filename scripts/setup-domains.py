#!/usr/bin/env python3
"""Add automatic local website domains to this user's existing development profile."""
import getpass
from pathlib import Path

PROFILES = {"nmfairus": (8090, 80), "mac2": (8091, 9081), "mac3": (8092, 9082)}

def configure(root, user):
    if user not in PROFILES:
        raise SystemExit("No automatic-domain profile for this user.")
    path = root / '.env'
    if not path.is_file():
        raise SystemExit('Run scripts/setup-dev.py first.')
    lines = path.read_text().splitlines()
    values = dict(line.split('=', 1) for line in lines if '=' in line and not line.lstrip().startswith('#'))
    project = values.get('COMPOSE_PROJECT_NAME')
    if project != f'ferrix-{user}':
        raise SystemExit('Use only your own clone and Compose project.')
    listener, proxy = PROFILES[user]
    expected = {'FERRIX_WEBSITE_BIND': f'127.0.0.1:{listener}', 'FERRIX_WEBSITE_PORT': str(listener), 'FERRIX_WEBSITE_HTTP_PORT': str(proxy), 'FERRIX_AUTO_DOMAIN': 'localhost', 'FERRIX_INGRESS_NETWORK': f'{project}_ingress'}
    expected['FERRIX_WEBSITE_HTTPS_PORT'] = str({'nmfairus': 443, 'mac2': 9444, 'mac3': 9445}[user])
    if user == 'nmfairus':
        expected.update(FERRIX_WEBSITE_HTTPS='true', FERRIX_WEBSITE_HTTPS_PORT='443', FERRIX_LOCAL_CADDYFILE='Caddyfile.local-https')
    for key, value in expected.items():
        if key in values and values[key] != value:
            raise SystemExit(f'{key} is already configured differently; preserve it and reconcile manually.')
    additions = [f'{key}={value}' for key, value in expected.items() if key not in values]
    if additions:
        path.write_text('\n'.join(lines + additions) + '\n')
        path.chmod(0o600)
    address = 'https://<service>.localhost' if user == 'nmfairus' else f'http://<service>.localhost:{proxy}'
    print(f'Automatic domains configured: {address}; private gateway port {listener}.')

if __name__ == '__main__':
    configure(Path(__file__).resolve().parents[1], getpass.getuser())
