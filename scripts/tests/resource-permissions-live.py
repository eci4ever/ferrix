#!/usr/bin/env python3
"""Real Docker deployment + PTY permission test, isolated from developer data.

Uses the current clone's database, a disposable schema/API port and labeled
containers. No browser auth/session files, extra Python packages or mocks.
"""
import base64
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import struct
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[2]
PROFILE = {}
for line in (ROOT / '.env').read_text().splitlines():
    if line.strip() and not line.lstrip().startswith('#') and '=' in line:
        key, value = line.split('=', 1)
        PROFILE[key.strip()] = value.strip().strip('"').strip("'")
ENV = os.environ | PROFILE
DATABASE = urllib.parse.urlsplit(PROFILE['DATABASE_URL'])
assert DATABASE.hostname in ('127.0.0.1', 'localhost')
assert DATABASE.port == int(PROFILE['POSTGRES_PORT'])
assert DATABASE.path == '/' + PROFILE['POSTGRES_DB'], 'Use only this clone\'s database.'
SCHEMA = 'permissions_live_' + uuid.uuid4().hex
PSQL = ['docker', 'compose', 'exec', '-T', 'postgres', 'psql', '-v', 'ON_ERROR_STOP=1', '-U', PROFILE.get('POSTGRES_USER', 'ferrix'), '-d', PROFILE['POSTGRES_DB'], '-At', '-c']


def command(args):
    return subprocess.check_output(args, cwd=ROOT, env=ENV, text=True).strip()


def sql(query):
    return command(PSQL + ['SET client_min_messages=warning; ' + query]).removeprefix('SET\n')


def snapshot():
    # Exclude telemetry/session timestamps updated by the normal running API.
    return sql("SELECT json_build_object('organizations',(SELECT json_agg(row_to_json(x)) FROM (SELECT id,name,restrict_member_resources FROM auth_organizations ORDER BY id) x),'projects',(SELECT json_agg(row_to_json(x)) FROM (SELECT id,name,organization_id FROM projects ORDER BY id) x),'environments',(SELECT json_agg(row_to_json(x)) FROM (SELECT id,name,project_id FROM environments ORDER BY id) x),'services',(SELECT json_agg(row_to_json(x)) FROM (SELECT id,name,environment_id,image FROM applications ORDER BY id) x),'project_assignments',(SELECT json_agg(row_to_json(x)) FROM (SELECT * FROM project_assignments ORDER BY project_id,user_id) x),'service_assignments',(SELECT json_agg(row_to_json(x)) FROM (SELECT * FROM service_assignments ORDER BY service_id,user_id) x))")


class Client:
    def __init__(self, origin):
        self.origin = origin
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))

    def request(self, method, path, body=None, expected=200):
        request = urllib.request.Request(self.origin + '/api/v1' + path, method=method,
            data=json.dumps(body).encode() if body is not None else None,
            headers={'Origin': self.origin, 'Content-Type': 'application/json'})
        try:
            response = self.opener.open(request, timeout=20)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            status, raw = response.code, response.read()
        result = json.loads(raw) if raw else None
        assert status == expected, f'{method} {path}: expected {expected}, got {status}: {result}'
        return result

    def websocket(self, service, expected=101):
        return WebSocket(self, f'/api/v1/services/{service}/terminal', expected)


class WebSocket:
    """Minimal RFC6455 client for a real local Ferrix PTY, including Origin/Cookie."""
    def __init__(self, client, path, expected):
        parsed = urllib.parse.urlsplit(client.origin)
        self.socket = socket.create_connection((parsed.hostname, parsed.port), timeout=15)
        self.buffer = b''
        self.output = ''
        self.events = []
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        cookies = '; '.join(f'{cookie.name}={cookie.value}' for cookie in client.jar)
        request = (f'GET {path} HTTP/1.1\r\nHost: {parsed.netloc}\r\n'
            f'Origin: {client.origin}\r\nCookie: {cookies}\r\nUpgrade: websocket\r\n'
            f'Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n\r\n')
        self.socket.sendall(request.encode())
        while b'\r\n\r\n' not in self.buffer:
            chunk = self.socket.recv(4096)
            assert chunk, 'WebSocket handshake disconnected'
            self.buffer += chunk
        header, self.buffer = self.buffer.split(b'\r\n\r\n', 1)
        lines = header.decode().split('\r\n')
        status = int(lines[0].split()[1])
        if status != expected:
            self.socket.close()
            raise AssertionError(f'WebSocket {path}: expected {expected}, got {status}')
        if status != 101:
            self.socket.close()
            return
        headers = {name.lower(): value.strip() for line in lines[1:] if ':' in line for name, value in [line.split(':', 1)]}
        accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
        assert headers['sec-websocket-accept'] == accept

    def read(self, size):
        while len(self.buffer) < size:
            chunk = self.socket.recv(max(4096, size - len(self.buffer)))
            if not chunk:
                raise EOFError('WebSocket closed')
            self.buffer += chunk
        result, self.buffer = self.buffer[:size], self.buffer[size:]
        return result

    def send(self, value, opcode=1):
        payload = json.dumps(value).encode() if not isinstance(value, bytes) else value
        mask = secrets.token_bytes(4)
        length = len(payload)
        header = bytes([0x80 | opcode, 0x80 | (length if length < 126 else 126)])
        if length >= 126:
            header += struct.pack('!H', length)
        self.socket.sendall(header + mask + bytes(byte ^ mask[i % 4] for i, byte in enumerate(payload)))

    def receive(self):
        fragments, message_type = bytearray(), None
        while True:
            first, second = self.read(2)
            assert not second & 0x80, 'Server WebSocket frames must not be masked'
            opcode, length = first & 0x0f, second & 0x7f
            if length == 126:
                length = struct.unpack('!H', self.read(2))[0]
            elif length == 127:
                length = struct.unpack('!Q', self.read(8))[0]
            assert length <= 1024 * 1024, 'Unexpectedly large terminal frame'
            payload = self.read(length)
            if opcode == 8:
                raise EOFError('WebSocket closed')
            if opcode == 9:
                self.send(payload, 10)
                continue
            if opcode == 10:
                continue
            if opcode:
                message_type = opcode
            fragments.extend(payload)
            if first & 0x80:
                if message_type == 1:
                    event = json.loads(fragments)
                    self.events.append(event)
                    return event
                self.output = (self.output + fragments.decode(errors='replace'))[-65536:]
                return None

    def until(self, predicate, timeout=15):
        deadline = time.monotonic() + timeout
        while not predicate():
            self.socket.settimeout(max(.1, deadline - time.monotonic()))
            assert time.monotonic() < deadline, 'Timed out waiting for real terminal output'
            self.receive()

    def close(self):
        try:
            self.send(b'', 8)
        except OSError:
            pass
        self.socket.close()


def wait_deployment(client, service, deployment):
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        history = client.request('GET', f'/services/{service}/deployments')
        job = next(item for item in history if item['id'] == deployment)
        assert job['status'] != 'failed', f'Deployment failed: {job["logs"]}'
        if job['status'] == 'succeeded':
            runtime = client.request('GET', f'/services/{service}/runtime')
            assert runtime['state'] == 'running', runtime
            with urllib.request.urlopen(f'http://127.0.0.1:{runtime["host_port"]}/', timeout=10) as response:
                assert response.status == 200
            return runtime
        time.sleep(.5)
    raise AssertionError('Timed out waiting for deployment worker')


def scenario(origin):
    owner, lecturer, student_a, student_b = (Client(origin) for _ in range(4))
    password = secrets.token_urlsafe(32)
    owner.request('POST', '/auth/setup', {'name': 'Fixture platform', 'email': 'platform@example.test', 'password': password})
    for client, name, role in [(lecturer, 'lecturer', 'admin'), (student_a, 'student-a', 'member'), (student_b, 'student-b', 'member')]:
        invitation = owner.request('POST', '/auth/invitations', {'email': f'{name}@example.test', 'role': role})
        client.request('POST', '/auth/accept-invitation', {'name': name, 'email': f'{name}@example.test', 'password': password, 'token': invitation['token']})
    identity = lecturer.request('GET', '/auth/session')
    assert identity['system_role'] == 'user' and identity['role'] == 'admin'
    project = lecturer.request('POST', '/projects', {'name': 'Disposable Docker permissions class'}, expected=201)
    environment = lecturer.request('GET', f'/projects/{project["id"]}/environments')['environments'][0]['id']
    services = []
    for name, client in [('A', student_a), ('B', student_b)]:
        app = lecturer.request('POST', f'/environments/{environment}/services', {'kind': 'application', 'name': f'Student {name} live', 'image': 'caddy:2-alpine', 'container_port': 80, 'memory_mib': 256, 'cpus': .25, 'environment': f'FIXTURE_OWNER={name}'}, expected=201)
        user = client.request('GET', '/auth/session')['user_id']
        lecturer.request('POST', f'/services/{app["id"]}/assignments', {'user_id': user, 'permission': 'developer'}, expected=204)
        services.append(app['id'])
    service_a, service_b = services
    lecturer.request('POST', '/auth/organizations/access', {'restricted': True}, expected=204)
    runtimes = []
    for service, client, name in [(service_a, student_a, 'A'), (service_b, student_b, 'B')]:
        deployment = client.request('POST', f'/services/{service}/deploy', {}, expected=202)
        runtimes.append(wait_deployment(client, service, deployment['id']))
        print(f'PASS: student {name} deployed own Docker application; running and HTTP 200.', flush=True)
    assert runtimes[0]['container_id'] != runtimes[1]['container_id']
    sockets = []
    try:
        terminal_a = student_a.websocket(service_a)
        sockets.append(terminal_a)
        terminal_a.until(lambda: any(event.get('type') == 'ready' for event in terminal_a.events))
        token = uuid.uuid4().hex
        marker_a = 'FERRIX_A_' + token
        # The marker is split in the command, so terminal input echo cannot pass.
        terminal_a.send({'type': 'input', 'data': f"printf '%s%s\\n' FERRIX_A_ {token}; printf '%s' \"$FIXTURE_OWNER\" > /tmp/ferrix-permissions-{token}; cat /tmp/ferrix-permissions-{token}; id\n"})
        terminal_a.until(lambda: marker_a in terminal_a.output and 'uid=' in terminal_a.output)
        print('PASS: student A real Docker PTY executed printf, file write/read and id.', flush=True)
        terminal_b = student_b.websocket(service_b)
        sockets.append(terminal_b)
        terminal_b.until(lambda: any(event.get('type') == 'ready' for event in terminal_b.events))
        marker_b = 'FERRIX_B_ISOLATED_' + token
        terminal_b.send({'type': 'input', 'data': f"test ! -e /tmp/ferrix-permissions-{token} && test \"$FIXTURE_OWNER\" = B && printf '%s%s\\n' FERRIX_B_ISOLATED_ {token}\n"})
        terminal_b.until(lambda: marker_b in terminal_b.output)
        print('PASS: B terminal has B environment and cannot see A temporary file.', flush=True)
        for suffix in ['', '/configuration', '/environment', '/logs', '/deployments', '/source', '/notes', '/backups']:
            student_a.request('GET', f'/services/{service_b}{suffix}', expected=404)
        student_a.request('POST', f'/services/{service_b}/deploy', {}, expected=404)
        student_a.request('POST', f'/services/{service_b}/control', {'operation': 'stop'}, expected=404)
        student_a.websocket(service_b, expected=404)
        visible = student_a.request('GET', f'/environments/{environment}/services')
        assert [item['id'] for item in visible] == [service_a]
        assert len(student_b.request('GET', f'/services/{service_b}/deployments')) == 1
        assert student_b.request('GET', f'/services/{service_b}/runtime')['state'] == 'running'
        print('PASS: A denied B API, deploy, stop and real WebSocket handshake; B remains running.', flush=True)
        a_id = student_a.request('GET', '/auth/session')['user_id']
        lecturer.request('POST', f'/services/{service_a}/assignments', {'user_id': a_id, 'permission': None}, expected=204)
        denied_marker = 'FERRIX_REVOKED_' + token
        terminal_a.send({'type': 'input', 'data': f"printf '%s%s\\n' FERRIX_REVOKED_ {token}\n"})
        terminal_a.until(lambda: any(event.get('type') == 'closed' for event in terminal_a.events))
        assert any(event.get('type') == 'error' and ('permission' in event.get('message', '') or 'session' in event.get('message', '')) for event in terminal_a.events)
        assert denied_marker not in terminal_a.output, 'Revoked shell input executed'
        student_a.websocket(service_a, expected=404)
        assert lecturer.request('GET', f'/services/{service_a}/runtime')['state'] == 'running'
        assert student_b.request('GET', f'/services/{service_b}/runtime')['state'] == 'running'
        print('PASS: revoke closes existing A PTY, blocks further input/reconnect, and preserves both apps.', flush=True)
    finally:
        for connection in sockets:
            connection.close()


def cleanup_resources():
    inventory = json.loads(sql(f"SELECT json_build_object('apps',coalesce((SELECT json_agg(id) FROM {SCHEMA}.applications),'[]'::json),'environments',coalesce((SELECT json_agg(id) FROM {SCHEMA}.environments),'[]'::json))"))
    for app in inventory['apps']:
        app = str(uuid.UUID(app))
        ids = command(['docker', 'ps', '-aq', '--filter', f'label=io.ferrix.application={app}']).split()
        if ids:
            subprocess.run(['docker', 'rm', '-f', *ids], cwd=ROOT, env=ENV, check=True, stdout=subprocess.DEVNULL)
        for resource in ['volume', 'network']:
            ids = command(['docker', resource, 'ls', '-q', '--filter', f'label=io.ferrix.application={app}']).split()
            if ids:
                subprocess.run(['docker', resource, 'rm', *ids], cwd=ROOT, env=ENV, check=True, stdout=subprocess.DEVNULL)
    for environment in inventory['environments']:
        ids = command(['docker', 'network', 'ls', '-q', '--filter', f'label=io.ferrix.environment={uuid.UUID(environment)}']).split()
        if ids:
            subprocess.run(['docker', 'network', 'rm', *ids], cwd=ROOT, env=ENV, check=True, stdout=subprocess.DEVNULL)


def main(static_dir=None, runtime_overrides=None):
    before = snapshot()
    running = set(command(['docker', 'ps', '-q', '--no-trunc']).split())
    # Reuse an already installed official image; do not build or replace local images.
    command(['docker', 'image', 'inspect', 'caddy:2-alpine', '--format', '{{.Id}}'])
    subprocess.run(['cargo', 'build', '-p', 'ferrix-api'], cwd=ROOT, env=ENV, check=True)
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    query = urllib.parse.parse_qsl(DATABASE.query) + [('options', '-csearch_path=' + SCHEMA)]
    isolated = urllib.parse.urlunsplit(DATABASE._replace(query=urllib.parse.urlencode(query)))
    preview = ENV | {'DATABASE_URL': isolated, 'FERRIX_BIND': f'127.0.0.1:{port}', 'FERRIX_AUTH_ORIGIN': origin, 'FERRIX_AUTH_KEY': secrets.token_hex(32), 'FERRIX_AUTO_DOMAIN': 'off', 'FERRIX_PUBLIC_IP': '', 'FERRIX_WEBSITE_BIND': '127.0.0.1:0'}
    preview.pop('FERRIX_STATIC_DIR', None)
    if static_dir is not None:
        preview['FERRIX_STATIC_DIR'] = str(static_dir)
    if runtime_overrides:
        preview.update(runtime_overrides)
    sql(f'CREATE SCHEMA {SCHEMA}')
    process = None
    try:
        with tempfile.TemporaryFile() as log:
            process = subprocess.Popen([str(ROOT / 'target/debug/ferrix-api')], cwd=ROOT, env=preview, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            for _ in range(120):
                try:
                    with urllib.request.urlopen(origin + '/health', timeout=1) as response:
                        assert response.status == 200
                    break
                except OSError:
                    assert process.poll() is None, 'Disposable API exited during startup'
                    time.sleep(.25)
            else:
                raise AssertionError('Disposable API did not start')
            print('Disposable API and PostgreSQL schema ready; main mac2 runtime unchanged.', flush=True)
            scenario(origin)
    finally:
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        try:
            cleanup_resources()
        finally:
            sql(f'DROP SCHEMA {SCHEMA} CASCADE')
        assert snapshot() == before, 'Existing organization/project/service/assignment metadata changed'
        assert running <= set(command(['docker', 'ps', '-q', '--no-trunc']).split()), 'An existing container stopped'
        print('PASS: temporary schema/containers/networks/volumes removed; existing data and running containers preserved.', flush=True)


if __name__ == '__main__':
    main()
