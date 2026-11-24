"""Archive safety tests; use temporary directories, never a Ferrix service volume."""
import importlib.util
import io
from pathlib import Path
import sqlite3
import tarfile
import tempfile
import unittest

root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('application_archive', root/'apps/api/src/deployments/backups/application_archive.py')
archive = importlib.util.module_from_spec(spec)
spec.loader.exec_module(archive)

class ArchiveTests(unittest.TestCase):
    def test_complete_sqlite_and_dotfile_roundtrip(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory); source=base/'source'; target=base/'target'
            source.mkdir(); target.mkdir(); (source/'.private').write_text('disposable fixture')
            (source/'.private').chmod(0o600)
            with sqlite3.connect(source/'state.db') as db:
                db.execute('CREATE TABLE records(value text)'); db.execute("INSERT INTO records VALUES('fixture')")
            output=base/'backup.dump'; archive.backup(source,output)
            archive.restore(target,output,archive.checksum(output))
            self.assertEqual((target/'.private').read_text(),'disposable fixture')
            self.assertEqual((target/'.private').stat().st_mode & 0o777,0o600)
            with sqlite3.connect(target/'state.db') as db:
                self.assertEqual(db.execute('PRAGMA integrity_check').fetchone()[0],'ok')
                self.assertEqual(db.execute('SELECT value FROM records').fetchone()[0],'fixture')
    def test_malicious_paths_links_and_devices_do_not_extract(self):
        for name,kind in [('../outside',tarfile.REGTYPE),('/outside',tarfile.REGTYPE),('a/../../outside',tarfile.REGTYPE),('a\\outside',tarfile.REGTYPE),('link',tarfile.SYMTYPE),('hard',tarfile.LNKTYPE),('device',tarfile.CHRTYPE)]:
            with self.subTest(name=name),tempfile.TemporaryDirectory() as directory:
                base=Path(directory);target=base/'target';target.mkdir();output=base/'backup.dump'
                with tarfile.open(output,'w:gz') as f:
                    member=tarfile.TarInfo(name);member.type=kind;member.linkname='../outside'
                    f.addfile(member,io.BytesIO())
                with self.assertRaises(ValueError):archive.restore(target,output,archive.checksum(output))
                self.assertEqual(list(target.iterdir()),[])
    def test_checksum_and_nonempty_target_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);source=base/'source';target=base/'target';source.mkdir();target.mkdir()
            (source/'file').write_text('fixture');output=base/'backup.dump';archive.backup(source,output)
            with self.assertRaises(ValueError):archive.restore(target,output,'0'*64)
            (target/'existing').write_text('keep')
            with self.assertRaises(ValueError):archive.restore(target,output,archive.checksum(output))
            self.assertEqual((target/'existing').read_text(),'keep')
    def test_backup_rejects_symlinks_and_does_not_finalize(self):
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);source=base/'source';source.mkdir();(source/'link').symlink_to('/etc/passwd')
            output=base/'backup.dump'
            with self.assertRaises(ValueError):archive.backup(source,output)
            self.assertFalse(output.exists());self.assertFalse(Path(str(output)+'.part').exists())

if __name__=='__main__':unittest.main()
