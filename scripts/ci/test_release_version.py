import unittest

from release_version import validate


class ReleaseVersion(unittest.TestCase):
    def check(self, tag="v0.2.0", version="0.2.0", locked="0.2.0", api="{ workspace = true }"):
        validate(tag, f'[workspace.package]\nversion = "{version}"',
                 f'[package]\nversion = {api}',
                 f'[[package]]\nname = "ferrix-api"\nversion = "{locked}"')

    def test_matching_release_and_prerelease(self):
        self.check()
        self.check("v0.3.0-rc.1", "0.3.0-rc.1", "0.3.0-rc.1")

    def test_stale_manifest_or_lock_rejected(self):
        for values in [{"version": "0.1.0"}, {"locked": "0.1.0"}, {"tag": "v0.3.0"}, {"api": '"0.2.0"'}, {"tag": "latest"}]:
            with self.subTest(values=values), self.assertRaises(ValueError):
                self.check(**values)
