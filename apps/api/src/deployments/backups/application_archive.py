"""Offline archive helper: never emits filenames, file contents or credentials."""
import hashlib
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys
import tarfile
import tempfile

MAX_FILES = 100_000
MAX_BYTES = 20 * 1024**3

def safe_member(member, seen):
    path = PurePosixPath(member.name)
    if (path.is_absolute() or '..' in path.parts or '\\' in member.name
            or '\x00' in member.name or len(member.name) > 4096 or str(path) in seen
            or member.size < 0 or (str(path) == '.' and not member.isdir())
            or not (member.isfile() or member.isdir())
            or not 0 <= member.uid < 2**32-1 or not 0 <= member.gid < 2**32-1):
        raise ValueError('unsafe archive')
    seen.add(str(path))
    return member

def checksum(path):
    h = hashlib.sha256()
    with open(path, 'rb') as source:
        for chunk in iter(lambda: source.read(1024*1024), b''):
            h.update(chunk)
    return h.hexdigest()

def backup(data, archive):
    partial = Path(str(archive)+'.part')
    os.umask(0o077)
    count = size = 0
    try:
        with tarfile.open(partial, 'w:gz', dereference=True) as output:
            for root, dirs, files in os.walk(data, followlinks=False):
                dirs.sort(); files.sort()
                for path in [Path(root)] + [Path(root)/name for name in files]:
                    info = path.lstat()
                    if not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
                        raise ValueError('unsupported filesystem entry')
                    # Reject links rather than following them to another filesystem.
                    if path.is_symlink() or any((Path(root)/d).is_symlink() for d in dirs):
                        raise ValueError('unsupported link')
                    count += 1; size += info.st_size if stat.S_ISREG(info.st_mode) else 0
                    if count > MAX_FILES or size > MAX_BYTES:
                        raise ValueError('archive limit')
                    member = output.gettarinfo(str(path), arcname=str(path.relative_to(data)))
                    member.uname = member.gname = ''
                    if member.isfile():
                        with open(path, 'rb') as source: output.addfile(member, source)
                    else: output.addfile(member)
        with open(partial, 'rb') as f: os.fsync(f.fileno())
        os.replace(partial, archive)
        directory = os.open(archive.parent, os.O_DIRECTORY)
        try: os.fsync(directory)
        finally: os.close(directory)
        print(archive.stat().st_size, checksum(archive), flush=True)
    finally:
        partial.unlink(missing_ok=True)

def restore(data, archive, expected):
    if checksum(archive) != expected:
        raise ValueError('checksum mismatch')
    if any(data.iterdir()):
        raise ValueError('target must be empty')
    seen = set(); size = 0
    with tarfile.open(archive, 'r:gz') as source:
        members = []
        for member in source:
            safe_member(member, seen)
            size += member.size
            if len(members) >= MAX_FILES or size > MAX_BYTES:
                raise ValueError('archive limit')
            members.append(member)
        staging = Path(tempfile.mkdtemp(prefix='.ferrix-restore-', dir=data))
        try:
            # Only relative regular files/directories survived validation. The target
            # is empty, offline and not exposed to another writer. Preserve numeric IDs.
            source.extractall(staging, members=members, numeric_owner=True, filter='fully_trusted')
            root = next((m for m in members if PurePosixPath(m.name) == PurePosixPath('.')), None)
            for item in staging.iterdir(): os.replace(item, data/item.name)
            staging.rmdir()
            if root:
                os.chown(data, root.uid, root.gid)
                os.chmod(data, root.mode)
                os.utime(data, (root.mtime, root.mtime))
            directory = os.open(data, os.O_DIRECTORY)
            try: os.fsync(directory)
            finally: os.close(directory)
        finally:
            if staging.exists(): shutil.rmtree(staging)

if __name__ == '__main__':
    try:
        operation, directory, filename = sys.argv[1:4]
        if operation == 'backup': backup(Path(directory), Path(filename))
        elif operation == 'restore': restore(Path(directory), Path(filename), sys.argv[4])
        else: raise ValueError('invalid operation')
    except Exception:
        # Do not leak archive paths, SQLite contents or secrets through a traceback.
        print('Application archive operation failed: check space, supported entries and integrity.', file=sys.stderr)
        sys.exit(42)
