#!/usr/bin/env python3
"""Real shared MariaDB/S3 outage audit; invoked only by backup-s3.py --failures-only."""
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

spec = importlib.util.spec_from_file_location('shared', Path(__file__).with_name('shared-mariadb-live.py'))
shared = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shared)
live = shared.live
live.SCHEMA = 's3_failures_' + uuid.uuid4().hex


def scenario(origin):
    assert os.environ.get('FERRIX_TEST_S3_DISPOSABLE') == '1'
    endpoint = os.environ['FERRIX_TEST_S3_ENDPOINT']
    assert urllib.parse.urlsplit(endpoint).hostname == '127.0.0.1'
    assert os.environ['FERRIX_TEST_S3_BUCKET'].startswith('ferrix-test-')
    s3 = os.environ['FERRIX_TEST_S3_CONTAINER']
    identifier = os.environ['FERRIX_TEST_S3_IDENTIFIER']
    assert s3 == f'ferrix-{live.ROOT.parent.name}-test-s3-{identifier}'
    assert live.command(['docker', 'inspect', '--format', '{{ index .Config.Labels "io.ferrix.test" }}', s3]) == identifier

    def start_s3():
        live.command(['docker', 'start', s3])
        end = time.monotonic()+60
        while time.monotonic()<end:
            try:
                with urllib.request.urlopen(endpoint+'/health', timeout=1) as response:
                    if response.status==200: return
            except OSError: pass
            time.sleep(.25)
        raise AssertionError('Disposable S3 recovery timed out')

    def stop_s3():
        live.command(['docker', 'stop', '--time', '5', s3])
        try:
            urllib.request.urlopen(endpoint+'/health', timeout=1)
        except OSError: return
        raise AssertionError('S3 outage did not actually occur')

    manager, a, b = (live.Client(origin) for _ in range(3))
    password = secrets.token_urlsafe(32)
    manager.request('POST', '/auth/setup', {'name': 'Outage audit', 'email': 'manager@example.test', 'password': password})
    for client, name in [(a, 'A'), (b, 'B')]:
        invite = manager.request('POST', '/auth/invitations', {'email': name+'@example.test', 'role': 'member'})
        client.request('POST', '/auth/accept-invitation', {'name': name, 'email': name+'@example.test', 'password': password, 'token': invite['token']})
    project = manager.request('POST', '/projects', {'name': 'Disposable S3 outage audit'}, expected=201)['id']
    env = manager.request('GET', f'/projects/{project}/environments')['environments'][0]['id']
    server = manager.request('POST', f'/environments/{env}/services', {'kind': 'database', 'name': 'Outage MariaDB', 'engine': 'mariadb'}, expected=201)['id']
    deploy = manager.request('POST', f'/services/{server}/deploy', {}, expected=202)
    end = time.monotonic()+180
    while True:
        current = next(j for j in manager.request('GET', f'/services/{server}/deployments') if j['id']==deploy['id'])
        assert current['status']!='failed', 'Fixture MariaDB deployment failed'
        if current['status']=='succeeded': break
        assert time.monotonic()<end, 'Fixture deployment timeout'
        time.sleep(.5)
    container = manager.request('GET', f'/services/{server}/runtime')['container_id']
    records, connections, bases, apps = [], [], [], []
    for client, name in [(a, 'A'), (b, 'B')]:
        app = manager.request('POST', f'/environments/{env}/services', {'kind': 'application', 'name': name, 'image': 'caddy:2-alpine', 'container_port': 80}, expected=201)['id']
        manager.request('POST', f'/services/{app}/assignments', {'user_id': client.request('GET', '/auth/session')['user_id'], 'permission': 'developer'}, expected=204)
        record = manager.request('POST', f'/services/{server}/databases', {'application_id': app})
        connection = client.request('GET', f'/services/{app}/database-connection')
        shared.account_sql(container, connection, f"CREATE TABLE isolation (value varchar(30)); INSERT INTO isolation VALUES ('{name}-original');")
        apps.append(app); records.append(record); connections.append(connection); bases.append(f'/services/{server}/databases/{record["id"]}')
    manager.request('POST', '/auth/organizations/access', {'restricted': True}, expected=204)
    a.request('GET', f'/services/{apps[1]}/database-connection', expected=404)
    b.request('GET', f'/services/{apps[0]}/database-connection', expected=404)
    policy = {'enabled': False, 'cron': '0 2 * * *', 'timezone': 'UTC', 'keep_last': 10, 's3': {
        'endpoint': endpoint, 'region': 'us-east-1', 'bucket': os.environ['FERRIX_TEST_S3_BUCKET'], 'prefix': 'logical-audit', 'path_style': True,
        'credentials': {'access_key': os.environ['FERRIX_TEST_S3_ACCESS_KEY'], 'secret_key': os.environ['FERRIX_TEST_S3_SECRET_KEY'], 'session_token': None}}}
    logical = {'enabled': False, 'cron': '0 2 * * *', 'timezone': 'UTC', 'keep_last': 1}
    manager.request('POST', f'/services/{server}/backup-policy', policy, expected=204)
    for base in bases: manager.request('POST', base+'/backup-policy', logical, expected=204)
    def jobs(): return manager.request('GET', f'/services/{server}/backups')
    def wait_job(job, expected='succeeded'):
        end = time.monotonic()+180
        while time.monotonic()<end:
            current = next(j for j in jobs() if j['id']==job['id'])
            if current['status'] in ['succeeded', 'failed']:
                assert current['status']==expected, current['message']
                return current
            time.sleep(.5)
        raise AssertionError('Outage job failed to reach terminal state')
    def backup(index): return wait_job(manager.request('POST', bases[index]+'/backups', {}, expected=202))
    def data(expected_a, expected_b):
        for connection, expected in zip(connections, [expected_a, expected_b]):
            assert shared.account_sql(container, connection, 'SELECT value FROM isolation;') == expected
        assert manager.request('GET', f'/services/{server}/runtime')['container_id']==container
    def objects(present, absent=(), no_multipart=False):
        cases=[]
        for job, exists in [(j, True) for j in present]+[(j, False) for j in absent]:
            key=live.sql(f"SELECT s3_key FROM {live.SCHEMA}.database_jobs WHERE id='{uuid.UUID(job['id'])}'")
            cases.append((key, job['sha256'] if exists else None))
        verify_env=live.ENV|{'FERRIX_TEST_S3_OBJECTS':json.dumps(cases)}
        if no_multipart:
            verify_env['FERRIX_TEST_S3_NO_MULTIPART_PREFIX']='logical-audit/ferrix/'+uuid.UUID(server).hex+'/'
        subprocess.run(['cargo','test','--test','s3_live','verify_disposable_s3_objects','--','--ignored'], cwd=live.ROOT, env=verify_env, check=True)
    def remove_local():
        volume='ferrix-backups-'+uuid.UUID(server).hex
        info=json.loads(live.command(['docker','volume','inspect',volume]))[0]
        assert info['Labels'].get('io.ferrix.application')==server and info['Labels'].get('io.ferrix.backups')=='true'
        live.command(['docker','volume','rm',volume])
    try:
        first, sibling = backup(0), backup(1)
        stop_s3()
        failed = wait_job(manager.request('POST', bases[0]+'/backups', {}, expected=202), 'failed')
        assert 'S3' in failed['message'] and not failed['sha256']
        assert not next(j for j in jobs() if j['id']==first['id'])['pruned_at']
        data('A-original','B-original')
        start_s3(); objects([first,sibling])
        retried = backup(0)
        assert retried['id']!=failed['id'] and next(j for j in jobs() if j['id']==failed['id'])['status']=='failed'
        # Let the last initial A prune complete before testing a fresh retention outage.
        end=time.monotonic()+60
        while not next(j for j in jobs() if j['id']==first['id'])['pruned_at']:
            assert time.monotonic()<end
            time.sleep(.5)
        data('A-original','B-original')
        print('PASS: stopped S3 makes upload fail visibly without pruning last good backup or changing A/B; S3 restart plus a new backup request succeeds, preserving failed history.', flush=True)
        shared.account_sql(container, connections[0], "UPDATE isolation SET value='A-current';")
        shared.account_sql(container, connections[1], "UPDATE isolation SET value='B-current';")
        remove_local(); stop_s3()
        failed_restore=wait_job(manager.request('POST', bases[0]+f'/backups/{retried["id"]}/restore', {'database_name':records[0]['database_name']}, expected=202),'failed')
        assert 'S3' in failed_restore['message']
        data('A-current','B-current')
        start_s3()
        restored=wait_job(manager.request('POST', bases[0]+f'/backups/{retried["id"]}/restore', {'database_name':records[0]['database_name']}, expected=202))
        data('A-original','B-current')
        shared.account_sql(container,connections[0],f'SELECT * FROM `{records[1]["database_name"]}`.isolation;',allowed=False)
        shared.account_sql(container,connections[1],f'SELECT * FROM `{records[0]["database_name"]}`.isolation;',allowed=False)
        print('PASS: S3 download outage with local backup removed fails before SQL import; A/B remain changed and intact; restored connectivity plus new restore request recovers A only, preserving B and SQL isolation.', flush=True)
        manager.request('POST',bases[0]+'/backup-policy',logical|{'keep_last':2},expected=204)
        second=backup(0)
        stop_s3()
        manager.request('POST',bases[0]+'/backup-policy',logical,expected=204)
        # A successful local replacement triggers retention of older S3 snapshots while S3 is offline.
        manager.request('POST',f'/services/{server}/backup-policy',policy|{'s3':None},expected=204)
        local=backup(0); assert local['storage']=='local'
        end=time.monotonic()+90
        while True:
            pending=[j for j in jobs() if j['id'] in [retried['id'],second['id']]]
            if any(j['retention_error'] for j in pending): break
            assert time.monotonic()<end, 'Retention did not report S3 deletion failure'
            time.sleep(.5)
        assert all(not j['pruned_at'] for j in pending)
        assert next(j for j in jobs() if j['id']==sibling['id'])['pruned_at'] is None
        backoff=live.sql(f"SELECT bool_and(prune_retry_at>now()+interval '4 minutes') FROM {live.SCHEMA}.database_jobs WHERE application_id='{server}' AND retention_error IS NOT NULL")
        assert backoff=='t'
        data('A-original','B-current')
        start_s3(); objects([retried,second,sibling])
        # Advance only this disposable retry timer rather than waiting five wall-clock minutes.
        live.sql(f"UPDATE {live.SCHEMA}.database_jobs SET prune_retry_at=now() WHERE application_id='{server}' AND pruning")
        end=time.monotonic()+90
        while True:
            pending=[j for j in jobs() if j['id'] in [retried['id'],second['id']]]
            if all(j['pruned_at'] and not j['retention_error'] for j in pending): break
            assert time.monotonic()<end, 'Automatic retention retry failed after S3 recovery'
            time.sleep(.5)
        objects([sibling],[retried,second])
        data('A-original','B-current')
        print('PASS: retention outage records error and five-minute backoff without false pruning; after S3 recovery and advancing fixture retry timer, worker automatically retries deletion; old A objects return 404 and B checksum/data remain intact.', flush=True)
        proxy_spec=importlib.util.spec_from_file_location('transfer_proxy',Path(__file__).with_name('s3-transfer-proxy.py'))
        proxy_module=importlib.util.module_from_spec(proxy_spec);proxy_spec.loader.exec_module(proxy_module)
        proxy=proxy_module.TransferProxy(endpoint)
        try:
            shared.account_sql(container,connections[0],'CREATE TABLE payload (value varbinary(1024));')
            for _ in range(9):
                rows=','.join("(UNHEX('"+secrets.token_hex(1024)+"'))" for _ in range(1000))
                shared.account_sql(container,connections[0],'INSERT INTO payload VALUES '+rows+';')
            payload_hash=shared.account_sql(container,connections[0],"SELECT COUNT(*),BIT_XOR(CRC32(value)) FROM payload;")
            assert payload_hash.startswith('9000')
            manager.request('POST',bases[0]+'/backup-policy',logical|{'keep_last':2},expected=204)
            destination=policy['s3']|{'endpoint':proxy.endpoint}
            manager.request('POST',f'/services/{server}/backup-policy',policy|{'s3':destination},expected=204)
            proxy.fault='upload'
            interrupted=wait_job(manager.request('POST',bases[0]+'/backups',{},expected=202),'failed')
            assert proxy.cuts['upload'] and proxy.forwarded['upload']>=131072 and 1 in proxy.completed_parts
            assert 'S3' in interrupted['message']
            data('A-original','B-current')
            assert shared.account_sql(container,connections[0],"SELECT COUNT(*),BIT_XOR(CRC32(value)) FROM payload;")==payload_hash
            objects([sibling],[interrupted],no_multipart=True)
            proxy.fault=None
            uploaded=backup(0)
            assert uploaded['size_bytes']>16*1024*1024 and uploaded['storage']=='s3'
            objects([uploaded,sibling],no_multipart=True)
            print('PASS: real multipart part 1 completed; TCP cut after 128 KiB of part 2 fails upload; SDK abort leaves no object or incomplete multipart; new backup request succeeds with more than 16 MiB and matching checksum; A/B unchanged.',flush=True)
            shared.account_sql(container,connections[0],"UPDATE isolation SET value='A-after-large-backup';")
            remove_local();proxy.fault='download'
            truncated=wait_job(manager.request('POST',bases[0]+f'/backups/{uploaded["id"]}/restore',{'database_name':records[0]['database_name']},expected=202),'failed')
            assert proxy.cuts['download'] and proxy.forwarded['download']>=131072
            assert 'S3' in truncated['message']
            data('A-after-large-backup','B-current')
            proxy.fault=None
            wait_job(manager.request('POST',bases[0]+f'/backups/{uploaded["id"]}/restore',{'database_name':records[0]['database_name']},expected=202))
            data('A-original','B-current')
            assert shared.account_sql(container,connections[0],"SELECT COUNT(*),BIT_XOR(CRC32(value)) FROM payload;")==payload_hash
            shared.account_sql(container,connections[0],f'SELECT * FROM `{records[1]["database_name"]}`.isolation;',allowed=False)
            shared.account_sql(container,connections[1],f'SELECT * FROM `{records[0]["database_name"]}`.isolation;',allowed=False)
            print('PASS: TCP cut after 128 KiB of remote object download fails before SQL import; A/B and payload unchanged; retry restores full payload without local backup, preserving B and SQL isolation.',flush=True)
        finally:
            proxy.close()
    finally:
        start_s3()  # Preserve the runner-owned container until the outer runner removes it.


live.scenario=scenario
if __name__=='__main__':
    live.main()
