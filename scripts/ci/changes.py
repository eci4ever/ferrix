#!/usr/bin/env python3
"""Select CI components from the complete Git diff, without API file-list limits."""
import os
from pathlib import Path
import re
import subprocess

RUST_FILES = {
    'Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml',
    'rustfmt.toml', '.rustfmt.toml', 'clippy.toml', '.clippy.toml',
}
SHARED_FILES = {'Dockerfile', '.dockerignore', 'compose.yaml', 'compose.production.yaml'}
DOC_FILES = {'LICENSE', 'LICENSE.txt', 'LICENSE.md'}
SHA = re.compile(r'^[0-9a-fA-F]{40}(?:[0-9a-fA-F]{24})?$')


def classify(paths):
    rust = frontend = False
    for path in paths:
        if path in SHARED_FILES or path.startswith(('.github/workflows/', 'scripts/')):
            rust = frontend = True
        elif path in RUST_FILES or path.startswith(('.cargo/', 'apps/api/')):
            rust = True
        elif path.startswith('apps/web/'):
            frontend = True
        elif path.startswith('docs/') or path.endswith('.md') or path in DOC_FILES:
            continue
        else:
            # New shared directories/configuration must never silently evade CI.
            rust = frontend = True
    return rust, frontend


def select(event, base, head, cwd=None):
    if event == 'workflow_dispatch':
        return True, True
    if event not in ('push', 'pull_request') or not all(
        SHA.fullmatch(sha or '') and set(sha) != {'0'} for sha in (base, head)
    ):
        return True, True
    comparison = f'{base}...{head}' if event == 'pull_request' else f'{base}..{head}'
    try:
        result = subprocess.run(
            ['git', 'diff', '--no-ext-diff', '--no-renames', '--name-only', '-z', comparison, '--'],
            cwd=cwd, check=True, capture_output=True, timeout=30,
        )
    except (OSError, subprocess.SubprocessError):
        print('::warning::Changed-file detection unavailable; running both CI components.')
        return True, True
    # No rename detection: moving files checks both the source and destination.
    paths = [os.fsdecode(path) for path in result.stdout.split(b'\0') if path]
    return classify(paths)


def main():
    rust, frontend = select(os.getenv('CI_EVENT', ''), os.getenv('CI_BASE', ''), os.getenv('CI_HEAD', ''))
    result = f'rust={str(rust).lower()}\nfrontend={str(frontend).lower()}\n'
    with Path(os.environ['GITHUB_OUTPUT']).open('a') as output:
        output.write(result)
    print(result, end='')


if __name__ == '__main__':
    main()
