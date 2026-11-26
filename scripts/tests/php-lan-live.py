#!/usr/bin/env python3
"""PHP starter over real LAN ingress + trusted TLS and an Internet-isolated client.

Does not edit system DNS/hosts/trust stores or the regular Ferrix configuration.
"""
import argparse
import importlib.util
import ipaddress
import json
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import uuid

spec = importlib.util.spec_from_file_location('php_fixture', Path(__file__).with_name('php-template-live.py'))
php_fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(php_fixture)
live = php_fixture.live
live.SCHEMA = 'php_lan_' + uuid.uuid4().hex


def free_port(address='127.0.0.1'):
    with socket.socket() as listener:
        listener.bind((address, 0))
        return listener.getsockname()[1]


def scenario(origin):
    owner, manager, student = (live.Client(origin) for _ in range(3))
    password = secrets.token_urlsafe(32)
    owner.request('POST', '/auth/setup', {'name': 'Platform', 'email': 'platform@example.test', 'password': password})
    for client, name, role in [(manager, 'lecturer', 'admin'), (student, 'student', 'member')]:
        invite = owner.request('POST', '/auth/invitations', {'email': name+'@example.test', 'role': role})
        client.request('POST', '/auth/accept-invitation', {'name': name, 'email': name+'@example.test', 'password': password, 'token': invite['token']})
    assert manager.request('GET', '/auth/session')['system_role'] == 'user'
    project = manager.request('POST', '/projects', {'name': 'Temporary PHP LAN check'}, expected=201)['id']
    environment = manager.request('GET', f'/projects/{project}/environments')['environments'][0]['id']
    server = manager.request('POST', f'/environments/{environment}/services', {'kind': 'database', 'name': 'LAN MariaDB', 'engine': 'mariadb'}, expected=201)['id']
    php_fixture.wait(manager, server, manager.request('POST', f'/services/{server}/deploy', {}, expected=202))
    manager.request('POST', '/auth/organizations/access', {'restricted': True}, expected=204)
    record = manager.request('POST', f'/environments/{environment}/php-applications', {'request_id': str(uuid.uuid4()), 'name': 'LAN PHP', 'student_id': student.request('GET', '/auth/session')['user_id'], 'database_service_id': server, 'memory_mib': 256, 'cpus': .5})
    app = record['application_id']
    manager.request('POST', f'/services/{app}/php-template/provision', {})
    php_fixture.wait(student, app, student.request('POST', f'/services/{app}/deploy', {}, expected=202))
    application = student.request('GET', f'/services/{app}')
    url = application['automatic_url'];host = 'app-'+app.replace('-', '')+'.'+args.suffix
    assert url == f'https://{host}:{edge_port}/'
    print('PASS: non-platform lecturer provisions assigned PHP/MariaDB application with a real private HTTPS URL.', flush=True)
    # Reach the actual host LAN interface, with explicit client-side host mapping.
    curl = ['curl', '--noproxy', '*', '--silent', '--show-error', '--max-time', '15', '--resolve', f'{host}:{edge_port}:{args.lan_ip}']
    untrusted = subprocess.run(curl+[url], env=live.ENV, capture_output=True)
    assert untrusted.returncode == 60, 'A new internal CA must not be trusted implicitly'
    with tempfile.TemporaryDirectory(prefix='ferrix-php-lan-') as directory:
        ca = Path(directory)/'root.crt'
        # Only the public certificate is exported; never export the CA private key.
        ca.write_bytes(subprocess.check_output(['docker', 'exec', proxy, 'cat', '/data/caddy/pki/authorities/local/root.crt'], env=live.ENV))
        page = subprocess.check_output(curl+['--cacert', str(ca), url], env=live.ENV).decode()
        assert 'Database connected' in page and 'Your application is running.' in page
        style = subprocess.check_output(curl+['--cacert', str(ca), url+'style.css'], env=live.ENV).decode()
        assert len(style) > 20
        print('PASS: ferrix.pi.local suffix HTTPS reaches the actual LAN IP; untrusted CA rejected, explicit CA validates hostname and serves page/styles.', flush=True)
        # Put a fresh client in an internal network with no Internet default route.
        proxy_ip = live.command(['docker', 'inspect', proxy, '--format', '{{(index .NetworkSettings.Networks "'+client_network+'").IPAddress}}'])
        code = "$ctx=stream_context_create(['ssl'=>['cafile'=>'/root.crt','verify_peer'=>true,'verify_peer_name'=>true]]);$html=file_get_contents($argv[1],false,$ctx);$css=file_get_contents($argv[1].'style.css',false,$ctx);if(!str_contains($html,'Database connected')||strlen($css)<20)exit(1);$outside=@fsockopen('1.1.1.1',443,$n,$s,2);if($outside)exit(2);echo 'PASS: isolated client loads PHP page and stylesheet with verified TLS while external Internet is unreachable.\\n';"
        subprocess.run(['docker', 'run', '--rm', '--network', client_network, '--add-host', host+':'+proxy_ip, '-v', str(ca)+':/root.crt:ro', '--entrypoint', 'php', 'php:8.5-apache-bookworm', '-r', code, '--', f'https://{host}/'], env=live.ENV, check=True, timeout=40)
    print('Client DNS/hosts and CA trust were explicit test prerequisites; school-device DNS/CA installation is still an operator step.', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lan-ip', required=True)
    parser.add_argument('--suffix', default='ferrix.pi.local')
    args = parser.parse_args()
    address = ipaddress.ip_address(args.lan_ip)
    assert address.is_private and not address.is_loopback and not address.is_unspecified
    edge_port, upstream_port = free_port(args.lan_ip), free_port()
    network = live.SCHEMA+'_ingress';client_network = live.SCHEMA+'_offline';proxy = live.SCHEMA+'_proxy'
    try:
        live.command(['docker', 'network', 'create', '--label', 'io.ferrix.ingress='+network, network])
        live.command(['docker', 'network', 'create', '--internal', client_network])
        live.command(['docker', 'run', '-d', '--name', proxy, '--network', network, '-p', f'{args.lan_ip}:{edge_port}:443', '-e', f'FERRIX_WEBSITE_UPSTREAM=host.docker.internal:{upstream_port}', '-v', str(live.ROOT/'deploy/Caddyfile.local-https')+':/etc/caddy/Caddyfile:ro', 'caddy:2-alpine'])
        live.command(['docker', 'network', 'connect', client_network, proxy])
        live.scenario = scenario
        # This temporary proxy is explicitly cleaned below and must not enter the
        # fixture's preservation baseline as a service it owns/removes itself.
        live.main(runtime_overrides={'FERRIX_AUTO_DOMAIN': 'private', 'FERRIX_INGRESS_IP': args.lan_ip, 'FERRIX_DOMAIN_SUFFIX': args.suffix, 'FERRIX_INGRESS_NETWORK': network, 'FERRIX_WEBSITE_HTTPS': 'true', 'FERRIX_WEBSITE_HTTPS_PORT': str(edge_port), 'FERRIX_WEBSITE_BIND': f'0.0.0.0:{upstream_port}'})
    finally:
        subprocess.run(['docker', 'rm', '-fv', proxy], env=live.ENV, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for owned in [client_network, network]:
            subprocess.run(['docker', 'network', 'rm', owned], env=live.ENV, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
