"""Exercise the shipped updater script with a recording Docker stub; never touches Docker or /opt."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'apps/api/src/self_update/update.sh'
OLD = 'ghcr.io/eci4ever/ferrix@sha256:' + 'a' * 64
NEW = 'ghcr.io/eci4ever/ferrix@sha256:' + 'b' * 64
STUB = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
args=sys.argv[1:]
root=pathlib.Path(os.environ['UPDATE_TEST_ROOT'])
with (root/'calls').open('a') as f: f.write(json.dumps(args)+'\n')
scenario=os.environ['UPDATE_TEST_SCENARIO']
if args[0]=='pull': sys.exit(1 if scenario=='pull_failure' else 0)
if args[:2]==['image','inspect']:
 fmt=args[3]
 print('amd64' if 'Architecture' in fmt else '0.2.4' if 'Labels' in fmt else 'ghcr.io/eci4ever/ferrix@sha256:'+'b'*64)
elif args[0]=='exec': print('31')
elif args[0]=='run': print('32' if scenario=='migration_failure' else '31')
elif args[0]=='compose':
 command=args[7:]
 if command[:2]==['ps','-q']: print('source-container')
 elif command[:2]==['config','--services']: print('postgres\nferrix\nproxy'+ ('\nforeign' if scenario=='altered_stack' else ''))
 elif command[:4]==['exec','-T','postgres','pg_dump']:
  print('valid postgres archive')
  if scenario=='backup_failure': sys.exit(1)
 elif command[:4]==['exec','-T','postgres','pg_restore']:
  assert sys.stdin.read().strip()=='valid postgres archive'
 elif command[0]=='stop' and scenario=='stop_failure': sys.exit(1)
 elif command[0]=='up':
  counter=root/'up-count'
  count=int(counter.read_text())+1 if counter.exists() else 1
  counter.write_text(str(count))
  if count==1 and scenario in ('runtime_failure','migration_failure'): sys.exit(1)
 elif command[:4]==['exec','-T','ferrix','/usr/local/bin/ferrix-api']: print('0.2.4')
'''

class UpdateScriptTests(unittest.TestCase):
    def run_update(self, scenario):
        task = tempfile.TemporaryDirectory(prefix='ferrix-update-test-')
        self.addCleanup(task.cleanup)
        root=Path(task.name)
        (root/'docker').write_text(STUB)
        (root/'docker').chmod(0o700)
        original='FERRIX_IMAGE=ghcr.io/eci4ever/ferrix:0.2.3\nFERRIX_AUTH_KEY=secret-sentinel\nPOSTGRES_PASSWORD=keep-this\n'
        (root/'.env').write_text(original)
        (root/'compose.yaml').write_text('test compose')
        (root/'Caddyfile').write_text('test caddy')
        script=SCRIPT.read_text().replace('/opt/ferrix',str(root))
        result=subprocess.run(['sh','-c',script,'test-update','0.2.4','test','source-container',OLD,'job'],env=os.environ|{'PATH':str(root)+os.pathsep+os.environ['PATH'],'UPDATE_TEST_ROOT':str(root),'UPDATE_TEST_SCENARIO':scenario},text=True,capture_output=True)
        calls=[json.loads(line) for line in (root/'calls').read_text().splitlines()]
        self.assertNotIn('secret-sentinel',result.stdout+result.stderr)
        return root,original,result,calls

    def test_success_backs_up_before_replacing_only_ferrix_and_pins_digest(self):
        root,original,result,calls=self.run_update('success')
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('FERRIX_UPDATE_STAGE=succeeded',result.stdout)
        self.assertEqual((root/'updates/job/env').read_text(),original)
        self.assertTrue((root/'updates/job/database.dump').stat().st_size)
        self.assertIn('FERRIX_IMAGE='+NEW,(root/'.env').read_text())
        self.assertIn('FERRIX_AUTH_KEY=secret-sentinel',(root/'.env').read_text())
        dump=next(i for i,c in enumerate(calls) if 'pg_dump' in c)
        up=next(i for i,c in enumerate(calls) if 'up' in c)
        stop=next(i for i,c in enumerate(calls) if 'stop' in c)
        self.assertLess(dump,stop)
        self.assertLess(stop,up)
        for call in calls:
            if call[0]=='compose' and ('up' in call or 'stop' in call): self.assertEqual(call[-1],'ferrix')
        self.assertEqual((root/'updates/job/env').stat().st_mode & 0o777,0o600)

    def test_pull_failure_does_not_stop_or_change_installation(self):
        root,original,result,calls=self.run_update('pull_failure')
        self.assertNotEqual(result.returncode,0)
        self.assertEqual((root/'.env').read_text(),original)
        self.assertFalse(any('stop' in c for c in calls))

    def test_backup_failure_keeps_original_running_and_environment_unchanged(self):
        root,original,result,calls=self.run_update('backup_failure')
        self.assertNotEqual(result.returncode,0)
        self.assertEqual((root/'.env').read_text(),original)
        self.assertFalse(any('stop' in c or 'start' in c for c in calls))
        self.assertFalse(any('up' in c for c in calls))

    def test_stop_failure_attempts_to_restart_the_original_image(self):
        root,original,result,calls=self.run_update('stop_failure')
        self.assertNotEqual(result.returncode,0)
        self.assertEqual((root/'.env').read_text(),original)
        self.assertTrue(any('start' in c for c in calls))
        self.assertFalse(any('up' in c for c in calls))

    def test_runtime_failure_recovers_only_if_schema_unchanged(self):
        root,_,result,calls=self.run_update('runtime_failure')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('FERRIX_UPDATE_STAGE=rolled_back',result.stdout)
        self.assertIn('FERRIX_IMAGE='+OLD,(root/'.env').read_text())
        self.assertEqual(sum('up' in c for c in calls),2)

    def test_changed_schema_failure_keeps_backup_without_unsafe_rollback(self):
        root,_,result,calls=self.run_update('migration_failure')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('FERRIX_UPDATE_STAGE=failed',result.stdout)
        self.assertTrue((root/'updates/job/database.dump').is_file())
        self.assertEqual(sum('up' in c for c in calls),1)
        self.assertIn('FERRIX_IMAGE='+NEW,(root/'.env').read_text())

    def test_altered_stack_is_rejected_before_any_replacement(self):
        _,_,result,calls=self.run_update('altered_stack')
        self.assertNotEqual(result.returncode,0)
        self.assertFalse(any(c[0]=='pull' or 'stop' in c or 'up' in c for c in calls))

if __name__=='__main__': unittest.main()
