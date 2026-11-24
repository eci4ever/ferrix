#!/usr/bin/env python3
"""Exercise PostgreSQL/MariaDB/Application S3 backup and restore using disposable RustFS."""
import os
from pathlib import Path
import secrets
import socket
import subprocess
import time
import urllib.request
import uuid
root=Path(__file__).resolve().parents[2]
profile={}
for line in (root/'.env').read_text().splitlines():
    if '=' in line and not line.lstrip().startswith('#'):
        key,value=line.split('=',1);profile[key.strip()]=value.strip().strip('"').strip("'")
env=os.environ|profile
assert profile['POSTGRES_DB']=='ferrix_nmfairus','Use the current nmfairus development profile.'
with socket.socket() as sock:
    sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
identifier=uuid.uuid4().hex
name='ferrix-nmfairus-test-s3-'+identifier
key='test'+secrets.token_hex(12);secret=secrets.token_hex(24)
env|={'RUSTFS_ACCESS_KEY':key,'RUSTFS_SECRET_KEY':secret,'FERRIX_TEST_DATABASE_URL':profile['DATABASE_URL'],'FERRIX_TEST_S3_ENDPOINT':f'http://127.0.0.1:{port}','FERRIX_TEST_S3_BUCKET':'ferrix-test-'+identifier,'FERRIX_TEST_S3_ACCESS_KEY':key,'FERRIX_TEST_S3_SECRET_KEY':secret,'FERRIX_TEST_S3_DISPOSABLE':'1'}
try:
    subprocess.run(['docker','run','-d','--name',name,'--label','io.ferrix.test='+identifier,'-p',f'127.0.0.1:{port}:9000','-e','RUSTFS_ACCESS_KEY','-e','RUSTFS_SECRET_KEY','rustfs/rustfs:latest','/data'],env=env,cwd=root,check=True,stdout=subprocess.DEVNULL)
    for attempt in range(60):
        try:
            with urllib.request.urlopen(env['FERRIX_TEST_S3_ENDPOINT']+'/health',timeout=1) as response:
                assert response.status==200
            break
        except (OSError,AssertionError):
            if attempt==59:raise RuntimeError('Disposable RustFS did not become ready.')
            time.sleep(1)
    for target in ['s3_live','deployment']:
        subprocess.run(['cargo','test','--test',target,'--','--include-ignored'],env=env,cwd=root,check=True)
    subprocess.run(['cargo','test','--lib','application_volume_s3_roundtrip_without_local_backup_storage','--','--include-ignored'],env=env,cwd=root,check=True)
finally:
    owned=subprocess.run(['docker','inspect','--format','{{ index .Config.Labels "io.ferrix.test" }}',name],env=env,cwd=root,capture_output=True,text=True)
    if owned.returncode==0 and owned.stdout.strip()==identifier:
        subprocess.run(['docker','rm','-fv',name],env=env,cwd=root,check=True,stdout=subprocess.DEVNULL)
print('PostgreSQL, MariaDB and Application volumes restored from S3 after local backup storage removal; disposable S3 removed.')
