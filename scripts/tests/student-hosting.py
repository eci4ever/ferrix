#!/usr/bin/env python3
"""Exercise real student registration, approval, shared MariaDB and PHP in disposable resources."""
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

root=Path(__file__).resolve().parents[2]
profile=dict(line.split('=',1) for line in (root/'.env').read_text().splitlines() if '=' in line and not line.startswith('#'))
parsed=urllib.parse.urlsplit(profile['DATABASE_URL'])
assert parsed.hostname in ('127.0.0.1','localhost') and parsed.port==int(profile['POSTGRES_PORT']) and parsed.path=='/'+profile['POSTGRES_DB']
env=os.environ|profile
schema='students_test_'+uuid.uuid4().hex
network=schema+'_ingress'
proxy=schema+'_proxy'
compose=['docker','compose','exec','-T','postgres','psql','-v','ON_ERROR_STOP=1','-U','ferrix','-d',profile['POSTGRES_DB'],'-Atc']
def sql(query):return subprocess.check_output(compose+[query],text=True,env=env).strip()
def port():
 with socket.socket() as sock:
  sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
api_port,public_port,edge_port,web_port=port(),port(),port(),port()
api_origin=f'http://127.0.0.1:{api_port}'
origin=f'http://127.0.0.1:{web_port}'
scope=None
def client():return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
admin=client();alice=client();bob=client()
def api(who,path,data=None,expected=200):
 req=urllib.request.Request(api_origin+'/api/v1'+path,headers={'Origin':origin,'Content-Type':'application/json'},data=None if data is None else json.dumps(data).encode())
 try:r=who.open(req,timeout=70)
 except urllib.error.HTTPError as e:r=e
 body=r.read();assert r.status==expected,(path,r.status,body[:400])
 return json.loads(body) if body else None
def site(user,path='/'):
 req=urllib.request.Request(f'http://127.0.0.1:{edge_port}{path}',headers={'Host':f'student-{uuid.UUID(user).hex}.localhost'})
 try:return urllib.request.urlopen(req,timeout=10)
 except urllib.error.HTTPError as e:return e

def ready(user):
 for _ in range(180):
  row=api(admin,f'/student-hosting/{user}')
  if row['status']=='failed':raise AssertionError(row)
  if row['status']=='active':return row
  time.sleep(1)
 raise AssertionError('Hosting provisioning timed out')

process=web=None
sql(f'CREATE SCHEMA {schema}')
try:
 subprocess.run(['cargo','build','-p','ferrix-api'],check=True,env=env)
 subprocess.run(['docker','network','create','--label',f'io.ferrix.ingress={network}',network],check=True,env=env,stdout=subprocess.DEVNULL)
 query=urllib.parse.parse_qsl(parsed.query)+[('options','-csearch_path='+schema)]
 runtime=env|{'DATABASE_URL':urllib.parse.urlunsplit(parsed._replace(query=urllib.parse.urlencode(query))),'FERRIX_WEB_PORT':str(web_port),'FERRIX_AUTH_ORIGIN':origin,'FERRIX_AUTH_KEY':secrets.token_hex(32),'FERRIX_BIND':f'127.0.0.1:{api_port}','FERRIX_WEBSITE_BIND':f'127.0.0.1:{public_port}','FERRIX_AUTO_DOMAIN':'localhost','FERRIX_WEBSITE_HTTPS':'false','FERRIX_WEBSITE_HTTP_PORT':str(edge_port),'FERRIX_INGRESS_NETWORK':network,'FERRIX_STUDENT_PHP_IMAGE':'ferrix-student-php:local','FERRIX_PUBLIC_IP':''}
 with open('/tmp/ferrix-student-hosting-test.log','w+b') as log:
  process=subprocess.Popen([str(root/'target/debug/ferrix-api')],cwd=root,env=runtime,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
  for _ in range(100):
   try:urllib.request.urlopen(api_origin+'/health',timeout=1).close();break
   except OSError:time.sleep(.2)
  scope=sql(f"SELECT replace(instance_id::text,'-','') FROM {schema}.student_hosting_settings")
  subprocess.run(['docker','run','-d','--name',proxy,'--network',network,'-p',f'127.0.0.1:{edge_port}:80','-e',f'FERRIX_WEBSITE_UPSTREAM=host.docker.internal:{public_port}','-v',f'{root}/deploy/Caddyfile.local:/etc/caddy/Caddyfile:ro','caddy:2-alpine'],check=True,env=env,stdout=subprocess.DEVNULL)
  teacher_password=secrets.token_urlsafe(24)
  api(admin,'/auth/setup',{'name':'Teacher','email':'teacher@example.test','password':teacher_password})
  password=secrets.token_urlsafe(24)
  api(alice,'/auth/student-register',{'name':'Alice','email':'alice@example.test','password':password},403)
  api(admin,'/student-hosting/settings',{'registration_open':True},204)
  api(client(),'/auth/student-register',{'name':'Not teacher','email':'teacher@example.test','password':password},202)
  assert api(admin,'/auth/session')['hosting_student'] is False
  for who,name in [(alice,'Alice'),(bob,'Bob')]:
   api(who,'/auth/student-register',{'name':name,'email':name.lower()+'@example.test','password':password},202)
   api(who,'/auth/sign-in',{'email':name.lower()+'@example.test','password':password})
  aid=api(alice,'/auth/session')['user_id'];bid=api(bob,'/auth/session')['user_id']
  assert api(alice,'/auth/session')['hosting_student'] is True
  for path in ['/projects','/containers','/home-summary','/auth/members','/auth/admin/users']:
   api(alice,path,expected=403)
  api(alice,'/auth/organizations/switch',{'organization_id':api(admin,'/auth/session')['organization_id']},403)
  api(alice,f'/student-hosting/{bid}',expected=404)
  api(alice,'/student-hosting/settings',expected=403)
  api(alice,f'/student-hosting/{aid}/approve',{},403)
  assert api(alice,'/student-hosting')[0]['status']=='pending'
  api(alice,f'/student-hosting/{aid}/upload',{'files':[{'path':'index.php','hex':'00'}]},409)
  for user in [aid,bid]:api(admin,f'/student-hosting/{user}/approve',{},202)
  a=ready(aid);b=ready(bid)
  assert a['database_host']==b['database_host'] and a['database_user']!=b['database_user'] and a['database_name']!=b['database_name']
  code='''<?php $pdo=new PDO('mysql:host='.getenv('DB_HOST').';dbname='.getenv('DB_NAME'),getenv('DB_USER'),getenv('DB_PASSWORD'),[PDO::ATTR_ERRMODE=>PDO::ERRMODE_EXCEPTION]);$pdo->exec('CREATE TABLE IF NOT EXISTS notes(id INT PRIMARY KEY, value VARCHAR(50))');$pdo->exec("INSERT IGNORE INTO notes VALUES(1,'persistent')");echo $pdo->query('SELECT value FROM notes WHERE id=1')->fetchColumn();try{$pdo->exec('USE `'''+b['database_name']+'''`');http_response_code(500);echo 'LEAK';}catch(PDOException $e){if(($e->errorInfo[1]??0)!==1044){http_response_code(500);}echo ' ISOLATED';} ?>'''
  api(alice,f'/student-hosting/{aid}/upload',{'files':[{'path':'index.php','hex':code.encode().hex()},{'path':'assets/test.txt','hex':b'asset'.hex()}]},204)
  with site(aid) as r:assert r.status==200 and r.read()==b'persistent ISOLATED'
  with site(aid,'/assets/test.txt') as r:assert r.read()==b'asset'
  api(alice,f'/student-hosting/{bid}/upload',{'files':[{'path':'index.php','hex':code.encode().hex()}]},404)
  for path in ['../escape.php','/absolute.php','.env','nested/.hidden','a/../../b.php']:
   api(alice,f'/student-hosting/{aid}/upload',{'files':[{'path':path,'hex':'00'}]},422)
  assert b'database_password' not in json.dumps(api(alice,'/student-hosting')).encode()
  own=api(alice,f'/student-hosting/{aid}');assert own['database_password']
  assert 'password_cipher' not in own
  api(admin,f'/student-hosting/{aid}/suspend',{},204)
  assert site(aid).status==404
  api(alice,f'/student-hosting/{aid}/upload',{'files':[{'path':'index.php','hex':'00'}]},409)
  api(admin,f'/student-hosting/{aid}/approve',{},202);ready(aid)
  with site(aid) as r:assert r.read()==b'persistent ISOLATED'
  web=subprocess.Popen(['npm','run','dev'],cwd=root/'apps/web',env=runtime,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
  for _ in range(100):
   try:urllib.request.urlopen(origin,timeout=1).close();break
   except OSError:time.sleep(.2)
  subprocess.run(['node',str(root/'scripts/tests/student-hosting-browser.mjs')],cwd=root,env=runtime|{'STUDENT_TEST_ORIGIN':origin,'STUDENT_TEST_PASSWORD':password,'STUDENT_TEST_TEACHER_PASSWORD':teacher_password},check=True)
  api(admin,'/student-hosting/settings',{'registration_open':False},204)
  api(client(),'/auth/student-register',{'name':'Closed','email':'closed@example.test','password':password},403)
  print('Passed: pending signup/approval, one shared MariaDB, two PHP sites, private grants, blocked platform/peer APIs, safe upload, suspend/resume and persistent files/data.')
finally:
 if web and web.poll() is None:
  os.killpg(web.pid,signal.SIGTERM)
  try:web.wait(timeout=15)
  except subprocess.TimeoutExpired:os.killpg(web.pid,signal.SIGKILL);web.wait()
 if process and process.poll() is None:
  os.killpg(process.pid,signal.SIGTERM)
  try:process.wait(timeout=20)
  except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);process.wait()
 subprocess.run(['docker','rm','-f',proxy],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 for kind in ['container','volume','network']:
  command=['docker','ps','-aq'] if kind=='container' else ['docker',kind,'ls','-q']
  ids=subprocess.check_output(command+['--filter',f'label=io.ferrix.hosting={scope}'],env=env,text=True).split() if scope else []
  if ids:subprocess.run(['docker','rm','-f',*ids] if kind=='container' else ['docker',kind,'rm',*ids],env=env,check=True,stdout=subprocess.DEVNULL)
 subprocess.run(['docker','network','rm',network],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 sql(f'DROP SCHEMA {schema} CASCADE')
