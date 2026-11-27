#!/usr/bin/env python3
"""Real scheduled MariaDB backups/restore in disposable mac2-owned resources."""
import argparse
import os
import json
import subprocess
import urllib.parse
import importlib.util
from pathlib import Path
import secrets
import time
import uuid

spec = importlib.util.spec_from_file_location('shared', Path(__file__).with_name('shared-mariadb-live.py'))
shared = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shared)
live = shared.live
live.SCHEMA = 'logical_schedules_' + uuid.uuid4().hex


def scenario(origin):
    owner, manager, student = (live.Client(origin) for _ in range(3))
    password = secrets.token_urlsafe(32)
    owner.request('POST', '/auth/setup', {'name': 'Platform', 'email': 'platform@example.test', 'password': password})
    for client, name, role in [(manager, 'lecturer', 'admin'), (student, 'student', 'member')]:
        invite = owner.request('POST', '/auth/invitations', {'email': name+'@example.test', 'role': role})
        client.request('POST', '/auth/accept-invitation', {'name': name, 'email': name+'@example.test', 'password': password, 'token': invite['token']})
    project = manager.request('POST', '/projects', {'name': 'Temporary scheduled backup test'}, expected=201)['id']
    environment = manager.request('GET', f'/projects/{project}/environments')['environments'][0]['id']
    server = manager.request('POST', f'/environments/{environment}/services', {'kind': 'database', 'name': 'Shared fixture', 'engine': 'mariadb'}, expected=201)['id']
    deployment = manager.request('POST', f'/services/{server}/deploy', {}, expected=202)
    for _ in range(240):
        deployed = next(j for j in manager.request('GET', f'/services/{server}/deployments') if j['id'] == deployment['id'])
        assert deployed['status'] != 'failed', deployed['logs']
        if deployed['status'] == 'succeeded':
            break
        time.sleep(.5)
    else:
        raise AssertionError('MariaDB deployment timed out')
    container = manager.request('GET', f'/services/{server}/runtime')['container_id']
    records, connections, bases = [], [], []
    for name in ['A', 'B']:
        app = manager.request('POST', f'/environments/{environment}/services', {'kind': 'application', 'name': name, 'image': 'caddy:2-alpine', 'container_port': 80}, expected=201)['id']
        record = manager.request('POST', f'/services/{server}/databases', {'application_id': app})
        manager.request('POST', f'/services/{app}/assignments', {'user_id': student.request('GET', '/auth/session')['user_id'], 'permission': 'developer'}, expected=204)
        connection = student.request('GET', f'/services/{app}/database-connection')
        shared.account_sql(container, connection, f"CREATE TABLE isolation (value varchar(30)); INSERT INTO isolation VALUES ('student{name}');")
        records.append(record); connections.append(connection); bases.append(f'/services/{server}/databases/{record["id"]}')
    remote_policy = None
    if REMOTE:
        assert os.environ.get('FERRIX_TEST_S3_DISPOSABLE') == '1'
        endpoint = os.environ['FERRIX_TEST_S3_ENDPOINT']
        assert urllib.parse.urlsplit(endpoint).hostname == '127.0.0.1'
        assert os.environ['FERRIX_TEST_S3_BUCKET'].startswith('ferrix-test-')
        remote_policy = {'enabled': False, 'cron': '0 2 * * *', 'timezone': 'UTC', 'keep_last': 10, 's3': {
            'endpoint': endpoint, 'region': 'us-east-1', 'bucket': os.environ['FERRIX_TEST_S3_BUCKET'], 'prefix': 'logical-audit',
            'path_style': True, 'credentials': {'access_key': os.environ['FERRIX_TEST_S3_ACCESS_KEY'],
                'secret_key': os.environ['FERRIX_TEST_S3_SECRET_KEY'], 'session_token': None}}}
        manager.request('POST', f'/services/{server}/backup-policy', remote_policy, expected=204)
        manager.request('POST', f'/services/{server}/backup-policy/test', {})
    manager.request('POST', '/auth/organizations/access', {'restricted': True}, expected=204)
    policy = {'enabled': True, 'cron': '* * * * *', 'timezone': 'Asia/Kuala_Lumpur', 'keep_last': 2}
    for base in bases:
        assert manager.request('GET', base+'/backup-policy')['enabled'] is False
        student.request('GET', base+'/backup-policy', expected=404)
        student.request('POST', base+'/backup-policy', policy, expected=404)
        for invalid in [dict(policy, cron='bad'), dict(policy, timezone='bad-zone'), dict(policy, keep_last=0)]:
            manager.request('POST', base+'/backup-policy', invalid, expected=422)
        manager.request('POST', base+'/backup-policy', policy, expected=204)
        saved = manager.request('GET', base+'/backup-policy')
        assert saved['next_run_at'] and saved['timezone'] == policy['timezone']
    manager.request('GET', f'/services/{server}/databases/{uuid.uuid4()}/backup-policy', expected=404)
    print('PASS: non-platform lecturer saves independent cron/timezone/retention; students denied; invalid settings rejected.', flush=True)

    def jobs():
        return manager.request('GET', f'/services/{server}/backups')

    def wait(predicate, message, seconds=150):
        end = time.monotonic()+seconds
        while time.monotonic() < end:
            current = jobs()
            assert not any(j['status'] == 'failed' for j in current), str([(j['status'], j['message']) for j in current])
            value = predicate(current)
            if value:
                return value
            time.sleep(.5)
        raise AssertionError(message)

    def remote_objects(present, absent=()):
        cases = []
        for job, exists in [(j, True) for j in present] + [(j, False) for j in absent]:
            assert job['storage'] == 's3'
            key = live.sql(f"SELECT s3_key FROM {live.SCHEMA}.database_jobs WHERE id='{uuid.UUID(job['id'])}'")
            assert key.startswith('logical-audit/ferrix/')
            cases.append((key, job['sha256'] if exists else None))
        env = live.ENV | {'FERRIX_TEST_S3_OBJECTS': json.dumps(cases)}
        subprocess.run(['cargo', 'test', '--test', 's3_live', 'verify_disposable_s3_objects', '--', '--ignored', '--nocapture'], cwd=live.ROOT, env=env, check=True)

    # Wait for the real cron minute, without forcing timestamps or queueing jobs ourselves.
    scheduled = wait(lambda current: [j for j in current if j['scheduled_for'] and j['status'] == 'succeeded'] if len([j for j in current if j['scheduled_for'] and j['status'] == 'succeeded']) >= 2 else None, 'Both same-minute schedules must run')
    assert {j['managed_database_id'] for j in scheduled} == {r['id'] for r in records}
    assert len({(j['managed_database_id'], j['scheduled_for']) for j in scheduled}) == len(scheduled)
    for base in bases:
        manager.request('POST', base+'/backup-policy', dict(policy, enabled=False), expected=204)
    print('PASS: real cron automatically backed up A and B on the same service, sequentially, without duplicate jobs.', flush=True)
    first = next(j for j in scheduled if j['managed_database_id'] == records[0]['id'])
    manager.request('POST', bases[0]+f'/backups/{first["id"]}/restore', {'database_name': 'wrong'}, expected=422)
    manager.request('POST', bases[1]+f'/backups/{first["id"]}/restore', {'database_name': records[1]['database_name']}, expected=404)
    student.request('POST', bases[0]+f'/backups/{first["id"]}/restore', {'database_name': records[0]['database_name']}, expected=404)
    print('PASS: wrong confirmation, backup ID substituted across A/B and student restore requests are rejected before import.', flush=True)
    if REMOTE:
        remote_objects(scheduled)
        volume = 'ferrix-backups-' + uuid.UUID(server).hex
        info = json.loads(live.command(['docker', 'volume', 'inspect', volume]))[0]
        assert info['Labels'].get('io.ferrix.application') == server and info['Labels'].get('io.ferrix.backups') == 'true'
        live.command(['docker', 'volume', 'rm', volume])
        manager.request('POST', f'/services/{server}/backup-policy', remote_policy | {'s3': None}, expected=204)
        print('PASS: A/B S3 objects downloaded and checksums verified; fixture local backup volume removed; current S3 destination cleared before A restore.', flush=True)
    shared.account_sql(container, connections[0], "UPDATE isolation SET value='changedA';")
    shared.account_sql(container, connections[1], "UPDATE isolation SET value='preserveB';")
    restore = manager.request('POST', bases[0]+f'/backups/{first["id"]}/restore', {'database_name': records[0]['database_name']}, expected=202)
    wait(lambda current: next((j for j in current if j['id']==restore['id'] and j['status']=='succeeded'), None), 'Restore timed out')
    assert shared.account_sql(container, connections[0], 'SELECT value FROM isolation;') == 'studentA'
    assert shared.account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'preserveB'
    shared.account_sql(container, connections[0], f'SELECT * FROM `{records[1]["database_name"]}`.isolation;', allowed=False)
    shared.account_sql(container, connections[1], f'SELECT * FROM `{records[0]["database_name"]}`.isolation;', allowed=False)
    print('PASS: restore of scheduled A backup restores A data; B changed data/account/connectivity preserved; A/B SQL isolation intact.', flush=True)
    if REMOTE:
        manager.request('POST', f'/services/{server}/backup-policy', remote_policy, expected=204)
    # Independent retention also covers manual backups; preserve B history.
    manager.request('POST', bases[0]+'/backup-policy', dict(policy, enabled=False, keep_last=1), expected=204)
    b_ids = {j['id'] for j in jobs() if j['managed_database_id'] == records[1]['id']}
    manual = manager.request('POST', bases[0]+'/backups', {}, expected=202)
    wait(lambda current: next((j for j in current if j['id']==manual['id'] and j['status']=='succeeded'), None), 'Manual backup failed')
    wait(lambda current: next((j for j in current if j['id']==first['id'] and j['pruned_at']), None), 'Logical retention did not prune A')
    assert all(not j['pruned_at'] for j in jobs() if j['id'] in b_ids)
    if REMOTE:
        retained = next(j for j in jobs() if j['id'] == manual['id'])
        remote_objects([retained] + [j for j in scheduled if j['managed_database_id'] == records[1]['id']], [first])
        manager.request('POST', bases[0]+f'/backups/{first["id"]}/restore', {'database_name': records[0]['database_name']}, expected=404)
        # Prove B's object is usable after A retention and all local backup files are removed again.
        volume = 'ferrix-backups-' + uuid.UUID(server).hex
        info = json.loads(live.command(['docker', 'volume', 'inspect', volume]))[0]
        assert info['Labels'].get('io.ferrix.application') == server and info['Labels'].get('io.ferrix.backups') == 'true'
        live.command(['docker', 'volume', 'rm', volume])
        b_backup = next(j for j in scheduled if j['managed_database_id'] == records[1]['id'])
        a_before = shared.account_sql(container, connections[0], 'SELECT value FROM isolation;')
        b_restore = manager.request('POST', bases[1]+f'/backups/{b_backup["id"]}/restore', {'database_name': records[1]['database_name']}, expected=202)
        wait(lambda current: next((j for j in current if j['id'] == b_restore['id'] and j['status']=='succeeded'), None), 'B S3 restore timed out')
        assert shared.account_sql(container, connections[1], 'SELECT value FROM isolation;') == 'studentB'
        assert shared.account_sql(container, connections[0], 'SELECT value FROM isolation;') == a_before
        print('PASS: A retention deletes only A old S3 object (404); B and new A objects retain matching checksums; B remote restore succeeds without local files and preserves A.', flush=True)
    # A overdue schedule owned by a demoted manager must pause rather than run.
    manager.request('POST', bases[0]+'/backup-policy', policy, expected=204)
    uid = manager.request('GET', '/auth/session')['user_id']
    live.sql(f"UPDATE {live.SCHEMA}.auth_members SET role='member' WHERE user_id='{uid}'; UPDATE {live.SCHEMA}.managed_backup_policies SET next_run_at=now()-interval '1 hour' WHERE managed_database_id='{records[0]['id']}';")
    end=time.monotonic()+20
    while time.monotonic()<end:
        paused=owner.request('GET', bases[0]+'/backup-policy')
        if not paused['enabled']: break
        time.sleep(.5)
    assert not paused['enabled'] and 'permission' in paused['last_message']
    print('PASS: A retention prunes A only; B history retained; demotion pauses overdue schedule.', flush=True)


REMOTE = False
live.scenario = scenario
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--s3', action='store_true', help='Use only the disposable local S3 runner environment.')
    REMOTE = parser.parse_args().s3
    live.main()
