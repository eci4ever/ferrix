#!/usr/bin/env python3
"""Run Services browser tests with a disposable schema, free preview ports, and scoped cleanup."""
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request
import urllib.parse
import uuid

root = Path(__file__).resolve().parents[2]
profile = {}
for line in (root / '.env').read_text().splitlines():
    if line.strip() and not line.lstrip().startswith('#') and '=' in line:
        key, value = line.split('=', 1)
        profile[key.strip()] = value.strip().strip('"').strip("'")
url = os.environ.get('FERRIX_TEST_DATABASE_URL', profile['DATABASE_URL'])
parsed = urllib.parse.urlsplit(url)
assert parsed.hostname in ('127.0.0.1', 'localhost') and parsed.port == int(profile['POSTGRES_PORT']) and parsed.path == '/' + profile['POSTGRES_DB'], 'Use only this clone\'s own local development database.'
schema = 'services_ui_' + uuid.uuid4().hex
compose = ['docker', 'compose', 'exec', '-T', 'postgres', 'psql', '-v', 'ON_ERROR_STOP=1', '-U', 'ferrix', '-d', profile['POSTGRES_DB']]
env = os.environ | profile

def sql(query):
    return subprocess.check_output(compose + ['-At', '-c', query], cwd=root, env=env, text=True)

def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

def stop(process):
    if process and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()

api = web = None
sql(f'CREATE SCHEMA {schema}')
try:
    subprocess.run(['cargo', 'build', '-p', 'ferrix-api'], cwd=root, env=env, check=True)
    api_port, web_port = free_port(), free_port()
    while web_port == api_port:
        web_port = free_port()
    query = urllib.parse.parse_qsl(parsed.query)
    query.append(('options', '-csearch_path=' + schema))
    isolated_url = urllib.parse.urlunsplit(parsed._replace(query=urllib.parse.urlencode(query)))
    preview = env | {'DATABASE_URL': isolated_url, 'FERRIX_BIND': f'127.0.0.1:{api_port}', 'FERRIX_AUTH_ORIGIN': f'http://127.0.0.1:{web_port}', 'FERRIX_AUTH_KEY': secrets.token_hex(32), 'FERRIX_WEB_PORT': str(web_port)}
    preview.pop('FERRIX_STATIC_DIR', None)
    preview['FERRIX_AUTO_DOMAIN'] = 'off'
    preview['FERRIX_PUBLIC_IP'] = ''
    preview['FERRIX_WEBSITE_BIND'] = '127.0.0.1:0'  # Prevent dotenv from reloading the development port.
    with tempfile.TemporaryDirectory(prefix="ferrix-ui-auth-") as session_dir, tempfile.TemporaryFile() as api_log, tempfile.TemporaryFile() as web_log:
        api = subprocess.Popen([str(root / 'target/debug/ferrix-api')], cwd=root, env=preview, stdout=api_log, stderr=subprocess.STDOUT, start_new_session=True)
        web = subprocess.Popen(['npm', 'run', 'dev'], cwd=root / 'apps/web', env=preview, stdout=web_log, stderr=subprocess.STDOUT, start_new_session=True)
        for address in [f'http://127.0.0.1:{api_port}/health', f'http://127.0.0.1:{web_port}/signup']:
            for attempt in range(120):
                try:
                    with urllib.request.urlopen(address, timeout=1) as response:
                        assert response.status == 200
                    break
                except (OSError, AssertionError):
                    if attempt == 119:
                        raise RuntimeError('Preview failed to become ready; no user data was changed.')
                    time.sleep(.5)
        subprocess.run(['npm', 'run', 'test:e2e'], cwd=root / 'apps/web', env=env | {'FERRIX_E2E_URL': f'http://127.0.0.1:{web_port}', 'FERRIX_E2E_DISPOSABLE': '1', 'FERRIX_E2E_SESSION_CACHE': str(Path(session_dir) / 'session.json')}, check=True)
finally:
    stop(web)
    stop(api)
    try:
        inventory = json.loads(sql(f"SELECT json_build_object('apps',coalesce((SELECT json_agg(id) FROM {schema}.applications),'[]'::json),'environments',coalesce((SELECT json_agg(id) FROM {schema}.environments),'[]'::json))"))
        for app in inventory['apps']:
            app = str(uuid.UUID(app))
            ids = subprocess.check_output(['docker', 'ps', '-aq', '--filter', f'label=io.ferrix.application={app}'], env=env, text=True).split()
            if ids:
                subprocess.run(['docker', 'rm', '-f', *ids], env=env, check=True, stdout=subprocess.DEVNULL)
            for resource, action in [('volume', 'rm'), ('network', 'rm')]:
                ids = subprocess.check_output(['docker', resource, 'ls', '-q', '--filter', f'label=io.ferrix.application={app}'], env=env, text=True).split()
                if ids:
                    subprocess.run(['docker', resource, action, *ids], env=env, check=True, stdout=subprocess.DEVNULL)
        for environment in inventory['environments']:
            ids = subprocess.check_output(['docker', 'network', 'ls', '-q', '--filter', f'label=io.ferrix.environment={uuid.UUID(environment)}'], env=env, text=True).split()
            if ids:
                subprocess.run(['docker', 'network', 'rm', *ids], env=env, check=True, stdout=subprocess.DEVNULL)
    finally:
        sql(f'DROP SCHEMA {schema} CASCADE')
print('Services browser tests passed; disposable schema, containers, networks and volumes removed.')
