"""Exercise component selection and real PR/push Git comparisons."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from changes import classify, select


class Classification(unittest.TestCase):
    def test_component_boundaries(self):
        for path in ['apps/web/src/main.tsx', 'apps/web/package-lock.json', 'apps/web/vendor/picomatch/index.js', 'apps/web/public/favicon.svg']:
            with self.subTest(path=path):
                self.assertEqual(classify([path]), (False, True))
        for path in ['docs/api-keys.md', 'apps/api/src/main.rs', 'apps/api/migrations/0010.sql', 'apps/api/tests/http.rs', 'Cargo.lock', 'Cargo.toml', '.cargo/config.toml', 'rust-toolchain.toml']:
            with self.subTest(path=path):
                self.assertEqual(classify([path]), (True, False))

    def test_shared_and_unknown_changes(self):
        for path in ['.github/workflows/ci.yaml', 'scripts/ci/changes.py', 'Dockerfile', 'compose.production.yaml', 'new-shared-library/code.rs']:
            with self.subTest(path=path):
                self.assertEqual(classify([path]), (True, True))
        self.assertEqual(classify(['apps/web/src/main.tsx', 'apps/api/src/main.rs']), (True, True))

    def test_documentation_and_empty_diff(self):
        self.assertEqual(classify(['README.md', 'AGENTS.md', 'docs/development.md', 'LICENSE']), (False, False))
        self.assertEqual(classify([]), (False, False))
        self.assertEqual(classify(['apps/web-extra/file.js']), (True, True))

    def test_manual_missing_base_and_git_failure_run_everything(self):
        self.assertEqual(select('workflow_dispatch', '', ''), (True, True))
        self.assertEqual(select('push', '0' * 40, '1' * 40), (True, True))
        self.assertEqual(select('pull_request', '--bad-option', '1' * 40), (True, True))
        with patch('changes.subprocess.run', side_effect=subprocess.CalledProcessError(1, 'git')):
            self.assertEqual(select('push', '1' * 40, '2' * 40), (True, True))


class GitComparisons(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='ferrix-ci-paths-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git('init', '-q')
        self.git('config', 'user.name', 'CI test')
        self.git('config', 'user.email', 'ci@example.test')
        self.write('README.md')
        self.base = self.commit()

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, text=True).strip()

    def write(self, path, text='test'):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def commit(self):
        self.git('add', '-A')
        self.git('commit', '-qm', 'fixture')
        return self.git('rev-parse', 'HEAD')

    def test_push_includes_all_commits_not_only_the_last(self):
        self.write('apps/web/src/page.tsx')
        self.commit()
        self.write('docs/guide.md')
        head = self.commit()
        self.assertEqual(select('push', self.base, head, self.root), (False, True))

    def test_pr_uses_merge_base_and_excludes_base_only_changes(self):
        self.git('checkout', '-qb', 'topic', self.base)
        self.write('apps/web/src/page.tsx')
        head = self.commit()
        self.git('checkout', '-qb', 'advanced-base', self.base)
        self.write('apps/api/src/server.rs')
        advanced = self.commit()
        self.assertEqual(select('pull_request', advanced, head, self.root), (False, True))

    def test_rename_and_deletion_check_both_components(self):
        self.write('apps/api/shared.txt')
        before = self.commit()
        self.write('apps/web/shared.txt')
        (self.root / 'apps/api/shared.txt').unlink()
        after = self.commit()
        self.assertEqual(select('push', before, after, self.root), (True, True))
        (self.root / 'apps/web/shared.txt').unlink()
        deleted = self.commit()
        self.assertEqual(select('push', after, deleted, self.root), (False, True))

    def test_diff_handles_newlines_and_has_no_300_file_limit(self):
        for index in range(350):
            self.write(f'docs/{index}.md')
        self.write('apps/api/src/unusual\nname.rs')
        head = self.commit()
        self.assertEqual(select('push', self.base, head, self.root), (True, False))


if __name__ == '__main__':
    unittest.main()
