#!/usr/bin/env python3
"""Real Caddy + Docker automatic-domain smoke test using only a disposable schema/network."""
import http.cookiejar
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

root = Path(__file__).resolve().parents[2]
profile = {}
for line in (root / '.env').read_text().splitlines():
    if line.strip() and not line.lstrip().startswith('#') and '=' in line:
        key, value = line.split('=', 1)
        profile[key.strip()] = value.strip().strip('"').strip("'")
parsed = urllib.parse.urlsplit(profile['DATABASE_URL'])
assert parsed.hostname in ('127.0.0.1', 'localhost') and parsed.port == int(profile['POSTGRES_PORT']) and parsed.path == '/' + profile['POSTGRES_DB']
schema = 'domains_test_' + uuid.uuid4().hex
network = schema + '_ingress'
proxy = schema + '_proxy'
env = os.environ | profile
compose = ['docker', 'compose', 'exec', '-T', 'postgres', 'psql', '-v', 'ON_ERROR_STOP=1', '-U', 'ferrix', '-d', profile['POSTGRES_DB']]

def sql(statement):
    return subprocess.check_output(compose + ['-At', '-c', statement], cwd=root, env=env, text=True)

def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

api_port, listener_port, edge_port = port(), port(), port()
assert len({api_port, listener_port, edge_port}) == 3
origin = f'http://127.0.0.1:{api_port}'
cookies = http.cookiejar.CookieJar()
client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))

def api(path, value=None, method=None):
    request = urllib.request.Request(origin + '/api/v1' + path, headers={'Origin': origin, 'Content-Type': 'application/json'}, data=None if value is None else json.dumps(value).encode(), method=method)
    with client.open(request, timeout=15) as response:
        body = response.read()
        return json.loads(body) if body else None

def gateway(url, path='/', extra=None):
    host = urllib.parse.urlsplit(url).hostname
    request = urllib.request.Request(f'http://127.0.0.1:{edge_port}{path}', headers={'Host': f'{host}:{edge_port}'} | (extra or {}))
    return urllib.request.urlopen(request, timeout=10)

process = None
proxy_tls = proxy + "_tls"
applications = []
html = '<!doctype html><html><head><link rel="stylesheet" href="/assets/style.css"></head><body><h1 id="result">Loading</h1><button id="counter">0</button><script type="module" src="/assets/app.js"></script></body></html>'
script = "import {message} from '/assets/value.js'; const data=await (await fetch('/data.json')).json(); document.querySelector('#result').textContent=message+' '+data.value; document.querySelector('#counter').onclick=e=>e.target.textContent=Number(e.target.textContent)+1;"
files = {'index.html': html, 'assets/app.js': script, 'assets/value.js': 'export const message="JavaScript works";', 'assets/style.css': 'h1 { color: rgb(20, 80, 140); }', 'data.json': '{"value":"through Caddy"}'}
sql(f'CREATE SCHEMA {schema}')
try:
    subprocess.run(['cargo', 'build', '-p', 'ferrix-api'], cwd=root, env=env, check=True)
    subprocess.run(['docker', 'network', 'create', '--label', f'io.ferrix.ingress={network}', network], env=env, check=True, stdout=subprocess.DEVNULL)
    query = urllib.parse.parse_qsl(parsed.query) + [('options', '-csearch_path=' + schema)]
    isolated = urllib.parse.urlunsplit(parsed._replace(query=urllib.parse.urlencode(query)))
    runtime = env | {'DATABASE_URL': isolated, 'FERRIX_BIND': f'127.0.0.1:{api_port}', 'FERRIX_AUTH_ORIGIN': origin, 'FERRIX_AUTH_KEY': secrets.token_hex(32), 'FERRIX_WEBSITE_BIND': f'127.0.0.1:{listener_port}', 'FERRIX_AUTO_DOMAIN': 'localhost', 'FERRIX_WEBSITE_HTTPS': 'false', 'FERRIX_PUBLIC_IP': '', 'FERRIX_INGRESS_NETWORK': network, 'FERRIX_WEBSITE_HTTP_PORT': str(edge_port)}
    runtime.pop('FERRIX_STATIC_DIR', None)
    with tempfile.TemporaryFile() as log:
        process = subprocess.Popen([str(root / 'target/debug/ferrix-api')], cwd=root, env=runtime, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        for attempt in range(100):
            try:
                urllib.request.urlopen(origin + '/health', timeout=1).close()
                break
            except OSError:
                if attempt == 99:
                    raise RuntimeError('Disposable API did not start')
                time.sleep(.2)
        subprocess.run(['docker', 'run', '-d', '--name', proxy, '--network', network, '-p', f'127.0.0.1:{edge_port}:80', '-e', f'FERRIX_WEBSITE_UPSTREAM=host.docker.internal:{listener_port}', '-v', f'{root}/deploy/Caddyfile.local:/etc/caddy/Caddyfile:ro', 'caddy:2-alpine'], env=env, check=True, stdout=subprocess.DEVNULL)
        api('/auth/setup', {'name': 'Domains test', 'email': 'domains@example.test', 'password': secrets.token_urlsafe(32)})
        project = api('/projects', {'name': 'Automatic domain test'})
        environment = api(f'/projects/{project["id"]}/environments')['environments'][0]['id']
        site = api(f'/environments/{environment}/static-websites', {'name': 'Interactive website'})
        site_path = f'/static-websites/{site["id"]}'
        assert site['automatic_url'].endswith(f'.localhost:{edge_port}/')
        api(site_path + '/publish', {'files': [{'path': path, 'hex': value.encode().hex()} for path, value in files.items()]})
        for attempt in range(30):
            try:
                with gateway(site['automatic_url']) as response:
                    assert response.read().decode() == html
                    assert 'sandbox' not in response.headers.get('Content-Security-Policy', '')
                break
            except OSError:
                if attempt == 29:
                    raise
                time.sleep(.25)
        for path, expected in files.items():
            with gateway(site['automatic_url'], '/' + path) as response:
                assert response.read().decode() == expected
        with gateway(site['automatic_url'], '/deep/client/route/', {'Accept': 'text/html'}) as response:
            assert response.read().decode() == html
        for path in ['/missing.js', '/_ferrix/route', '/_ferrix/tls-ask?domain=anything.test']:
            try:
                gateway(site['automatic_url'], path)
                raise AssertionError('Unregistered assets/private routes must fail')
            except urllib.error.HTTPError as error:
                assert error.code == 404
        try:
            gateway(f'http://unknown.localhost:{edge_port}/', extra={'X-Ferrix-Service-Type': 'application', 'X-Ferrix-Upstream': f'host.docker.internal:{api_port}'})
            raise AssertionError('Forged upstream headers accepted')
        except urllib.error.HTTPError as error:
            assert error.code == 404
        with gateway(site['automatic_url'], extra={'X-Ferrix-Service-Type': 'application', 'X-Ferrix-Upstream': f'host.docker.internal:{api_port}'}) as response:
            assert response.read().decode() == html
        with client.open(origin + '/api/v1' + site_path + '/preview/') as response:
            assert 'sandbox' in response.headers['Content-Security-Policy']
        app = api(f'/environments/{environment}/services', {'kind': 'application', 'name': 'Interactive application', 'image': 'nginx:alpine', 'container_port': 80})
        applications.append(app['id'])
        stable_url = app['automatic_url']
        api(f'/services/{app["id"]}/deploy', {})
        def wait_deploy():
            for attempt in range(180):
                history = api(f'/services/{app["id"]}/deployments')
                if history and history[0]['status'] == 'succeeded':
                    return history[0]['container_id']
                if history and history[0]['status'] == 'failed':
                    raise AssertionError('Disposable Application deploy failed: ' + history[0]['logs'])
                time.sleep(.5)
            raise AssertionError('Deploy timed out')
        first = wait_deploy()
        for path, value in files.items():
            subprocess.run(['docker', 'exec', '-i', first, 'sh', '-c', 'mkdir -p /usr/share/nginx/html/assets; cat > /usr/share/nginx/html/' + path], input=value, text=True, env=env, check=True)
        with gateway(stable_url) as response:
            assert response.read().decode() == html
        # Real browser verifies native .localhost resolution, module JS, fetch, CSS, clicks and deep-link reload.
        browser_test = r'''import {chromium} from 'playwright';
const browser=await chromium.launch(); const page=await browser.newPage(); const errors=[];page.on('pageerror',e=>errors.push(e.message));
for(const address of process.argv.slice(2)){ await page.goto(address);await page.getByRole('heading',{name:'JavaScript works through Caddy',exact:true}).waitFor();await page.locator('#counter').click();if(await page.locator('#counter').textContent()!=='1')throw Error('JS click failed');if(await page.locator('h1').evaluate(el=>getComputedStyle(el).color)!=='rgb(20, 80, 140)')throw Error('CSS failed');}
if(errors.length)throw Error(errors.join('\n'));await browser.close();console.log('Browser: native .localhost, module JS, fetch, CSS and click handlers passed.');'''
        # Resolve Playwright from the existing workspace without adding dependencies.
        browser_test = browser_test.replace("'playwright'", repr(str(root / 'apps/web/node_modules/playwright/index.mjs')))
        with tempfile.NamedTemporaryFile(suffix='.mjs', mode='w', delete=False) as file:
            file.write(browser_test)
            browser_file = Path(file.name)
        try:
            subprocess.run(['node', str(browser_file), site['automatic_url'], site['automatic_url'] + 'deep/client/route/', stable_url], cwd=root / 'apps/web', env=env, check=True)
        finally:
            browser_file.unlink(missing_ok=True)
        saved=api(f'/services/{app["id"]}/configuration')
        changed=saved | {'container_port': 81}
        api(f'/services/{app["id"]}/configuration',changed)
        with gateway(stable_url) as response:
            assert response.read().decode() == html, 'Routing must use the active deployment port, not unsaved deployment settings'
        api(f'/services/{app["id"]}/configuration',saved)
        api(f'/services/{app["id"]}/deploy', {})
        second = wait_deploy()
        assert first != second
        assert api(f'/services/{app["id"]}')['automatic_url'] == stable_url
        with gateway(stable_url) as response:
            assert b'Welcome to nginx' in response.read()
        # Exercise the production Caddyfile and sslip.io hostname shape over TLS.
        # A private test CA replaces public ACME; production still uses public issuers.
        os.killpg(process.pid,signal.SIGTERM)
        process.wait(timeout=15)
        remote_runtime=runtime | {'FERRIX_AUTO_DOMAIN':'sslip.io','FERRIX_PUBLIC_IP':'203.0.113.10'}
        process=subprocess.Popen([str(root / 'target/debug/ferrix-api')],cwd=root,env=remote_runtime,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        for attempt in range(100):
            try:
                urllib.request.urlopen(origin+'/health',timeout=1).close(); break
            except OSError:
                if attempt==99: raise
                time.sleep(.2)
        with tempfile.TemporaryDirectory(prefix='ferrix-domain-tls-') as directory:
            tls_port=port()
            fixture=Path(directory)
            caddy=(root/'deploy/Caddyfile').read_text().replace('{\n','{\n    local_certs\n',1).replace('ferrix:8081',f'host.docker.internal:{listener_port}').replace('ferrix:8080',f'host.docker.internal:{api_port}')
            (fixture/'Caddyfile').write_text(caddy)
            subprocess.run(['docker','run','-d','--name',proxy_tls,'--network',network,'-p',f'127.0.0.1:{tls_port}:443','-e','FERRIX_DOMAIN=dashboard.ferrix.test','-v',f'{fixture}/Caddyfile:/etc/caddy/Caddyfile:ro','caddy:2-alpine'],env=env,check=True,stdout=subprocess.DEVNULL)
            for attempt in range(40):
                copy=subprocess.run(['docker','cp',proxy_tls+':/data/caddy/pki/authorities/local/root.crt',str(fixture/'root.crt')],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                if copy.returncode==0: break
                if attempt==39: raise RuntimeError('Caddy test CA unavailable')
                time.sleep(.25)
            remote_site=api(site_path)['site']['automatic_url']
            remote_app=api(f'/services/{app["id"]}')['automatic_url']
            for address,expected in [(remote_site,html.encode()),(remote_app,b'Welcome to nginx')]:
                host=urllib.parse.urlsplit(address).hostname
                output=subprocess.check_output(['curl','--silent','--show-error','--fail','--cacert',str(fixture/'root.crt'),'--noproxy','*','--resolve',f'{host}:{tls_port}:127.0.0.1',f'https://{host}:{tls_port}/'],env=env)
                assert expected in output
            host=urllib.parse.urlsplit(remote_site).hostname
            other=urllib.parse.urlsplit(remote_app).hostname
            code=subprocess.check_output(['curl','--silent','--cacert',str(fixture/'root.crt'),'--noproxy','*','--resolve',f'{host}:{tls_port}:127.0.0.1','-H',f'Host: {other}:{tls_port}','-o','/dev/null','-w','%{http_code}',f'https://{host}:{tls_port}/'],env=env,text=True)
            assert code=='421','SNI/Host mismatch must be rejected'
            print('TLS: production Caddyfile, sslip.io automatic hostnames, trusted test certificates and strict SNI/Host isolation passed.')
            subprocess.run(['docker','rm','-f',proxy_tls],env=env,stdout=subprocess.DEVNULL,check=True)
        # Both hostname modes deny a deleted resource.
        api(site_path, {'name': 'Interactive website'}, method='DELETE')
        try:
            gateway(site['automatic_url'])
            raise AssertionError('Deleted website still resolves')
        except urllib.error.HTTPError as error:
            assert error.code == 404
        print('Caddy: static assets, SPA fallback, isolation, spoof protection and stable Application redeploy URL passed.')
finally:
    if process and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
    subprocess.run(['docker', 'rm', '-f', proxy_tls], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(['docker', 'rm', '-f', proxy], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for app in applications:
        ids = subprocess.check_output(['docker', 'ps', '-aq', '--filter', f'label=io.ferrix.application={app}'], env=env, text=True).split()
        if ids:
            subprocess.run(['docker', 'rm', '-f', *ids], env=env, check=True, stdout=subprocess.DEVNULL)
    # Only resources with environment IDs belonging to this disposable schema are removed.
    try:
        environments = sql(f'SELECT id FROM {schema}.environments').split()
        for environment in environments:
            subprocess.run(['docker', 'network', 'rm', 'ferrix-env-' + uuid.UUID(environment).hex], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    finally:
        subprocess.run(['docker', 'network', 'rm', network], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        sql(f'DROP SCHEMA {schema} CASCADE')
