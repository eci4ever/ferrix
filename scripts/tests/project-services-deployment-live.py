#!/usr/bin/env python3
"""Audit project/service/deployment lifecycle against isolated real API/Postgres/Docker."""
import importlib.util
from pathlib import Path
import secrets
import time
import urllib.error
import urllib.request
import uuid

spec = importlib.util.spec_from_file_location('live', Path(__file__).with_name('resource-permissions-live.py'))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
live.SCHEMA = 'lifecycle_' + uuid.uuid4().hex


def scenario(origin):
    owner = live.Client(origin)
    owner.request('POST', '/auth/setup', {'name': 'Lifecycle audit', 'email': 'audit@example.test', 'password': secrets.token_urlsafe(32)})
    owner.request('POST', '/projects', {'name': '  '}, expected=422)
    project = owner.request('POST', '/projects', {'name': '  Disposable lifecycle  ', 'description': ' audit '}, expected=201)
    pid = project['id']
    assert project['name'] == 'Disposable lifecycle' and project['description'] == 'audit'
    environments = owner.request('GET', f'/projects/{pid}/environments')['environments']
    assert len(environments) == 1 and environments[0]['name'] == 'production'
    env = environments[0]['id']
    ids = []
    def summary(running, stopped, failed, other):
        row = next(p for p in owner.request('GET', '/projects/service-status')['projects'] if p['id'] == pid)
        assert row['available'] and [row[k] for k in ['running', 'stopped', 'failed', 'other']] == [running, stopped, failed, other], row
    def count(expected):
        assert next(p for p in owner.request('GET', '/projects') if p['id'] == pid)['service_count'] == expected
    def control(service, operation, state):
        owner.request('POST', f'/services/{service}/control', {'operation': operation}, expected=204)
        assert owner.request('GET', f'/services/{service}/runtime')['state'] == state
    def http_ready(service):
        deadline = time.monotonic() + 20
        while True:
            runtime = owner.request('GET', f'/services/{service}/runtime')
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{runtime["host_port"]}/', timeout=2) as response:
                    assert response.status == 200
                return runtime
            except (OSError, urllib.error.URLError):
                assert time.monotonic() < deadline, 'Running service did not become HTTP ready'
                time.sleep(.25)
    config = {'kind': 'application', 'name': 'Lifecycle web', 'image': 'caddy:2-alpine', 'container_port': 80, 'memory_mib': 256, 'cpus': .25, 'environment': 'AUDIT_REVISION=one'}
    try:
        owner.request('POST', f'/environments/{env}/services', config | {'container_port': 0}, expected=422)
        app = owner.request('POST', f'/environments/{env}/services', config, expected=201)
        service = app['id']; ids.append(service)
        count(1); summary(0, 0, 0, 1)
        owner.request('DELETE', f'/projects/{pid}', {'name': project['name']}, expected=409)
        owner.request('POST', f'/services/{service}/control', {'operation': 'start'}, expected=409)
        owner.request('POST', f'/services/{service}/environment', {'environment': 'A=1\nA=2'}, expected=422)
        assert owner.request('GET', f'/services/{service}/environment')['environment'] == 'AUDIT_REVISION=one'
        notes = owner.request('GET', f'/services/{service}/notes')
        saved = owner.request('POST', f'/services/{service}/notes', {'notes': 'Saved audit note', 'revision': notes['revision']})
        owner.request('POST', f'/services/{service}/notes', {'notes': 'Stale overwrite', 'revision': notes['revision']}, expected=409)
        assert owner.request('GET', f'/services/{service}/notes') == saved
        print('PASS: project validation/default production, service count/draft status, nonempty deletion guard, runtime-before-deploy guard, environment validation and notes conflict protection.', flush=True)
        staging = owner.request('POST', f'/projects/{pid}/environments', {'name': 'staging'}, expected=201)['id']
        owner.request('POST', f'/projects/{pid}/environments', {'name': 'staging'}, expected=409)
        owner.request('POST', f'/services/{service}/clone', {'environment_id': env}, expected=422)
        owner.request('POST', f'/services/{service}/move', {'environment_id': str(uuid.uuid4())}, expected=404)
        cloned = owner.request('POST', f'/services/{service}/clone', {'environment_id': staging, 'name': 'Lifecycle clone'}, expected=201)
        clone = cloned['id']; ids.append(clone)
        count(2); summary(0, 0, 0, 2)
        assert owner.request('GET', f'/services/{clone}/environment')['environment'] == config['environment']
        assert owner.request('GET', f'/services/{clone}/notes')['notes'] == saved['notes']
        assert owner.request('GET', f'/services/{clone}/deployments') == []
        moved = owner.request('POST', f'/services/{clone}/move', {'environment_id': env})
        assert moved['environment_id'] == env
        owner.request('DELETE', f'/services/{clone}', {'name': 'Lifecycle clone', 'remove_data': True}, expected=204)
        count(1)
        print('PASS: environment duplicate rejected; clone copies saved environment/notes without deployment history; move stays within project and updates environment.', flush=True)
        first = owner.request('POST', f'/services/{service}/deploy', {}, expected=202)
        runtime = live.wait_deployment(owner, service, first['id']); summary(1, 0, 0, 0)
        control(service, 'stop', 'exited'); summary(0, 1, 0, 0)
        control(service, 'stop', 'exited')
        control(service, 'start', 'running'); summary(1, 0, 0, 0)
        control(service, 'start', 'running'); control(service, 'reload', 'running')
        runtime = http_ready(service)
        assert owner.request('GET', f'/services/{service}/logs')['logs']
        print('PASS: real Docker deployment serves HTTP 200; stop/start/restart, repeated stop/start, runtime and project summary agree; real logs endpoint works.', flush=True)
        configuration = owner.request('GET', f'/services/{service}/configuration')
        owner.request('POST', f'/services/{service}/configuration', configuration | {'command': ['/bin/sh', '-c', 'exit 7']})
        failed = owner.request('POST', f'/services/{service}/deploy', {}, expected=202)
        deadline = time.monotonic() + 90
        while True:
            history = owner.request('GET', f'/services/{service}/deployments')
            job = next(d for d in history if d['id'] == failed['id'])
            if job['status'] == 'failed':
                assert job['logs']
                break
            assert job['status'] != 'succeeded' and time.monotonic() < deadline, 'Failed command should fail deployment'
            time.sleep(.5)
        assert owner.request('GET', f'/services/{service}/runtime')['container_id'] == runtime['container_id']
        summary(1, 0, 0, 0)
        runtime = http_ready(service)
        owner.request('POST', f'/services/{service}/rollback', {'deployment_id': failed['id']}, expected=422)
        rollback = owner.request('POST', f'/services/{service}/rollback', {'deployment_id': first['id']}, expected=202)
        live.wait_deployment(owner, service, rollback['id']); summary(1, 0, 0, 0)
        owner.request('POST', f'/services/{service}/configuration', configuration | {'environment': 'AUDIT_REVISION=two'})
        redeploy = owner.request('POST', f'/services/{service}/deploy', {}, expected=202)
        live.wait_deployment(owner, service, redeploy['id'])
        assert len(owner.request('GET', f'/services/{service}/deployments')) == 4
        print('PASS: failed real deployment retains previous HTTP service/running summary; failed rollback target rejected; pinned successful rollback and subsequent redeploy succeed.', flush=True)
        owner.request('DELETE', f'/services/{service}', {'name': 'wrong'}, expected=422)
        owner.request('DELETE', f'/services/{service}', {'name': config['name'], 'remove_data': True}, expected=204)
        owner.request('GET', f'/services/{service}', expected=404)
        assert not live.command(['docker', 'ps', '-aq', '--filter', f'label=io.ferrix.application={service}'])
        count(0); summary(0, 0, 0, 0)
        owner.request('DELETE', f'/projects/{pid}', {'name': 'wrong'}, expected=422)
        owner.request('DELETE', f'/projects/{pid}', {'name': project['name']}, expected=204)
        assert not owner.request('GET', '/projects')
        owner.request('GET', f'/projects/{pid}/environments', expected=404)
        print('PASS: exact-name confirmation required; service containers removed and count updated; empty project deletion succeeds.', flush=True)
    finally:
        # Explicit IDs survive API deletion, so cleanup also covers deleted metadata.
        for service in ids:
            containers = live.command(['docker', 'ps', '-aq', '--filter', f'label=io.ferrix.application={service}']).split()
            if containers:
                live.command(['docker', 'rm', '-f', *containers])
        networks = live.command(['docker', 'network', 'ls', '-q', '--filter', f'label=io.ferrix.environment={env}']).split()
        if networks:
            live.command(['docker', 'network', 'rm', *networks])


live.scenario = scenario
if __name__ == '__main__':
    live.main()
