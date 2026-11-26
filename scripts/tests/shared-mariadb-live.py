#!/usr/bin/env python3
"""Real shared MariaDB isolation; disposable API/schema/owned Docker resources."""
import argparse
import importlib.util
from pathlib import Path
import json
import secrets
import subprocess
import time
import uuid
import urllib.error
import urllib.request

spec = importlib.util.spec_from_file_location('live', Path(__file__).with_name('resource-permissions-live.py'))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
live.SCHEMA = 'mariadb_live_' + uuid.uuid4().hex


def account_sql(container, connection, statement, allowed=True):
    env = dict(line.split('=', 1) for line in connection['environment'].splitlines())
    # Password is stdin, never a process argument or printed test output.
    script = 'IFS= read -r MYSQL_PWD; export MYSQL_PWD; exec mariadb --protocol=tcp --host=127.0.0.1 --batch --skip-column-names --user="$1" "$2"'
    result = subprocess.run(['docker', 'exec', '-i', container, '/bin/sh', '-c', script, 'ferrix-isolation-test', env['DB_USERNAME'], env['DB_DATABASE']], input=env['DB_PASSWORD'] + '\n' + statement + '\n', text=True, env=live.ENV, capture_output=True, timeout=20)
    assert (result.returncode == 0) == allowed, 'Unexpected MariaDB account permission result'
    if not allowed:
        assert 'denied' in result.stderr.lower(), 'Expected actual SQL access denial'
    return result.stdout.strip()


def root_sql(container, statement):
    result = subprocess.run(['docker', 'exec', '-i', container, '/bin/sh', '-c', 'MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb --protocol=socket --user=root --batch --skip-column-names'], input=statement + '\n', text=True, env=live.ENV, capture_output=True, timeout=20)
    assert result.returncode == 0, 'Fixture root SQL failed'
    return result.stdout.strip()


def scenario(origin):
    owner, lecturer, a, b, c = (live.Client(origin) for _ in range(5))
    password = secrets.token_urlsafe(32)
    owner.request('POST', '/auth/setup', {'name': 'Fixture platform', 'email': 'platform@example.test', 'password': password})
    for client, name, role in [(lecturer, 'lecturer', 'admin'), (a, 'student-a', 'member'), (b, 'student-b', 'member'), (c, 'student-c', 'member')]:
        invitation = owner.request('POST', '/auth/invitations', {'email': name + '@example.test', 'role': role})
        client.request('POST', '/auth/accept-invitation', {'name': name, 'email': name + '@example.test', 'password': password, 'token': invitation['token']})
    project = lecturer.request('POST', '/projects', {'name': 'Disposable shared MariaDB class'}, expected=201)
    environment = lecturer.request('GET', f'/projects/{project["id"]}/environments')['environments'][0]['id']
    db = lecturer.request('POST', f'/environments/{environment}/services', {'kind': 'database', 'name': 'Shared class MariaDB', 'engine': 'mariadb'}, expected=201)
    dbid = db['id']
    job = lecturer.request('POST', f'/services/{dbid}/deploy', {}, expected=202)
    for _ in range(240):
        current = next(item for item in lecturer.request('GET', f'/services/{dbid}/deployments') if item['id'] == job['id'])
        assert current['status'] != 'failed', 'MariaDB deployment failed: ' + current['logs']
        if current['status'] == 'succeeded':
            break
        time.sleep(.5)
    else:
        raise AssertionError('MariaDB deployment timed out')
    container = lecturer.request('GET', f'/services/{dbid}/runtime')['container_id']
    bindings = json.loads(live.command(['docker', 'inspect', container, '--format', '{{json .HostConfig.PortBindings}}']))
    assert not bindings, 'MariaDB must remain private'
    print('PASS: real MariaDB 11.4 deployed by Ferrix, no published host port.', flush=True)
    apps, records, connections = [], [], []
    for name, student in [('A', a), ('B', b), ('C', c)]:
        app = lecturer.request('POST', f'/environments/{environment}/services', {'kind': 'application', 'name': 'Student ' + name, 'image': 'caddy:2-alpine', 'container_port': 80, 'environment': 'UNRELATED=preserve'}, expected=201)
        appid = app['id']; apps.append(appid)
        lecturer.request('POST', f'/services/{appid}/assignments', {'user_id': student.request('GET', '/auth/session')['user_id'], 'permission': 'developer'}, expected=204)
        record = lecturer.request('POST', f'/services/{dbid}/databases', {'application_id': appid})
        assert record['status'] == 'ready' and 'credentials_cipher' not in record
        records.append(record)
        duplicate = lecturer.request('POST', f'/services/{dbid}/databases', {'application_id': appid})
        assert duplicate['id'] == record['id']
        preview = lecturer.request('GET', f'/services/{dbid}/databases/{record["id"]}/environment')
        assert preview['before'] == 'UNRELATED=preserve' and 'UNRELATED=preserve' in preview['after']
        lecturer.request('POST', f'/services/{dbid}/databases/{record["id"]}/environment', {'database_name': record['database_name']}, expected=204)
        connection = student.request('GET', f'/services/{appid}/database-connection')
        connections.append(connection)
        assert account_sql(container, connection, f"CREATE TABLE isolation (value varchar(30)); INSERT INTO isolation VALUES ('student{name}'); SELECT value FROM isolation;") == 'student' + name
    lecturer.request('POST', '/auth/organizations/access', {'restricted': True}, expected=204)
    for other in records[1:]:
        for statement in [f'SELECT * FROM `{other["database_name"]}`.isolation;', f'INSERT INTO `{other["database_name"]}`.isolation VALUES (\'intrusion\');']:
            account_sql(container, connections[0], statement, allowed=False)
    for statement in ["GRANT SELECT ON *.* TO CURRENT_USER;", "SELECT * FROM mysql.global_priv;", "SET GLOBAL max_connections=999;", "SELECT 'unsafe' INTO OUTFILE '/tmp/ferrix-account-file-test';"]:
        account_sql(container, connections[0], statement, allowed=False)
    print('PASS: A/B/C normal DDL/DML succeeds; A cannot read/write B/C, grant privileges, inspect root hashes, change server settings or write files.', flush=True)
    a.request('GET', f'/services/{apps[1]}/database-connection', expected=404)
    a.request('GET', f'/services/{dbid}/configuration', expected=404)
    a.request('GET', f'/services/{dbid}/databases', expected=404)
    a.request('GET', f'/services/{dbid}/backups', expected=404)
    a.websocket(dbid, expected=404)
    print('PASS: student A denied B connection settings, shared root configuration, exports and actual Docker terminal handshake.', flush=True)
    key = a.request('POST', '/auth/api-keys', {'name': 'Temporary A database key', 'scopes': ['read', 'secrets:read'], 'expires_in_days': 1, 'project_id': project['id']}, expected=201)['key']
    def key_get(path, expected):
        req = urllib.request.Request(origin + '/api/v1' + path, headers={'Authorization': 'Bearer ' + key})
        try:
            response = urllib.request.urlopen(req, timeout=20)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            assert response.code == expected, 'API key resource isolation failed'
            return json.loads(response.read())
    assert key_get(f'/services/{apps[0]}/database-connection', 200)['database'] == records[0]['database_name']
    key_get(f'/services/{apps[1]}/database-connection', 404)
    key_get(f'/services/{dbid}/configuration', 404)
    print('PASS: real project-scoped A API key reads only A limited credentials; B/root denied.', flush=True)
    ui_app = lecturer.request('POST', f'/environments/{environment}/services', {'kind': 'application', 'name': 'Student UI', 'image': 'caddy:2-alpine', 'container_port': 80, 'environment': 'UNRELATED=preserve'}, expected=201)
    def cookies(client):
        return [{'name': cookie.name, 'value': cookie.value, 'url': origin, 'httpOnly': True, 'sameSite': 'Lax'} for cookie in client.jar]
    browser_fixture = {'origin': origin, 'project': project['id'], 'server': dbid, 'target': ui_app['id'], 'studentApp': apps[0], 'managerCookies': cookies(lecturer), 'studentCookies': cookies(a)}
    if not SKIP_BROWSER:
        subprocess.run(['node', str(Path(__file__).with_name('shared-mariadb-browser.mjs'))], input=json.dumps(browser_fixture), text=True, env=live.ENV, cwd=live.ROOT, check=True, timeout=120)
    else:
        # Keep the same four-student capacity fixture without starting a browser.
        lecturer.request('POST', f'/services/{dbid}/databases', {'application_id': ui_app['id']})
        print('Browser checks skipped explicitly; API/Docker/SQL checks remain real.', flush=True)
    base = f'/services/{dbid}/databases/{records[0]["id"]}'
    def wait_job(job):
        for _ in range(180):
            result = next(item for item in lecturer.request('GET', f'/services/{dbid}/backups') if item['id'] == job['id'])
            assert result['status'] != 'failed', result['message']
            if result['status'] == 'succeeded':
                return result
            time.sleep(.5)
        raise AssertionError('Logical backup/restore worker timed out')
    backup = lecturer.request('POST', base + '/backups', {}, expected=202)
    assert wait_job(backup)['managed_database_id'] == records[0]['id']
    account_sql(container, connections[0], "UPDATE isolation SET value='changedA';")
    account_sql(container, connections[1], "UPDATE isolation SET value='preserveB';")
    lecturer.request('POST', f'/services/{dbid}/backups/{backup["id"]}/restore', {'name': 'Must not restore logical dump as a new service'}, expected=404)
    restore = lecturer.request('POST', base + f'/backups/{backup["id"]}/restore', {'database_name': records[0]['database_name']}, expected=202)
    wait_job(restore)
    assert account_sql(container, connections[0], 'SELECT value FROM isolation;') == 'studentA'
    assert account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'preserveB'
    account_sql(container, connections[1], "UPDATE isolation SET value='studentB';")
    print('PASS: existing backup worker exports/restores A in place; B data, credentials and connectivity preserved.', flush=True)
    lecturer.request('POST', base + '/rotate', {'database_name': records[0]['database_name']}, expected=204)
    account_sql(container, connections[0], 'SELECT 1;', allowed=False)
    connections[0] = a.request('GET', f'/services/{apps[0]}/database-connection')
    assert account_sql(container, connections[0], 'SELECT value FROM isolation;') == 'studentA'
    lecturer.request('POST', base + '/environment', {'database_name': records[0]['database_name']}, expected=204)
    new_environment = a.request('GET', f'/services/{apps[0]}/environment')['environment']
    assert 'UNRELATED=preserve' in new_environment
    assert dict(line.split('=', 1) for line in connections[0]['environment'].splitlines())['DB_PASSWORD'] in new_environment
    assert account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'studentB'
    print('PASS: A password rotation rejects the old password; new settings preserve unrelated environment and B connectivity.', flush=True)
    deployment = a.request('POST', f'/services/{apps[0]}/deploy', {}, expected=202)
    deployed = live.wait_deployment(a, apps[0], deployment['id'])
    applied = json.loads(live.command(['docker', 'inspect', deployed['container_id'], '--format', '{{json .Config.Env}}']))
    assert all(line in applied for line in connections[0]['environment'].splitlines())
    assert 'UNRELATED=preserve' in applied
    assert account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'studentB'
    print('PASS: A real application redeployment receives rotated limited credentials; unrelated variables and B remain intact.', flush=True)
    # Simulate a crash after SQL objects were created but before PostgreSQL completion.
    live.sql(f"UPDATE {live.SCHEMA}.managed_databases SET status='failed' WHERE id='{records[0]['id']}'")
    assert lecturer.request('POST', base + '/retry', {})['status'] == 'ready'
    assert account_sql(container, connections[0], 'SELECT value FROM isolation;') == 'studentA'
    # A foreign ownership marker must never be adopted or overwritten on retry.
    marker = root_sql(container, f"SELECT SCHEMA_COMMENT FROM information_schema.SCHEMATA WHERE SCHEMA_NAME='{records[0]['database_name']}';")
    root_sql(container, f"ALTER DATABASE `{records[0]['database_name']}` COMMENT 'externalownership';")
    live.sql(f"UPDATE {live.SCHEMA}.managed_databases SET status='failed' WHERE id='{records[0]['id']}'")
    lecturer.request('POST', base + '/retry', {}, expected=409)
    assert root_sql(container, f"SELECT SCHEMA_COMMENT FROM information_schema.SCHEMATA WHERE SCHEMA_NAME='{records[0]['database_name']}';") == 'externalownership'
    assert account_sql(container, connections[0], 'SELECT value FROM isolation;') == 'studentA'
    root_sql(container, f"ALTER DATABASE `{records[0]['database_name']}` COMMENT '{marker}';")
    lecturer.request('POST', base + '/retry', {})
    print('PASS: interrupted provisioning retries without duplicates; foreign ownership collision is rejected without altering data.', flush=True)
    root_sql(container, f"DROP USER '{records[2]['username']}'@'%';")
    live.sql(f"UPDATE {live.SCHEMA}.managed_databases SET status='failed' WHERE id='{records[2]['id']}'")
    lecturer.request('POST', f'/services/{dbid}/databases/{records[2]["id"]}/retry', {})
    assert account_sql(container, connections[2], 'SELECT value FROM isolation;') == 'studentC'
    print('PASS: partial failure with an owned database but missing SQL account retries without deleting existing tables.', flush=True)
    for number in range(5, 31):
        app = lecturer.request('POST', f'/environments/{environment}/services', {'kind': 'application', 'name': f'Student {number:02}', 'image': 'caddy:2-alpine', 'container_port': 80}, expected=201)
        assert lecturer.request('POST', f'/services/{dbid}/databases', {'application_id': app['id']})['status'] == 'ready'
    assert len(lecturer.request('GET', f'/services/{dbid}/databases')) == 30
    containers = live.command(['docker', 'ps', '-aq', '--filter', f'label=io.ferrix.application={dbid}']).split()
    assert len(containers) == 1
    assert account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'studentB'
    print('PASS: 30 managed logical databases on one MariaDB container; B data and connectivity preserved.', flush=True)
    lecturer.request('POST', f'/services/{dbid}/databases/{records[0]["id"]}/revoke', {'database_name': records[0]['database_name']}, expected=204)
    account_sql(container, connections[0], 'SELECT 1;', allowed=False)
    assert account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'studentB'
    print('PASS: revoking A SQL account denies new connections and preserves B.', flush=True)
    lecturer.request('DELETE', f'/services/{dbid}', {'name': 'Shared class MariaDB', 'remove_data': True}, expected=422)
    lecturer.request('DELETE', f'/services/{apps[2]}', {'name': 'Student C', 'remove_data': False}, expected=204)
    retained = lecturer.request('GET', f'/services/{dbid}/databases')
    assert next(r for r in retained if r['id'] == records[2]['id'])['application_id'] is None
    assert account_sql(container, connections[2], 'SELECT value FROM isolation;') == 'studentC'
    print('PASS: application removal retains C data; shared-service removal is blocked while managed databases exist.', flush=True)
    lecturer.request('DELETE', f'/services/{dbid}/databases/{records[2]["id"]}', {'database_name': 'wrong'}, expected=422)
    lecturer.request('DELETE', f'/services/{dbid}/databases/{records[2]["id"]}', {'database_name': records[2]['database_name']}, expected=204)
    account_sql(container, connections[2], 'SELECT 1;', allowed=False)
    assert account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'studentB'
    print('PASS: separate exact-name deletion removes only C SQL database/account and preserves B.', flush=True)


SKIP_BROWSER = False
live.scenario = scenario
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-browser', action='store_true', help='Run real API/Docker/SQL checks only; browser checks must be verified separately.')
    SKIP_BROWSER = parser.parse_args().skip_browser
    live.main(static_dir=live.ROOT / 'apps/web/dist')
