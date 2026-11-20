#!/usr/bin/env python3
"""Privileged control plane for shared FPM. Never evaluates student PHP code."""
import fcntl
import grp
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import uuid
import traceback

ROOT=Path('/srv/hosting')
RUNTIME=Path('/runtime')
SOCKETS=Path('/run/hosting')
DISABLED='exec,shell_exec,system,passthru,popen,proc_open,pcntl_exec,pcntl_fork,dl,symlink,link,mail'
IDENT=re.compile(r'^[a-f0-9]{32}$')
SAFE=re.compile(r'^[A-Za-z0-9_./-]+$')
PRIVATE=re.compile(r'(?i)\.(?:inc|ini|sql|conf|log|bak|phtml|phar)(?:$|[.~])')
PHP=re.compile(r'(?i)\.php(?:$|[.~])')

def validate(row):
    sid=row['id'];uid=row['uid']
    assert IDENT.fullmatch(sid) and type(uid) is int and 10000<=uid<=60000
    assert row['database']==f'student_{sid[:24]}' and row['username']==f's_{sid[:24]}'
    assert re.fullmatch(r'[a-f0-9]{64}',row['password'])
    return sid,uid

def save(path,value):
    data=json.dumps(value,sort_keys=True).encode()
    if path.exists() and path.read_bytes()==data:return False
    tmp=path.with_suffix('.tmp');tmp.write_bytes(data);tmp.chmod(0o600);os.replace(tmp,path);return True

def account(uid):
    name=f'h{uid}'
    try:assert grp.getgrgid(uid).gr_name==name
    except KeyError:subprocess.run(['groupadd','-g',str(uid),name],check=True,stdout=subprocess.DEVNULL)
    try:assert pwd.getpwuid(uid).pw_name==name
    except KeyError:subprocess.run(['useradd','-u',str(uid),'-g',str(uid),'-M','-s','/usr/sbin/nologin',name],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    return name

def prepare(row):
    sid,uid=validate(row);name=account(uid);home=ROOT/sid
    if home.exists():assert home.is_dir() and not home.is_symlink() and home.stat().st_uid==0
    home.mkdir(mode=0o750,exist_ok=True);os.chown(home,0,uid);home.chmod(0o750)
    for folder in ['public','tmp']:
        target=home/folder
        assert not target.is_symlink()
        if not target.exists():
            target.mkdir(mode=0o750);target.chmod(0o750);os.chown(target,uid,uid)
        else:
            assert target.is_dir() and target.stat().st_uid==uid and target.stat().st_gid==uid
    etc=home/'etc';etc.mkdir(mode=0o755,exist_ok=True)
    for filename in ['resolv.conf','hosts','nsswitch.conf']:
        source=Path('/etc')/filename
        if source.exists():
            target=etc/filename;data=source.read_bytes()
            if not target.exists() or target.read_bytes()!=data:
                temporary=etc/(filename+'.new');temporary.write_bytes(data);temporary.chmod(0o444);os.replace(temporary,target)
    index=home/'public/index.php'
    if not any((home/'public').iterdir()):
        index.write_bytes(Path('/usr/local/share/hosting-index.php').read_bytes());index.chmod(0o640);os.chown(index,uid,uid)
    return home,name

def pool(row):
    sid,uid=validate(row);home,name=prepare(row)
    return f'''[{sid}]
user = {name}
group = {name}
listen = /run/hosting/{sid}.sock
listen.owner = root
listen.group = root
listen.mode = 0600
pm = ondemand
pm.max_children = 2
pm.process_idle_timeout = 10s
pm.max_requests = 200
chroot = {home}
chdir = /public
clear_env = yes
catch_workers_output = yes
decorate_workers_output = no
request_terminate_timeout = 30s
security.limit_extensions = .php
php_admin_value[open_basedir] = /public:/tmp
php_admin_value[disable_functions] = {DISABLED}
php_admin_value[session.save_path] = /tmp
php_admin_value[upload_tmp_dir] = /tmp
php_admin_value[memory_limit] = 64M
php_admin_flag[display_errors] = off
env[DB_HOST] = mariadb
env[DB_NAME] = {row['database']}
env[DB_USER] = {row['username']}
env[DB_PASSWORD] = {row['password']}
env[TMPDIR] = /tmp
'''

def write_pools(rows):
    config=RUNTIME/'pools';config.mkdir(mode=0o700,exist_ok=True)
    desired={r['id'] for r in rows};changed=False
    for path in config.glob('*.conf'):
        if path.stem not in desired:path.unlink();changed=True
    for row in rows:
        content=pool(row);path=config/(row['id']+'.conf')
        if not path.exists() or path.read_text()!=content:
            tmp=path.with_suffix('.tmp');tmp.write_text(content);tmp.chmod(0o600);os.replace(tmp,path);changed=True
    return changed

def safe_path(path):
    return 0<len(path)<=100 and SAFE.fullmatch(path) and all(p and not p.startswith('.') for p in path.split('/'))

def write_files(row,files):
    sid,uid=validate(row);prepare(row)
    # Resolve every component with O_NOFOLLOW. Renames/symlinks created by a
    # student cannot trick a privileged writer into another student's tree.
    root_fd=os.open(ROOT/sid/'public',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        for item in files:
            relative=item['path'];assert safe_path(relative)
            data=bytes.fromhex(item['hex']);parts=relative.split('/');fd=os.dup(root_fd)
            try:
                for part in parts[:-1]:
                    try:os.mkdir(part,0o750,dir_fd=fd);os.chown(part,uid,uid,dir_fd=fd,follow_symlinks=False)
                    except FileExistsError:pass
                    nxt=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);os.close(fd);fd=nxt
                temporary='.upload-'+uuid.uuid4().hex
                file_fd=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o640,dir_fd=fd)
                with os.fdopen(file_fd,'wb') as handle:handle.write(data);os.fchown(handle.fileno(),uid,uid)
                os.replace(temporary,parts[-1],src_dir_fd=fd,dst_dir_fd=fd)
            finally:os.close(fd)
    finally:os.close(root_fd)
    publish(row,force=True)

def file_operation(row, payload):
    sid,uid=validate(row)
    operation=payload['operation'];relative=payload.get('path','')
    assert relative=='' or safe_path(relative)
    fd=os.open(ROOT/sid/'public',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        parts=relative.split('/') if relative else []
        for part in (parts if operation=='list' else parts[:-1]):
            child=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
            os.close(fd);fd=child
        if operation=='list':
            entries=[]
            for name in sorted(os.listdir(fd)):
                if not safe_path((relative+'/' if relative else '')+name):continue
                info=os.stat(name,dir_fd=fd,follow_symlinks=False)
                if not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):continue
                if len(entries)>=1024:raise ValueError('Directory limit exceeded')
                entries.append(dict(name=name,directory=stat.S_ISDIR(info.st_mode),size=info.st_size))
            return dict(entries=entries)
        assert parts
        name=parts[-1]
        if operation=='read':
            data=read_regular(fd,name)
            if len(data)>8*1024*1024:raise ValueError('File exceeds 8 MiB')
            return dict(hex=data.hex())
        if operation=='mkdir':
            os.mkdir(name,0o750,dir_fd=fd)
            os.chown(name,uid,uid,dir_fd=fd,follow_symlinks=False)
        elif operation=='write':
            data=bytes.fromhex(payload['hex']);assert len(data)<=8*1024*1024
            # Refuse symlinks and directories, including existing destinations.
            try:assert stat.S_ISREG(os.stat(name,dir_fd=fd,follow_symlinks=False).st_mode)
            except FileNotFoundError:pass
            temporary='.edit-'+uuid.uuid4().hex
            out=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o640,dir_fd=fd)
            try:
                with os.fdopen(out,'wb') as handle:handle.write(data);os.fchown(handle.fileno(),uid,uid)
                os.replace(temporary,name,src_dir_fd=fd,dst_dir_fd=fd)
            finally:
                try:os.unlink(temporary,dir_fd=fd)
                except FileNotFoundError:pass
        elif operation in ('rename','delete'):
            info=os.stat(name,dir_fd=fd,follow_symlinks=False)
            assert stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)
            if operation=='delete':
                # Only empty folders can be deleted. Never recurse through user trees.
                if stat.S_ISDIR(info.st_mode):os.rmdir(name,dir_fd=fd)
                else:os.unlink(name,dir_fd=fd)
            else:
                target=payload['name'];assert safe_path(target) and '/' not in target
                # Linux renameat2(RENAME_NOREPLACE) prevents overwrite races.
                import ctypes
                libc=ctypes.CDLL(None,use_errno=True)
                if libc.renameat2(fd,os.fsencode(name),fd,os.fsencode(target),1)!=0:
                    raise OSError(ctypes.get_errno(),'Rename failed')
        else:raise ValueError('Unknown file operation')
    finally:os.close(fd)
    publish(row,force=True)
    return dict(ok=True)

def database_operation(row,payload):
    sid,uid=validate(row)
    assert payload['operation'] in ('import','export')
    sql=payload.get('sql','');assert isinstance(sql,str) and len(sql.encode())<=8*1024*1024
    # Fixed trusted PHP code; SQL is data passed to PDO, never CLI commands or PHP.
    # Drop UID/GID and supplementary groups before PHP starts. DB grants remain
    # the authoritative boundary, even for USE, qualified SQL and malicious dumps.
    request=dict(row,operation=payload['operation'],sql=sql)
    result=subprocess.run(['php','-d','display_errors=0','-d','log_errors=0',
        '-d','memory_limit=96M','-d','max_execution_time=30',
        '/usr/local/share/hosting-database.php'],input=json.dumps(request).encode(),
        stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=40,
        user=uid,group=uid,extra_groups=[],cwd='/',env={'PATH':'/usr/local/bin:/usr/bin:/bin'})
    if result.returncode or len(result.stdout)>34*1024*1024:raise ValueError('Database operation failed')
    return json.loads(result.stdout)

def collect(fd,prefix='',result=None):
    result=[] if result is None else result
    for name in sorted(os.listdir(fd)):
        relative=prefix+name
        if not safe_path(relative):continue
        info=os.stat(name,dir_fd=fd,follow_symlinks=False)
        if stat.S_ISDIR(info.st_mode):
            child=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
            try:collect(child,relative+'/',result)
            finally:os.close(child)
        elif stat.S_ISREG(info.st_mode) and not PRIVATE.search(name):
            if len(result)>=1024:raise ValueError('Publishing file limit exceeded')
            result.append((relative,info.st_mtime_ns,info.st_size,bool(PHP.search(name))))
    return result

def read_regular(root_fd,relative):
    fd=os.dup(root_fd)
    try:
        parts=relative.split('/')
        for part in parts[:-1]:
            nxt=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);os.close(fd);fd=nxt
        file_fd=os.open(parts[-1],os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=fd)
        with os.fdopen(file_fd,'rb') as handle:
            if not stat.S_ISREG(os.fstat(handle.fileno()).st_mode):raise ValueError('Not a regular file')
            data=handle.read(32*1024*1024+1)
            if len(data)>32*1024*1024:raise ValueError('Publishing size limit exceeded')
            return data
    finally:os.close(fd)

def publish(row,force=False):
    sid,uid=validate(row);home=ROOT/sid;root_fd=os.open(home/'public',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        files=collect(root_fd);manifest=hashlib.sha256(json.dumps(files).encode()).hexdigest()
        marker=RUNTIME/(sid+'.manifest')
        if not force and marker.exists() and marker.read_text()==manifest:return
        published=home/'.published';published.mkdir(mode=0o700,exist_ok=True)
        stage=Path(tempfile.mkdtemp(dir=published));total=0
        try:
            for relative,_,_,is_php in files:
                target=stage/relative;target.parent.mkdir(parents=True,exist_ok=True)
                # PHP source never reaches Caddy: placeholders support its file
                # matcher; FPM executes the actual script inside the user's jail.
                data=b'' if is_php else read_regular(root_fd,relative)
                total+=len(data)
                if total>32*1024*1024:raise ValueError('Publishing size limit exceeded')
                target.write_bytes(data);target.chmod(0o444)
            next_link=home/'.site-next'
            next_link.unlink(missing_ok=True);next_link.symlink_to(stage.relative_to(home),target_is_directory=True)
            os.replace(next_link,home/'site');marker.write_text(manifest);marker.chmod(0o600)
            for old in published.iterdir():
                if old!=stage:shutil.rmtree(old)
        except Exception:
            shutil.rmtree(stage,ignore_errors=True);raise
    finally:os.close(root_fd)

def kill_uid(uid):
    for proc in Path('/proc').iterdir():
        if proc.name.isdigit():
            try:
                if proc.stat().st_uid==uid:os.kill(int(proc.name),signal.SIGKILL)
            except (FileNotFoundError,ProcessLookupError):pass

def main():
    ROOT.mkdir(mode=0o711,parents=True,exist_ok=True);ROOT.chmod(0o711)
    RUNTIME.mkdir(mode=0o700,parents=True,exist_ok=True);RUNTIME.chmod(0o700)
    SOCKETS.mkdir(mode=0o700,parents=True,exist_ok=True);SOCKETS.chmod(0o700)
    mode=sys.argv[1]
    if mode=='ready':
        assert Path('/proc/1/comm').read_text().strip().startswith('php-fpm') and (SOCKETS/'_health.sock').is_socket()
        return
    with open(RUNTIME/'manager.lock','w') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        state=RUNTIME/'students.json';old=json.loads(state.read_text()) if state.exists() else []
        if mode=='init':
            config='''[global]
daemonize = no
error_log = /proc/self/fd/2
process.max = 16
include = /runtime/pools/*.conf
[health]
user = www-data
group = www-data
listen = /run/hosting/_health.sock
listen.mode = 0600
pm = ondemand
pm.max_children = 1
pm.process_idle_timeout = 5s
'''
            (RUNTIME/'php-fpm.conf').write_text(config)
            write_pools(old)
        else:
            payload=json.load(sys.stdin)
            if mode in ('files','database'):
                row=next(r for r in old if r['id']==payload['id']);validate(row)
                result=file_operation(row,payload) if mode=='files' else database_operation(row,payload)
                print(json.dumps(result));return
            if mode=='upload':
                row=next(r for r in old if r['id']==payload['id']);write_files(row,payload['files']);return
            if mode=='sync':rows=payload['students']
            elif mode=='suspend':rows=[r for r in old if r['id']!=payload['id']]
            else:raise ValueError('Unknown command')
            for row in rows:validate(row)
            desired={r['id'] for r in rows}
            for row in old:
                if row['id'] not in desired:kill_uid(row['uid'])
            changed=write_pools(rows)
            save(state,rows)
            for row in rows:
                try:publish(row)
                except (OSError,ValueError):print('Static publication deferred for one hosting.',file=sys.stderr)
            if changed:os.kill(1,signal.SIGUSR2)
    if mode=='init':os.execvp('php-fpm',['php-fpm','-F','-y',str(RUNTIME/'php-fpm.conf')])

if __name__=='__main__':
    try:main()
    except Exception as error:
        frames=' > '.join(f'{Path(frame.filename).name}:{frame.name}:{frame.lineno}' for frame in traceback.extract_tb(error.__traceback__))
        print(f'Hosting control failed ({type(error).__name__}): {frames}',file=sys.stderr);sys.exit(1)
