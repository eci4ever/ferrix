#!/usr/bin/env python3
"""Real HTTP/PostgreSQL/MariaDB backup roundtrips in a disposable schema and containers."""
import importlib.util
from pathlib import Path
import json
import secrets
import subprocess
import time
import uuid

spec = importlib.util.spec_from_file_location('live', Path(__file__).with_name('resource-permissions-live.py'))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
live.SCHEMA = 'database_roundtrip_' + uuid.uuid4().hex


def database_sql(container, engine, statement):
    if engine == 'postgres':
        args = ['psql', '-U', 'ferrix', '-d', 'ferrix', '-v', 'ON_ERROR_STOP=1', '-At']
    else:
        args = ['/bin/sh', '-c', 'MYSQL_PWD="$MARIADB_PASSWORD" exec mariadb --protocol=tcp --host=127.0.0.1 --user="$MARIADB_USER" "$MARIADB_DATABASE" --batch --skip-column-names']
    result = subprocess.run(['docker', 'exec', '-i', container, *args], input=statement+'\n', text=True, capture_output=True, env=live.ENV, timeout=20)
    assert result.returncode == 0, 'Disposable database SQL failed'
    return result.stdout.strip()


def scenario(origin):
    manager, student = live.Client(origin), live.Client(origin)
    password = secrets.token_urlsafe(32)
    manager.request('POST', '/auth/setup', {'name': 'Backup audit', 'email': 'audit@example.test', 'password': password})
    invite = manager.request('POST', '/auth/invitations', {'email': 'student@example.test', 'role': 'member'})
    student.request('POST', '/auth/accept-invitation', {'name': 'Student', 'email': 'student@example.test', 'password': password, 'token': invite['token']})
    project = manager.request('POST', '/projects', {'name': 'Disposable database roundtrip'}, expected=201)['id']
    environment = manager.request('GET', f'/projects/{project}/environments')['environments'][0]['id']
    def wait_deploy(service, job):
        deadline = time.monotonic()+180
        while time.monotonic()<deadline:
            current = next(j for j in manager.request('GET', f'/services/{service}/deployments') if j['id']==job['id'])
            assert current['status'] != 'failed', 'Database deployment failed'
            if current['status']=='succeeded':
                return manager.request('GET', f'/services/{service}/runtime')['container_id']
            time.sleep(.5)
        raise AssertionError('Database deployment timed out')
    def wait_job(service, job, expected='succeeded'):
        deadline = time.monotonic()+120
        while time.monotonic()<deadline:
            current = next(j for j in manager.request('GET', f'/services/{service}/backups') if j['id']==job['id'])
            if current['status'] in ['failed', 'succeeded']:
                assert current['status']==expected, 'Backup/restore terminal state did not match expected result'
                return current
            time.sleep(.5)
        raise AssertionError('Backup/restore timed out')
    for engine in ['postgres', 'mariadb']:
        service = manager.request('POST', f'/environments/{environment}/services', {'kind': 'database', 'name': engine+' audit', 'engine': engine}, expected=201)['id']
        base = f'/services/{service}/backups'
        manager.request('POST', base, {}, expected=422)
        container = wait_deploy(service, manager.request('POST', f'/services/{service}/deploy', {}, expected=202))
        assert not json.loads(live.command(['docker', 'inspect', container, '--format', '{{json .HostConfig.PortBindings}}']))
        config = manager.request('GET', f'/services/{service}/configuration')
        database_sql(container, engine, "CREATE TABLE audit_data (value varchar(30)); INSERT INTO audit_data VALUES ('before');")
        assert database_sql(container, engine, 'SELECT value FROM audit_data;') == 'before'
        student.request('POST', base, {}, expected=403)
        backup = manager.request('POST', base, {}, expected=202)
        manifest = wait_job(service, backup)
        assert manifest['size_bytes']>0 and len(manifest['sha256'])==64
        database_sql(container, engine, "INSERT INTO audit_data VALUES ('after');")
        student.request('POST', base+f'/{backup["id"]}/restore', {'name': 'Forbidden restore'}, expected=403)
        manager.request('POST', base+f'/{backup["id"]}/restore', {'name': ''}, expected=422)
        restore = manager.request('POST', base+f'/{backup["id"]}/restore', {'name': engine+' restored'}, expected=202)
        target = restore['application_id']; assert target != service
        wait_job(service, restore)
        settings = manager.request('GET', f'/services/{target}/configuration')
        assert settings['password'] != config['password']
        restored = manager.request('GET', f'/services/{target}/runtime')
        assert restored['state']=='running'
        assert database_sql(restored['container_id'], engine, 'SELECT value FROM audit_data;')=='before'
        assert database_sql(container, engine, 'SELECT value FROM audit_data ORDER BY value;')=='after\nbefore'
        manager.request('POST', f'/services/{target}/backups/{backup["id"]}/restore', {'name': 'Wrong source'}, expected=404)
        # Corrupt metadata only inside this disposable schema: the real worker must reject integrity mismatch.
        original = manifest['sha256']
        live.sql(f"UPDATE {live.SCHEMA}.database_jobs SET sha256=repeat('0',64) WHERE id='{backup['id']}'")
        corrupt = manager.request('POST', base+f'/{backup["id"]}/restore', {'name': engine+' corrupt rejected'}, expected=202)
        failed = wait_job(service, corrupt, expected='failed')
        assert 'checksum' in failed['message'].lower() or 'integrity' in failed['message'].lower(), failed['message']
        assert database_sql(container, engine, 'SELECT value FROM audit_data ORDER BY value;')=='after\nbefore'
        assert database_sql(restored['container_id'], engine, 'SELECT value FROM audit_data;')=='before'
        live.sql(f"UPDATE {live.SCHEMA}.database_jobs SET sha256='{original}' WHERE id='{backup['id']}'")
        print(f'PASS: {engine} private Docker database; backup manifest; restored snapshot into separate running service with new password; source unchanged; student/cross-service restore denied; corrupt backup rejected without altering valid source/restore.', flush=True)


live.scenario = scenario
if __name__ == '__main__':
    live.main()
