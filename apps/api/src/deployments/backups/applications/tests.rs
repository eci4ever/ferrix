use super::*;
use crate::deployments::test_support::*;
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
async fn request(f: &Fixture, service: Uuid, method: &str, suffix: &str, body: Value) -> Reply {
    call(
        &f.app,
        method,
        &format!("/api/v1/services/{service}{suffix}"),
        &f.cookie,
        body,
        &[],
        true,
    )
    .await
}
async fn deployment(f: &Fixture) -> Value {
    for _ in 0..360 {
        let rows = request(f, f.service, "GET", "/deployments", Value::Null)
            .await
            .body;
        if rows[0]["status"] == "succeeded" {
            return rows[0].clone();
        }
        if rows[0]["status"] == "failed" {
            panic!("{}", rows[0]["logs"]);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Deployment timed out")
}
async fn job(f: &Fixture, id: Uuid, status: &str) -> Value {
    for _ in 0..480 {
        let rows = request(f, f.service, "GET", "/backups", Value::Null)
            .await
            .body;
        let found = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id.to_string())
            .cloned();
        if let Some(row) = found {
            if row["status"] == status {
                return row;
            }
            if row["status"] == "failed" && status != "failed" {
                panic!("{}", row["message"]);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Backup job timed out")
}
async fn running(f: &Fixture, service: Uuid) -> bool {
    request(f, service, "GET", "/runtime", Value::Null)
        .await
        .body["state"]
        == "running"
}
const APP: &str = r#"import sqlite3,pathlib,time,signal,sys,os
p=pathlib.Path('/opt/data')
c=sqlite3.connect(p/'state.db');c.execute('PRAGMA journal_mode=WAL');c.execute('CREATE TABLE IF NOT EXISTS state(id integer primary key,value text)');c.execute("INSERT OR IGNORE INTO state VALUES(1,'original')");c.commit()
for name in ['.bot-config','sessions-wal','sessions-shm']:
 f=p/name
 if not f.exists(): f.write_text('disposable-test-state')
 os.chown(f,123,456);os.chmod(f,0o600)
def stop(*args):
 c.close();sys.exit(0)
signal.signal(signal.SIGTERM,stop)
while True: time.sleep(1)
"#;
async fn configure(f: &Fixture) {
    let config = json!({"name":"Backup worker","image":IMAGE,"container_port":80,"worker_mode":true,"memory_mib":768,"cpus":0.75,"command":["python3","-c",APP],"storage":{"target":"/opt/data","read_only":false}});
    assert_eq!(
        request(f, f.service, "POST", "/configuration", config)
            .await
            .status,
        StatusCode::OK
    );
}
async fn cleanup(f: Fixture, services: Vec<Uuid>) {
    for service in services {
        let filters = HashMap::from([(
            "label".to_owned(),
            vec![format!("io.ferrix.application={service}")],
        )]);
        if let Ok(rows) = f
            .state
            .docker
            .list_containers(Some(
                bollard::query_parameters::ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
        {
            for row in rows {
                if let Some(id) = row.id {
                    let _ = f
                        .state
                        .docker
                        .remove_container(
                            &id,
                            Some(
                                RemoveContainerOptionsBuilder::default()
                                    .force(true)
                                    .v(true)
                                    .build(),
                            ),
                        )
                        .await;
                }
            }
        }
        for name in [application_runtime::volume(service), volume_name(service)] {
            let _ = f
                .state
                .docker
                .remove_volume(
                    &name,
                    None::<bollard::query_parameters::RemoveVolumeOptions>,
                )
                .await;
        }
    }
    f.finish().await;
}
#[tokio::test]
#[ignore = "requires PostgreSQL and Docker"]
async fn scheduled_offline_volume_backups_restore_sqlite_and_preserve_runtime_state() {
    let f = Fixture::new().await;
    let mut targets = vec![f.service];
    let (stop, rx) = watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let result=AssertUnwindSafe(async {
        configure(&f).await;
        assert_eq!(request(&f,f.service,"POST","/deploy",json!({})).await.status,StatusCode::ACCEPTED);
        let initial=deployment(&f).await;
        assert_eq!(request(&f,f.service,"POST","/backups",json!({})).await.status,StatusCode::UNPROCESSABLE_ENTITY);
        let mut policy=json!({"enabled":true,"cron":"0 2 * * *","timezone":"Asia/Kuala_Lumpur","keep_last":2,"s3":null,"application_offline":false});
        assert_eq!(request(&f,f.service,"POST","/backup-policy",policy.clone()).await.status,StatusCode::UNPROCESSABLE_ENTITY);
        policy["application_offline"]=json!(true);
        assert_eq!(request(&f,f.service,"POST","/backup-policy",policy.clone()).await.status,StatusCode::NO_CONTENT);
        sqlx::query("UPDATE backup_policies SET next_run_at=now()-interval '1 minute' WHERE application_id=$1").bind(f.service).execute(&f.state.pool).await.unwrap();
        let mut scheduled=None;
        for _ in 0..240 {
            let id:Option<Uuid>=sqlx::query_scalar("SELECT id FROM database_jobs WHERE application_id=$1 AND scheduled_for IS NOT NULL").bind(f.service).fetch_optional(&f.state.pool).await.unwrap();
            if let Some(id)=id {scheduled=Some(id);break;}
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let scheduled=scheduled.expect("Scheduled backup did not queue");
        let manifest=job(&f,scheduled,"succeeded").await;
        assert!(manifest["size_bytes"].as_i64().unwrap()>0);assert_eq!(manifest["sha256"].as_str().unwrap().len(),64);assert!(running(&f,f.service).await);
        assert_eq!(request(&f,f.service,"POST","/control",json!({"operation":"stop"})).await.status,StatusCode::NO_CONTENT);
        let manual=request(&f,f.service,"POST","/backups",json!({"offline":true})).await;
        assert_eq!(manual.status,StatusCode::ACCEPTED);
        assert_eq!(request(&f,f.service,"POST","/deploy",json!({})).await.status,StatusCode::CONFLICT);
        assert_eq!(request(&f,f.service,"POST","/rollback",json!({"deployment_id":initial["id"]})).await.status,StatusCode::CONFLICT);
        assert_eq!(request(&f,f.service,"POST","/control",json!({"operation":"start"})).await.status,StatusCode::CONFLICT);
        job(&f,manual.body["id"].as_str().unwrap().parse().unwrap(),"succeeded").await;
        assert!(!running(&f,f.service).await);
        let restored=request(&f,f.service,"POST",&format!("/backups/{scheduled}/restore"),json!({"name":"Restored worker"})).await;
        assert_eq!(restored.status,StatusCode::ACCEPTED,"{}",restored.body);
        let target:Uuid=restored.body["application_id"].as_str().unwrap().parse().unwrap();targets.push(target);
        job(&f,restored.body["id"].as_str().unwrap().parse().unwrap(),"succeeded").await;
        assert!(!running(&f,target).await);
        let target_container=request(&f,target,"GET","/runtime",Value::Null).await.body["container_id"].as_str().unwrap().to_owned();
        let prepared=f.state.docker.inspect_container(&target_container,None).await.unwrap();
        assert!(prepared.state.as_ref().and_then(|s|s.started_at.as_deref()).is_none_or(|s|s.starts_with("0001") || s.starts_with("1970")));
        let host=prepared.host_config.unwrap();assert_eq!(host.memory,Some(768*1024*1024));assert_eq!(host.nano_cpus,Some(750_000_000));
        let target_volume=application_runtime::volume(target);
        let verify=r#"import sqlite3,pathlib,os,stat
p=pathlib.Path('/data');c=sqlite3.connect('file:/data/state.db?mode=rw',uri=True)
assert c.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
assert c.execute('SELECT value FROM state WHERE id=1').fetchone()[0]=='original'
for name in ['.bot-config','sessions-wal','sessions-shm']:
 f=p/name;assert f.read_text()=='disposable-test-state';s=f.stat();assert (s.st_uid,s.st_gid,stat.S_IMODE(s.st_mode))==(123,456,0o600)
"#;
        let output=tokio::process::Command::new("docker").args(["run","--rm","--network","none","--mount",&format!("type=volume,src={target_volume},dst=/data"),IMAGE,"python3","-c",verify]).output().await.unwrap();assert!(output.status.success(),"Archive fixture verification failed: {}",String::from_utf8_lossy(&output.stderr));
        let (source_start,target_start)=tokio::join!(request(&f,f.service,"POST","/control",json!({"operation":"start"})),request(&f,target,"POST","/control",json!({"operation":"start"})));
        let mut outcomes=[source_start.status.as_u16(),target_start.status.as_u16()];outcomes.sort();assert_eq!(outcomes,[204,409]);
        let winner=if source_start.status==StatusCode::NO_CONTENT {f.service} else {target};
        assert_eq!(request(&f,winner,"POST","/control",json!({"operation":"stop"})).await.status,StatusCode::NO_CONTENT);
        assert_eq!(request(&f,f.service,"POST","/control",json!({"operation":"start"})).await.status,StatusCode::NO_CONTENT);
        assert_eq!(request(&f,target,"POST","/control",json!({"operation":"start"})).await.status,StatusCode::CONFLICT);
        assert_eq!(request(&f,f.service,"POST","/control",json!({"operation":"stop"})).await.status,StatusCode::NO_CONTENT);
        assert_eq!(request(&f,target,"POST","/control",json!({"operation":"start"})).await.status,StatusCode::NO_CONTENT);
        assert!(running(&f,target).await);assert!(!running(&f,f.service).await);
        // Corrupt only disposable checksum metadata: failed restore stays blocked/stopped.
        let original=manifest["sha256"].as_str().unwrap();
        sqlx::query("UPDATE database_jobs SET sha256=$2 WHERE id=$1").bind(scheduled).bind("0".repeat(64)).execute(&f.state.pool).await.unwrap();
        let failed=request(&f,f.service,"POST",&format!("/backups/{scheduled}/restore"),json!({"name":"Corrupt restore"})).await;assert_eq!(failed.status,StatusCode::ACCEPTED);
        let bad:Uuid=failed.body["application_id"].as_str().unwrap().parse().unwrap();targets.push(bad);
        job(&f,failed.body["id"].as_str().unwrap().parse().unwrap(),"failed").await;
        assert_eq!(request(&f,bad,"POST","/control",json!({"operation":"start"})).await.status,StatusCode::CONFLICT);
        assert_eq!(request(&f,bad,"POST","/deploy",json!({})).await.status,StatusCode::CONFLICT);
        assert!(!running(&f,bad).await);
        sqlx::query("UPDATE database_jobs SET sha256=$2 WHERE id=$1").bind(scheduled).bind(original).execute(&f.state.pool).await.unwrap();
        // Retention protects only eligible completed backups, not Application data.
        policy["keep_last"]=json!(1);policy["enabled"]=json!(false);
        assert_eq!(request(&f,f.service,"POST","/backup-policy",policy).await.status,StatusCode::NO_CONTENT);
        let next=request(&f,f.service,"POST","/backups",json!({"offline":true})).await;assert_eq!(next.status,StatusCode::ACCEPTED);
        job(&f,next.body["id"].as_str().unwrap().parse().unwrap(),"succeeded").await;
        for _ in 0..120 {if sqlx::query_scalar::<_,bool>("SELECT pruned_at IS NOT NULL FROM database_jobs WHERE id=$1").bind(scheduled).fetch_one(&f.state.pool).await.unwrap(){break;}tokio::time::sleep(Duration::from_millis(250)).await;}
        assert!(sqlx::query_scalar::<_,bool>("SELECT pruned_at IS NOT NULL FROM database_jobs WHERE id=$1").bind(scheduled).fetch_one(&f.state.pool).await.unwrap());
        assert_eq!(request(&f,f.service,"POST",&format!("/backups/{scheduled}/restore"),json!({"name":"Expired"})).await.status,StatusCode::NOT_FOUND);
        assert!(f.state.docker.inspect_volume(&application_runtime::volume(f.service)).await.is_ok());
        sqlx::query("UPDATE auth_members SET role='member' WHERE user_id=$1 AND organization_id=$2").bind(f.user).bind(f.org).execute(&f.state.pool).await.unwrap();
        assert_eq!(request(&f,f.service,"POST","/backups",json!({"offline":true})).await.status,StatusCode::FORBIDDEN);
        assert_eq!(request(&f,f.service,"POST",&format!("/backups/{}/restore",next.body["id"].as_str().unwrap()),json!({"name":"Unauthorized restore"})).await.status,StatusCode::FORBIDDEN);
    }).catch_unwind().await;
    stop.send(true).unwrap();
    worker.await.unwrap();
    cleanup(f, targets).await;
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Docker"]
async fn interrupted_offline_backup_recovers_helpers_and_original_runtime() {
    let f = Fixture::new().await;
    let result=AssertUnwindSafe(async {
        configure(&f).await;
        let queued=request(&f,f.service,"POST","/deploy",json!({})).await;
        let deployment:Uuid=queued.body["id"].as_str().unwrap().parse().unwrap();
        sqlx::query("UPDATE deployments SET status='running' WHERE id=$1").bind(deployment).execute(&f.state.pool).await.unwrap();
        let snapshot:worker::Job=sqlx::query_as("SELECT d.id,d.application_id,d.image,d.pinned_image_id,d.source_cipher,d.start_stopped,d.container_port,d.env_cipher,d.service_kind,d.service_cipher,p.organization_id,p.id AS project_id,e.id AS environment_id FROM deployments d JOIN applications a ON a.id=d.application_id JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE d.id=$1").bind(deployment).fetch_one(&f.state.pool).await.unwrap();
        let mut conn=f.state.pool.acquire().await.unwrap();worker::execute(&f.state,&mut conn,&snapshot).await.unwrap();drop(conn);
        let container=request(&f,f.service,"GET","/runtime",Value::Null).await.body["container_id"].as_str().unwrap().to_owned();
        for was_running in [true,false] {
            if !was_running {
                assert_eq!(request(&f,f.service,"POST","/control",json!({"operation":"stop"})).await.status,StatusCode::NO_CONTENT);
                f.state.docker.update_container(&container,bollard::models::ContainerUpdateBody{restart_policy:Some(RestartPolicy{name:Some(RestartPolicyNameEnum::NO),maximum_retry_count:None}),..Default::default()}).await.unwrap();
            }
            let restart=f.state.docker.inspect_container(&container,None).await.unwrap().host_config.unwrap().restart_policy.unwrap();
            let queued=request(&f,f.service,"POST","/backups",json!({"offline":true})).await;assert_eq!(queued.status,StatusCode::ACCEPTED);
            let id:Uuid=queued.body["id"].as_str().unwrap().parse().unwrap();
            sqlx::query("UPDATE database_jobs SET status='running',resume_container_id=$2,resume_was_running=$3,resume_needed=true,resume_restart_policy=$4 WHERE id=$1").bind(id).bind(&container).bind(was_running).bind(serde_json::to_string(&restart).unwrap()).execute(&f.state.pool).await.unwrap();
            f.state.docker.update_container(&container,bollard::models::ContainerUpdateBody{restart_policy:Some(RestartPolicy{name:Some(RestartPolicyNameEnum::NO),maximum_retry_count:None}),..Default::default()}).await.unwrap();
            match f.state.docker.stop_container(&container,Some(StopContainerOptionsBuilder::default().t(30).build())).await {Ok(())|Err(bollard::errors::Error::DockerResponseServerError{status_code:304,..})=>{},e=>panic!("{e:?}")}
            let helper=helper_name(id);
            f.state.docker.create_container(Some(CreateContainerOptionsBuilder::default().name(&helper).build()),ContainerCreateBody{image:Some(IMAGE.into()),cmd:Some(vec!["python3".into(),"-c".into(),"import time;time.sleep(1000)".into()]),labels:Some(HashMap::from([("io.ferrix.managed".into(),"true".into()),("io.ferrix.application".into(),f.service.to_string()),("io.ferrix.backup-job".into(),id.to_string())])),host_config:Some(HostConfig{network_mode:Some("none".into()),mounts:Some(vec![Mount{typ:Some(MountTypeEnum::VOLUME),source:Some(application_runtime::volume(f.service)),target:Some("/data".into()),read_only:Some(true),..Default::default()}]),..Default::default()}),..Default::default()}).await.unwrap();
            f.state.docker.start_container(&helper,None).await.unwrap();
            let (stop,rx)=watch::channel(false);let worker=tokio::spawn(crate::deployments::run_worker(f.state.clone(),rx));
            let recovered=job(&f,id,"failed").await;
            stop.send(true).unwrap();worker.await.unwrap();
            assert!(recovered["message"].as_str().unwrap().contains("interrupted"));
            assert_eq!(running(&f,f.service).await,was_running);
            assert!(f.state.docker.inspect_container(&helper,None).await.is_err());
            let current=f.state.docker.inspect_container(&container,None).await.unwrap().host_config.unwrap().restart_policy.unwrap();assert_eq!(current,restart);
        }
        // An unsupported filesystem entry fails safely and resumes a running source.
        assert_eq!(request(&f,f.service,"POST","/control",json!({"operation":"start"})).await.status,StatusCode::NO_CONTENT);
        let out=tokio::process::Command::new("docker").args(["exec",&container,"python3","-c","import os;os.symlink('/etc/passwd','/opt/data/unsupported-link')"]).output().await.unwrap();assert!(out.status.success());
        let queued=request(&f,f.service,"POST","/backups",json!({"offline":true})).await;
        let id:Uuid=queued.body["id"].as_str().unwrap().parse().unwrap();
        let (stop,rx)=watch::channel(false);let worker=tokio::spawn(crate::deployments::run_worker(f.state.clone(),rx));
        let failed=job(&f,id,"failed").await;stop.send(true).unwrap();worker.await.unwrap();
        assert!(failed["message"].as_str().unwrap().contains("archive failed"));assert!(running(&f,f.service).await);
    }).catch_unwind().await;
    let service = f.service;
    cleanup(f, vec![service]).await;
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}

#[tokio::test]
#[ignore = "requires the disposable S3 runner"]
async fn application_volume_s3_roundtrip_without_local_backup_storage() {
    let Ok(endpoint) = std::env::var("FERRIX_TEST_S3_ENDPOINT") else {
        return;
    };
    assert_eq!(
        std::env::var("FERRIX_TEST_S3_DISPOSABLE").as_deref(),
        Ok("1")
    );
    assert_eq!(
        url::Url::parse(&endpoint).unwrap().host_str(),
        Some("127.0.0.1")
    );
    let f = Fixture::new().await;
    let mut targets = vec![f.service];
    let (stop, rx) = watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let result=AssertUnwindSafe(async {
        configure(&f).await;request(&f,f.service,"POST","/deploy",json!({})).await;deployment(&f).await;
        let policy=json!({"enabled":false,"cron":"0 2 * * *","timezone":"UTC","keep_last":2,"application_offline":true,"s3":{"endpoint":endpoint,"region":"us-east-1","bucket":std::env::var("FERRIX_TEST_S3_BUCKET").unwrap(),"prefix":format!("app-test-{}",f.service.simple()),"path_style":true,"credentials":{"access_key":std::env::var("FERRIX_TEST_S3_ACCESS_KEY").unwrap(),"secret_key":std::env::var("FERRIX_TEST_S3_SECRET_KEY").unwrap(),"session_token":null}}});
        assert_eq!(request(&f,f.service,"POST","/backup-policy",policy).await.status,StatusCode::NO_CONTENT);
        let queued=request(&f,f.service,"POST","/backups",json!({"offline":true})).await;let id:Uuid=queued.body["id"].as_str().unwrap().parse().unwrap();
        let manifest=job(&f,id,"succeeded").await;assert_eq!(manifest["storage"],"s3");assert!(running(&f,f.service).await);
        let name=volume_name(f.service);let info=f.state.docker.inspect_volume(&name).await.unwrap();assert_eq!(info.labels["io.ferrix.application"],f.service.to_string());f.state.docker.remove_volume(&name,None::<bollard::query_parameters::RemoveVolumeOptions>).await.unwrap();
        let restored=request(&f,f.service,"POST",&format!("/backups/{id}/restore"),json!({"name":"S3 restored Application"})).await;assert_eq!(restored.status,StatusCode::ACCEPTED);
        let target:Uuid=restored.body["application_id"].as_str().unwrap().parse().unwrap();targets.push(target);
        job(&f,restored.body["id"].as_str().unwrap().parse().unwrap(),"succeeded").await;assert!(!running(&f,target).await);
        let verify="import sqlite3;c=sqlite3.connect('/data/state.db');assert c.execute('PRAGMA integrity_check').fetchone()[0]=='ok';assert c.execute('SELECT value FROM state').fetchone()[0]=='original'";
        let output=tokio::process::Command::new("docker").args(["run","--rm","--network","none","--mount",&format!("type=volume,src={},dst=/data",application_runtime::volume(target)),IMAGE,"python3","-c",verify]).output().await.unwrap();assert!(output.status.success());
    }).catch_unwind().await;
    stop.send(true).unwrap();
    worker.await.unwrap();
    cleanup(f, targets).await;
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}
