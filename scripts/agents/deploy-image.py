#!/usr/bin/env python3
"""Create/find an application and deploy it using an organization-scoped Ferrix key.

Credentials are read only from FERRIX_API_KEY. This client never sends cookies,
prints credentials, follows redirects, or overwrites an existing configuration.
"""
import argparse
import ipaddress
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request


class AgentError(Exception):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Client:
    def __init__(self, url, key):
        parsed = urllib.parse.urlsplit(url)
        try:
            local = parsed.hostname == 'localhost' or ipaddress.ip_address(parsed.hostname or '').is_loopback
        except ValueError:
            local = False
        if (parsed.scheme != 'https' and not (parsed.scheme == 'http' and local)) or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise AgentError('Use an HTTPS Ferrix URL; HTTP is allowed only on loopback.')
        if parsed.path.rstrip('/') not in ('', '/api/v1'):
            raise AgentError('Use the Ferrix origin or its /api/v1 URL.')
        if not key or not key.startswith('frx_') or len(key) != 68 or any(char not in '0123456789abcdefABCDEF' for char in key[4:]):
            raise AgentError('Set FERRIX_API_KEY through your agent secret store.')
        self.base = urllib.parse.urlunsplit(parsed._replace(path='/api/v1', query='', fragment='')).rstrip('/')
        self.key = key
        self.opener = urllib.request.build_opener(NoRedirect())

    def call(self, method, path, body=None):
        payload = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(self.base + path, data=payload, method=method, headers={
            'Authorization': 'Bearer ' + self.key,
            'Content-Type': 'application/json',
            'User-Agent': 'ferrix-deploy-image/1',
        })
        try:
            with self.opener.open(request, timeout=20) as response:
                return json.load(response) if response.status != 204 else None
        except urllib.error.HTTPError as error:
            hints = {
                401: 'The API key was rejected. Stop and request a valid key.',
                403: 'The API key scope or current organization role does not permit this action.',
                404: 'The resource is not available in this organization.',
                409: 'A conflicting resource or deployment exists. Inspect it in Ferrix before retrying.',
                429: 'Rate limited. Wait for Retry-After before retrying.',
            }
            # Do not echo an untrusted response body, URL or HTTP header containing secrets.
            raise AgentError(hints.get(error.code, f'Ferrix returned HTTP {error.code}. Inspect the operation in the dashboard.')) from None
        except (OSError, ValueError):
            raise AgentError('Cannot read a Ferrix response. Check connectivity; do not blindly retry writes.') from None


def choose(items, name, kind):
    matches = [item for item in items if item['name'] == name]
    if len(matches) > 1:
        raise AgentError(f'Multiple {kind} records have this name. Use an explicit project ID or rename the records.')
    return matches[0] if matches else None


def deploy(client, args):
    identity = client.call('GET', '/agent')
    if identity['organization_role'] not in ('owner', 'admin') or not {'read', 'manage', 'deploy'}.issubset(identity['scopes']):
        raise AgentError('This workflow needs read/manage/deploy and an owner/admin organization role.')
    bound_project = identity.get('project_id')
    if bound_project:
        if args.project_id and args.project_id != bound_project:
            raise AgentError('This API key is limited to another project. Use its assigned project ID.')
        project = {'id': bound_project}
    elif args.project_id:
        project = {'id': args.project_id}
    else:
        project = choose(client.call('GET', '/projects'), args.project, 'project')
        if project is None:
            project = client.call('POST', '/projects', {'name': args.project, 'description': 'Created by a deployment agent'})
    project_id = urllib.parse.quote(project['id'], safe='')
    environments = client.call('GET', f'/projects/{project_id}/environments')['environments']
    environment = choose(environments, args.environment, 'environment')
    if environment is None:
        environment = client.call('POST', f'/projects/{project_id}/environments', {'name': args.environment})
    environment_id = urllib.parse.quote(environment['id'], safe='')
    service = choose(client.call('GET', f'/environments/{environment_id}/services'), args.application, 'service')
    if service is None:
        service = client.call('POST', f'/environments/{environment_id}/services', {
            'kind': 'application', 'name': args.application, 'image': args.image,
            'container_port': args.port, 'environment': '',
        })
    elif service['kind'] != 'application' or service['image'] != args.image or service['container_port'] != args.port:
        raise AgentError('Existing service configuration differs. Update it explicitly in Ferrix first; this script preserves existing environment variables and database settings.')
    service_id = urllib.parse.quote(service['id'], safe='')
    job = client.call('POST', f'/services/{service_id}/deploy', {})
    deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        history = client.call('GET', f'/services/{service_id}/deployments')
        current = next((item for item in history if item['id'] == job['id']), None)
        if current and current['status'] == 'failed':
            raise AgentError('Deployment failed. Inspect its logs in Ferrix; log output may contain application secrets.')
        if current and current['status'] == 'succeeded':
            runtime = client.call('GET', f'/services/{service_id}/runtime')
            service = client.call('GET', f'/services/{service_id}')
            return {'project_id': project['id'], 'environment_id': environment['id'], 'service_id': service['id'],
                    'deployment_id': job['id'], 'status': current['status'], 'runtime_state': runtime['state'],
                    'automatic_url': service.get('automatic_url'), 'host_port': service.get('host_port')}
        time.sleep(3)
    raise AgentError('Timed out waiting for deployment. It may still be running; inspect Ferrix before retrying.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', default=os.environ.get('FERRIX_URL', 'http://127.0.0.1:5173'))
    parser.add_argument('--project', default='Agent project')
    parser.add_argument('--project-id', help='Use an existing project ID instead of matching a name')
    parser.add_argument('--environment', default='production')
    parser.add_argument('--application', default='web')
    parser.add_argument('--image', default='nginx:alpine')
    parser.add_argument('--port', type=int, default=80)
    parser.add_argument('--timeout', type=int, default=300)
    args = parser.parse_args()
    if not 1 <= args.port <= 65535 or not 1 <= args.timeout <= 3600:
        parser.error('Port must be 1–65535 and timeout 1–3600 seconds.')
    try:
        print(json.dumps(deploy(Client(args.url, os.environ.get('FERRIX_API_KEY', '')), args)))
    except AgentError as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
