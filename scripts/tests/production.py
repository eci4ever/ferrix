#!/usr/bin/env python3
"""Smoke test the real production image with isolated Compose volumes and HTTPS.
Requires Docker Compose v2.24.4+ (the test override uses !override).
Usage: python3 scripts/tests/production.py [local-image]
"""
import http.cookiejar
import json
import os
from pathlib import Path
import re
import secrets
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

root = Path(__file__).resolve().parents[2]
image = sys.argv[1] if len(sys.argv) > 1 else 'ferrix:production-test'
project = f'ferrix-production-smoke-{os.getpid()}'
# Fixed loopback-only test ports; fail rather than disturbing an existing service.
origin = 'https://localhost:18443'
socket_gid = subprocess.check_output(['docker', 'run', '--rm', '--entrypoint', 'stat', '-v', '/var/run/docker.sock:/var/run/docker.sock:ro', image, '-c', '%g', '/var/run/docker.sock'], text=True).strip()
assert socket_gid.isdigit()
with tempfile.TemporaryDirectory(prefix='ferrix-production-') as directory:
    fixture = Path(directory)
    (fixture / '.env').write_text(f'FERRIX_IMAGE={image}\nFERRIX_DOMAIN=localhost\nPOSTGRES_PASSWORD={secrets.token_hex(32)}\nFERRIX_AUTH_KEY={secrets.token_hex(32)}\nDOCKER_GID={socket_gid}\nTZ=Asia/Kuala_Lumpur\n')
    (fixture / '.env').chmod(0o600)
    caddyfile = (root / 'deploy/Caddyfile').read_text()
    (fixture / 'Caddyfile').write_text(caddyfile.replace('{\n', '{\n    local_certs\n', 1))
    (fixture / 'override.yaml').write_text(f'''services:
  ferrix:
    environment:
      FERRIX_AUTH_ORIGIN: {origin}
  proxy:
    ports: !override
      - '127.0.0.1:18443:443'
    volumes:
      - {fixture}/Caddyfile:/etc/caddy/Caddyfile:ro
''')
    compose = ['docker', 'compose', '-p', project, '--env-file', str(fixture / '.env'), '-f', str(root / 'compose.production.yaml'), '-f', str(fixture / 'override.yaml')]
    cookies = http.cookiejar.CookieJar()
    client = urllib.request.build_opener(urllib.request.HTTPSHandler(context=ssl._create_unverified_context()), urllib.request.HTTPCookieProcessor(cookies))

    def request(path, payload=None, origin_header=origin):
        headers = {'Origin': origin_header}
        if payload is not None:
            headers['Content-Type'] = 'application/json'
        req = urllib.request.Request(origin + path, data=None if payload is None else json.dumps(payload).encode(), headers=headers)
        return client.open(req, timeout=10)

    try:
        subprocess.run(compose + ['up', '-d', '--wait', '--wait-timeout', '120'], check=True)
        for attempt in range(20):
            try:
                with request('/health') as response:
                    assert json.load(response)['status'] == 'ok'
                break
            except (OSError, urllib.error.URLError):
                if attempt == 19:
                    raise
                time.sleep(1)
        with request('/') as response:
            html = response.read().decode()
            assert 'text/html' in response.headers['Content-Type']
        asset = re.search(r'src="(/assets/[^\"]+\.js)"', html).group(1)
        with request(asset) as response:
            assert 'javascript' in response.headers['Content-Type']
        for path in ['/login', '/signup', '/dashboard', '/favicon.svg']:
            with request(path) as response:
                assert response.status == 200
        try:
            request('/api/not-an-endpoint')
            raise AssertionError('Unknown API endpoint must not return SPA HTML')
        except urllib.error.HTTPError as error:
            assert error.code == 404
        with request('/api/v1/auth/setup') as response:
            assert json.load(response)['setup_required'] is True
        registration = {'name': 'Production Test', 'email': 'production@example.com', 'password': secrets.token_urlsafe(24)}
        try:
            request('/api/v1/auth/setup', registration, 'https://evil.example')
            raise AssertionError('Cross-origin setup accepted')
        except urllib.error.HTTPError as error:
            assert error.code == 403
        with request('/api/v1/auth/setup', registration) as response:
            assert 'Secure' in response.headers['Set-Cookie']
            assert 'HttpOnly' in response.headers['Set-Cookie']
        with request('/api/v1/auth/session') as response:
            identity = json.load(response)
            assert identity['system_role'] == 'admin'
            assert identity['role'] == 'owner'
        with request('/api/v1/status') as response:
            status = json.load(response)
            assert status['database'] is True
            assert status['docker'] is True
        with request('/api/v1/server-time') as response:
            clock = json.load(response)
            assert clock['timezone'] == 'Asia/Kuala_Lumpur'
            assert clock['utc_offset'] == 'UTC+08:00'
        with request('/api/v1/projects', {'name': 'Persistent production test'}) as response:
            assert response.status in (200, 201)
            project_id = json.load(response)['id']
        with request(f'/api/v1/projects/{project_id}/environments') as response:
            environment_id = json.load(response)['environments'][0]['id']
        for kind in ('compose', 'static_website'):
            try:
                request(f'/api/v1/environments/{environment_id}/services', {'kind':kind,'name':'unsupported'})
                raise AssertionError('Retired service type was accepted')
            except urllib.error.HTTPError as error:
                assert error.code == 422
        subprocess.run(compose + ['restart', 'ferrix'], check=True)
        for attempt in range(20):
            try:
                with request('/api/v1/projects') as response:
                    assert 'Persistent production test' in response.read().decode()
                break
            except (OSError, urllib.error.URLError):
                if attempt == 19:
                    raise
                time.sleep(1)
        with request('/api/v1/auth/sign-out', {}) as response:
            assert response.status in (200, 204)
        with request('/api/v1/auth/sign-in', {'email': registration['email'], 'password': registration['password']}) as response:
            assert response.status == 200
        print('Production smoke passed: HTTPS, SPA routes/assets, migrations, secure cookies, first admin, origin checks, clock, service-type rejection, restart persistence and login.')
    finally:
        subprocess.run(compose + ['down', '-v', '--remove-orphans'], check=True)
