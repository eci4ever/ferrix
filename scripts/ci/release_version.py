"""Check the release tag against the exact source being published (Python 3.11+)."""
import re
import subprocess
import sys
import tomllib


def validate(tag, manifest, api, lock):
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[a-zA-Z0-9.-]+)?", tag):
        raise ValueError("Expected a version tag such as v0.2.0")
    expected = tag[1:]
    version = tomllib.loads(manifest)["workspace"]["package"]["version"]
    if version != expected:
        raise ValueError(f"Release {tag} requires workspace version {expected}; found {version}")
    if tomllib.loads(api)["package"]["version"] != {"workspace": True}:
        raise ValueError("API must inherit workspace.package.version")
    packages = tomllib.loads(lock)["package"]
    versions = [package["version"] for package in packages if package["name"] == "ferrix-api"]
    if versions != [expected]:
        raise ValueError(f"Cargo.lock ferrix-api version must be {expected}")


if __name__ == "__main__":
    try:
        tag, commit = sys.argv[1:]
        files = [subprocess.check_output(["git", "show", f"{commit}:{path}"], text=True)
                 for path in ["Cargo.toml", "apps/api/Cargo.toml", "Cargo.lock"]]
        validate(tag, *files)
    except (ValueError, KeyError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
    print(f"Release {tag} matches API source and lockfile.")
