#!/usr/bin/env python3
"""Real PHP template deployment, MariaDB, source updates and student boundaries."""
import importlib.util
import io
import json
from pathlib import Path
import secrets
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile

spec = importlib.util.spec_from_file_location('live', Path(__file__).with_name('resource-permissions-live.py'))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
live.SCHEMA = 'php_live_' + uuid.uuid4().hex
spec = importlib.util.spec_from_file_location('maria', Path(__file__).with_name('shared-mariadb-live.py'))
maria = importlib.util.module_from_spec(spec)
spec.loader.exec_module(maria)
maria.live = live
TEMPLATE = '.ferrix/php-v1/Dockerfile'


def php(container, body):
    result = subprocess.run(['docker', 'exec', '-i', container, 'php'], input='<?php\n' + body, text=True, capture_output=True, env=live.ENV, timeout=20)
    assert result.returncode == 0, 'PHP fixture command failed: ' + result.stderr[:200]
    return result.stdout.strip()


CONNECT = "$db=new PDO('mysql:host='.getenv('DB_HOST').';port='.getenv('DB_PORT').';dbname='.getenv('DB_DATABASE'),getenv('DB_USERNAME'),getenv('DB_PASSWORD'),[PDO::ATTR_ERRMODE=>PDO::ERRMODE_EXCEPTION]);"


def wait(client, service, job, expected='succeeded'):
    deadline = time.monotonic() + 360
    while time.monotonic() < deadline:
        current = next(item for item in client.request('GET', f'/services/{service}/deployments') if item['id'] == job['id'])
        if current['status'] in ['succeeded', 'failed']:
            assert current['status'] == expected, current['logs'][-3000:]
            return client.request('GET', f'/services/{service}/runtime')
        time.sleep(.5)
    raise AssertionError('PHP deployment worker timed out')


def web(runtime, path='/', expected=200):
    url = f'http://127.0.0.1:{runtime["host_port"]}{path}'
    try:
        response = urllib.request.urlopen(url, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        assert response.status in (expected if isinstance(expected, tuple) else (expected,)), f'Unexpected HTTP {response.status} for {path}'
        return response.read().decode()


def upload(client, app, entries, dockerfile=TEMPLATE, expected=200):
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as output:
        for name, content in entries.items():
            output.writestr(name, content)
    url = client.origin + f'/api/v1/services/{app}/source/upload?' + urllib.parse.urlencode({'context': '.', 'dockerfile': dockerfile})
    request = urllib.request.Request(url, method='POST', data=archive.getvalue(), headers={'Origin': client.origin, 'Content-Type': 'application/octet-stream'})
    try:
        response = client.opener.open(request, timeout=40)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        assert response.status == expected, f'Upload returned {response.status}'
        result = json.loads(response.read())
    if expected == 200:
        client.request('POST', f'/services/{app}/source', {'mode': 'upload', 'upload_id': result['upload_id']}, expected=204)
    return result


def scenario(origin):
    owner, lecturer, a, b, c = (live.Client(origin) for _ in range(5))
    password = secrets.token_urlsafe(32)
    owner.request('POST', '/auth/setup', {'name': 'Fixture platform', 'email': 'platform@example.test', 'password': password})
    for client, name, role in [(lecturer, 'lecturer', 'admin'), (a, 'student-a', 'member'), (b, 'student-b', 'member'), (c, 'student-c', 'member')]:
        invitation = owner.request('POST', '/auth/invitations', {'email': name + '@example.test', 'role': role})
        client.request('POST', '/auth/accept-invitation', {'name': name, 'email': name + '@example.test', 'password': password, 'token': invitation['token']})
    assert lecturer.request('GET', '/auth/session')['system_role'] == 'user'
    project = lecturer.request('POST', '/projects', {'name': 'Disposable PHP classroom'}, expected=201)
    environment = lecturer.request('GET', f'/projects/{project["id"]}/environments')['environments'][0]['id']
    server = lecturer.request('POST', f'/environments/{environment}/services', {'kind': 'database', 'name': 'Shared PHP class MariaDB', 'engine': 'mariadb'}, expected=201)['id']
    wait(lecturer, server, lecturer.request('POST', f'/services/{server}/deploy', {}, expected=202))
    lecturer.request('POST', '/auth/organizations/access', {'restricted': True}, expected=204)
    catalog = lecturer.request('GET', '/templates/php-v1')
    assert catalog['runtime'] == 'php:8.5-apache-bookworm'
    assert catalog['document_root'] == 'public/' and set(catalog['extensions']) == {'mysqli', 'pdo_mysql'}
    a.request('GET', '/templates/php-v1', expected=403)
    records, runtimes, connections = [], [], []
    for name, client in [('A', a), ('B', b), ('C', c)]:
        student = client.request('GET', '/auth/session')['user_id']
        payload = {'request_id': str(uuid.uuid4()), 'name': 'PHP student ' + name, 'student_id': student, 'database_service_id': server, 'memory_mib': 256, 'cpus': .5}
        record = lecturer.request('POST', f'/environments/{environment}/php-applications', payload)
        app = record['application_id']
        assert lecturer.request('POST', f'/environments/{environment}/php-applications', payload)['application_id'] == app
        client.request('POST', f'/services/{app}/deploy', {}, expected=422)
        a.request('POST', f'/environments/{environment}/php-applications', payload, expected=403)
        if name == 'A':
            lecturer.request('POST', f'/services/{server}/control', {'operation': 'stop'}, expected=204)
            lecturer.request('POST', f'/services/{app}/php-template/provision', {}, expected=503)
            assert lecturer.request('GET', f'/services/{app}/php-template')['status'] == 'failed'
            lecturer.request('POST', f'/services/{server}/control', {'operation': 'start'}, expected=204)
            for _ in range(60):
                if lecturer.request('GET', f'/services/{server}/runtime').get('health') == 'healthy':
                    break
                time.sleep(.5)
            assert lecturer.request('POST', f'/environments/{environment}/php-applications', payload)['application_id'] == app
            lecturer.request('POST', f'/environments/{environment}/php-applications', dict(payload, memory_mib=512), expected=422)
        lecturer.request('POST', f'/services/{app}/php-template/provision', {})
        connection = client.request('GET', f'/services/{app}/database-connection')
        records.append(record); connections.append(connection)
        runtime = wait(client, app, client.request('POST', f'/services/{app}/deploy', {}, expected=202))
        runtimes.append(runtime)
        assert 'Database connected' in web(runtime)
        assert 'font-family' in web(runtime, '/style.css')
        assert php(runtime['container_id'], "echo PHP_MAJOR_VERSION.'.'.PHP_MINOR_VERSION.' '.(int)extension_loaded('mysqli').' '.(int)extension_loaded('pdo_mysql');") == '8.5 1 1'
        assert php(runtime['container_id'], CONNECT + "echo $db->query('SELECT DATABASE()')->fetchColumn();") == connection['database']
        inspect = json.loads(live.command(['docker', 'inspect', runtime['container_id'], '--format', '{{json .HostConfig}}']))
        assert inspect['Memory'] == 256 * 1024 * 1024 and inspect['NanoCpus'] == 500_000_000
        container_env = json.loads(live.command(['docker', 'inspect', runtime['container_id'], '--format', '{{json .Config.Env}}']))
        assert not any(line.startswith(('MARIADB_ROOT_PASSWORD=', 'MYSQL_ROOT_PASSWORD=')) for line in container_env)
        for path in ['/.env', '/.git/config', '/.ferrix/php-v1/Dockerfile', '/composer.json']:
            web(runtime, path, expected=(403, 404))
        for line in connection['environment'].splitlines():
            if line.startswith('DB_PASSWORD='):
                secret = line.split('=', 1)[1]
                assert secret not in web(runtime)
                assert secret not in json.dumps(catalog)
                assert secret not in json.dumps(client.request('GET', f'/services/{app}/deployments'))
        print(f'PASS: ordinary lecturer provisioned and deployed PHP student {name}; PHP 8.5 + mysqli/PDO connects to own MariaDB with enforced CPU/RAM.', flush=True)
    apps = [record['application_id'] for record in records]
    for query in [f"SELECT * FROM `{connections[1]['database']}`.ferrix_visits", f"INSERT INTO `{connections[1]['database']}`.ferrix_visits VALUES(99,99)", 'SELECT * FROM mysql.global_priv', 'SET GLOBAL max_connections=999']:
        result = php(runtimes[0]['container_id'], CONNECT + "try { $db->exec(" + json.dumps(query) + "); echo 'UNEXPECTED'; } catch (PDOException $e) { echo 'DENIED'; }")
        assert result == 'DENIED', 'A PHP account crossed database privilege boundary'
    for suffix in ['', '/source', '/configuration', '/database-connection', '/php-template', '/backups']:
        a.request('GET', f'/services/{apps[1]}{suffix}', expected=404)
    a.request('POST', f'/services/{apps[1]}/deploy', {}, expected=404)
    a.request('POST', f'/services/{apps[0]}/php-template/provision', {}, expected=403)
    a.request('GET', f'/services/{server}/configuration', expected=404)
    a.websocket(server, expected=404)
    config = a.request('GET', f'/services/{apps[0]}/configuration')
    a.request('POST', f'/services/{apps[0]}/configuration', dict(config, memory_mib=512), expected=403)
    a.request('POST', f'/applications/{apps[0]}/configuration', dict(config, cpus=1), expected=403)
    print('PASS: SQL executed inside actual PHP A container cannot read/write B or administer MariaDB; A cannot access B API/root terminal or raise limits by alternate route.', flush=True)
    terminal = a.websocket(apps[0])
    try:
        terminal.until(lambda: any(event.get('type') == 'ready' for event in terminal.events))
        token = uuid.uuid4().hex
        terminal.send({'type': 'input', 'data': f"printf '%s%s\\n' PHP_TERMINAL_ {token}; php -r 'echo PHP_MAJOR_VERSION, PHP_MINOR_VERSION;'\n"})
        terminal.until(lambda: 'PHP_TERMINAL_' + token in terminal.output)
    finally:
        terminal.close()
    a.websocket(apps[1], expected=404)
    php(runtimes[0]['container_id'], CONNECT + "$db->exec('CREATE TABLE retain_data(value varchar(20))'); $db->exec(\"INSERT INTO retain_data VALUES ('preserved')\"); file_put_contents('/opt/data/uploads/retain.txt','persistent-upload');")
    entries = {'public/index.php': '<?php echo "Updated student A";', 'public/style.css': 'body{font-family:system-ui}', 'Dockerfile': 'FROM original/user-dockerfile', '.env': 'fixture-secret-never-build', '.git/config': 'fixture-git-never-build'}
    preview = upload(a, apps[0], entries)
    assert 'Dockerfile' in preview['preview']['files'] and TEMPLATE in preview['preview']['files']
    assert '.env' not in preview['preview']['files'] and '.git/config' not in preview['preview']['files']
    runtime = wait(a, apps[0], a.request('POST', f'/services/{apps[0]}/deploy', {}, expected=202))
    assert web(runtime) == 'Updated student A'
    assert php(runtime['container_id'], CONNECT + "echo $db->query('SELECT value FROM retain_data')->fetchColumn().' '.file_get_contents('/opt/data/uploads/retain.txt');") == 'preserved persistent-upload'
    assert a.request('GET', f'/services/{apps[0]}/database-connection') == connections[0]
    inspect = json.loads(live.command(['docker', 'inspect', runtime['container_id'], '--format', '{{json .HostConfig}}']))
    assert inspect['Memory'] == 256 * 1024 * 1024 and inspect['NanoCpus'] == 500_000_000
    assert 'original/user-dockerfile' in php(runtime['container_id'], "echo file_get_contents('/var/www/app/Dockerfile');")
    assert php(runtime['container_id'], "echo (int)file_exists('/var/www/app/.env').' '.(int)file_exists('/var/www/app/.git/config');") == '0 0'
    assert 'Database connected' in web(runtimes[1])
    print('PASS: student A ZIP update/redeploy keeps database identity, rows, persistent upload and limits; original Dockerfile retained and secrets/Git metadata excluded.', flush=True)
    upload(a, apps[0], {'index.php': '<?php echo 1;'}, expected=422)
    upload(a, apps[0], {'public/index.php': '<?php echo 1;', TEMPLATE: 'FROM changed/image'}, expected=422)
    upload(a, apps[0], {'Dockerfile': 'FROM scratch\nRUN exit 1\n'}, dockerfile='Dockerfile')
    wait(a, apps[0], a.request('POST', f'/services/{apps[0]}/deploy', {}, expected=202), expected='failed')
    assert php(runtime['container_id'], CONNECT + "echo $db->query('SELECT value FROM retain_data')->fetchColumn();") == 'preserved'
    upload(a, apps[0], entries)
    wait(a, apps[0], a.request('POST', f'/services/{apps[0]}/deploy', {}, expected=202))
    assert a.request('GET', f'/services/{apps[0]}/database-connection') == connections[0]
    print('PASS: invalid source/reserved-file conflicts are rejected; failed Docker build and retry retain application/database data.', flush=True)
    assert len(lecturer.request('GET', f'/services/{server}/databases')) == 3
    # Small, reviewed PHP fixture: no Composer/dependency download at runtime.
    # Pin source so this test cannot silently execute a changed upstream branch.
    git_config = {'mode': 'github', 'repository': 'yingkitw/phprs', 'branch': '5f3f74a7c37dda631b4a1197e56b7f86e45a4c68', 'context': 'examples/codeigniter', 'dockerfile': TEMPLATE, 'auto_deploy': False}
    c.request('POST', f'/services/{apps[2]}/source', git_config, expected=204)
    git_runtime = wait(c, apps[2], c.request('POST', f'/services/{apps[2]}/deploy', {}, expected=202))
    assert 'Hello from HomeController::index()' in web(git_runtime)
    assert php(git_runtime['container_id'], CONNECT + "echo $db->query('SELECT DATABASE()')->fetchColumn();") == connections[2]['database']
    assert c.request('GET', f'/services/{apps[2]}/database-connection') == connections[2]
    print('PASS: real GitHub commit download/build/deploy through existing Git engine works with PHP template and preserves C database identity.', flush=True)
    def cookies(client):
        return [{'name': cookie.name, 'value': cookie.value, 'url': origin, 'httpOnly': True, 'sameSite': 'Lax'} for cookie in client.jar]
    fixture = {'origin': origin, 'project': project['id'], 'environment': environment, 'student': a.request('GET', '/auth/session')['user_id'], 'server': server, 'application': apps[0], 'managerCookies': cookies(lecturer), 'studentCookies': cookies(a), 'starterUrl': f'http://127.0.0.1:{runtimes[1]["host_port"]}/'}
    subprocess.run(['node', str(Path(__file__).with_name('php-template-browser.mjs'))], input=json.dumps(fixture), text=True, env=live.ENV, cwd=live.ROOT, check=True, timeout=180)
    browser_app = next(app for app in lecturer.request('GET', f'/environments/{environment}/services') if app['name'] == 'PHP browser fixture')['id']
    browser_job = lecturer.request('GET', f'/services/{browser_app}/deployments')[0]
    assert 'Database connected' in web(wait(lecturer, browser_app, browser_job))
    for number in range(5, 31):
        payload = {'request_id': str(uuid.uuid4()), 'name': f'PHP capacity fixture {number}', 'student_id': fixture['student'], 'database_service_id': server, 'memory_mib': 256, 'cpus': .5}
        record = lecturer.request('POST', f'/environments/{environment}/php-applications', payload)
        assert lecturer.request('POST', f'/services/{record["application_id"]}/php-template/provision', {})['status'] == 'ready'
    assert len(lecturer.request('GET', f'/services/{server}/databases')) == 30
    assert int(live.sql(f'SELECT count(*) FROM {live.SCHEMA}.php_classroom_applications')) == 30
    print('PASS: flow expands to 30 assigned PHP application drafts/logical databases using one MariaDB service; no duplicate mappings or manually uploaded credentials.', flush=True)


original_cleanup = live.cleanup_resources


def cleanup():
    apps = json.loads(live.sql(f"SELECT coalesce(json_agg(id),'[]'::json) FROM {live.SCHEMA}.applications"))
    images = set()
    for app in apps:
        images.update(live.command(['docker', 'image', 'ls', '-q', '--no-trunc', '--filter', 'label=io.ferrix.managed=true', '--filter', f'label=io.ferrix.application={uuid.UUID(app)}']).split())
    original_cleanup()
    for image in images:
        # Never force removal; only images labeled for this disposable schema.
        subprocess.run(['docker', 'image', 'rm', image], cwd=live.ROOT, env=live.ENV, check=True, stdout=subprocess.DEVNULL)
    print('PASS: only this PHP fixture\'s built images removed; shared/base images retained.', flush=True)


live.cleanup_resources = cleanup
live.scenario = scenario
if __name__ == '__main__':
    live.main(static_dir=live.ROOT / 'apps/web/dist')
