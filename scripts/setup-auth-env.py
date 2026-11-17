#!/usr/bin/env python3
"""Add missing authentication settings to an existing local .env without exposing secrets."""
import getpass
import secrets
from pathlib import Path


def main():
    root = Path(__file__).resolve().parent.parent
    path = root / '.env'
    if not path.is_file():
        raise SystemExit('Run scripts/setup-dev.py first.')
    user = getpass.getuser()
    if user not in {'nmfairus', 'mac2', 'mac3'}:
        raise SystemExit('Use a configured Ferrix development account.')
    text = path.read_text()
    values = dict(line.split('=', 1) for line in text.splitlines()
                  if '=' in line and not line.lstrip().startswith('#'))
    defaults = {
        'FERRIX_AUTH_ORIGIN': f"http://127.0.0.1:{values.get('FERRIX_WEB_PORT', '5173')}",
        'FERRIX_AUTH_KEY': secrets.token_hex(32),
    }
    added = []
    for key, value in defaults.items():
        if key not in values:
            text += f'\n{key}={value}\n'
            added.append(key)
        elif not values[key].strip():
            raise SystemExit(f'{key} is blank; generate a unique value before starting Ferrix.')
    if added:
        path.write_text(text)
        path.chmod(0o600)
    print('Added auth settings: ' + ', '.join(added) if added else 'Auth settings already exist; preserved.')


if __name__ == '__main__':
    main()
