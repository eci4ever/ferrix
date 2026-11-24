use super::*;
use crate::deployments::test_support::*;
use futures_util::FutureExt;
use std::{panic::AssertUnwindSafe, time::Duration};
async fn request(f: &Fixture, method: &str, suffix: &str, value: Value) -> Reply {
    call(
        &f.app,
        method,
        &format!("/api/v1/services/{}{suffix}", f.service),
        &f.cookie,
        value,
        &[],
        true,
    )
    .await
}
async fn wait(f: &Fixture, status: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..360 {
        let rows = request(f, "GET", "/deployments", Value::Null).await.body;
        last = rows.clone();
        if rows[0]["status"] == status {
            return rows[0].clone();
        }
        if rows[0]["status"] == "failed" && status != "failed" {
            panic!("{}", rows[0]["logs"]);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Deployment timed out: {last}")
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker"]
async fn storage_workers_redeploy_failure_rollback_and_deletion() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let f = Fixture::new().await;
    let (shutdown, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let (observe_stop, mut observe_rx) = tokio::sync::watch::channel(false);
    let docker = f.state.docker.clone();
    let watched_volume = volume(f.service);
    let observer = tokio::spawn(async move {
        let mut max = 0;
        loop {
            let filters = HashMap::from([("volume".to_owned(), vec![watched_volume.clone()])]);
            let rows = docker
                .list_containers(Some(
                    bollard::query_parameters::ListContainersOptionsBuilder::default()
                        .filters(&filters)
                        .build(),
                ))
                .await
                .unwrap();
            max = max.max(rows.len());
            tokio::select! {_=tokio::time::sleep(Duration::from_millis(40))=>{},_=observe_rx.changed()=>break}
        }
        max
    });
    let result=AssertUnwindSafe(async {
        let command=json!(["sh","-c","test -f /opt/data/identity || echo original-identity > /opt/data/identity; trap 'exit 0' TERM INT; while :; do sleep 1 & wait $!; done"]);
        let config=json!({"name":"Worker storage","image":"nginx:alpine","container_port":80,"environment":"HERMES_HOME=/opt/data","worker_mode":true,"command":command,"storage":{"target":"/opt/data","read_only":false}});
        assert_eq!(request(&f,"POST","/configuration",config.clone()).await.status,StatusCode::OK);
        let mut invalid=config.clone(); invalid["storage"]["target"]=json!("/var/run/docker.sock");
        assert_eq!(request(&f,"POST","/configuration",invalid).await.status,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(request(&f,"POST","/deploy",json!({})).await.status,StatusCode::ACCEPTED);
        let first=wait(&f,"succeeded").await;
        let first_container=first["container_id"].as_str().unwrap().to_owned();
        let volume_name=volume(f.service);
        let volume_info=f.state.docker.inspect_volume(&volume_name).await.unwrap();
        assert_eq!(volume_info.labels["io.ferrix.application"],f.service.to_string());
        let info=f.state.docker.inspect_container(&first_container,None).await.unwrap();
        assert!(info.host_config.unwrap().port_bindings.unwrap_or_default().is_empty());
        assert!(request(&f,"GET","/runtime",Value::Null).await.body["host_port"].is_null());
        // User data and uid/gid survive each replacement. No initializer or chown is run.
        let output=tokio::process::Command::new("docker").args(["exec",&first_container,"sh","-c","chown 123:456 /opt/data/identity; chmod 600 /opt/data/identity"]).output().await.unwrap(); assert!(output.status.success());
        assert_eq!(request(&f,"POST","/deploy",json!({})).await.status,StatusCode::ACCEPTED);
        let second=wait(&f,"succeeded").await;
        assert_ne!(second["id"],first["id"]);
        let second_container=second["container_id"].as_str().unwrap().to_owned();
        let output=tokio::process::Command::new("docker").args(["exec",&second_container,"sh","-c","test \"$(cat /opt/data/identity)\" = original-identity && test \"$(stat -c %u:%g /opt/data/identity)\" = 123:456"]).output().await.unwrap(); assert!(output.status.success());
        let mut moved=config.clone();moved["storage"]["target"]=json!("/different");
        assert_eq!(request(&f,"POST","/configuration",moved).await.status,StatusCode::UNPROCESSABLE_ENTITY);
        let mut failing=config.clone();failing["command"]=json!(["sh","-c","exit 7"]);
        assert_eq!(request(&f,"POST","/configuration",failing).await.status,StatusCode::OK);
        assert_eq!(request(&f,"GET","/configuration",Value::Null).await.body["command"],json!(["sh","-c","exit 7"]));
        assert_eq!(request(&f,"POST","/deploy",json!({})).await.status,StatusCode::ACCEPTED);
        wait(&f,"failed").await;
        for _ in 0..40 {
            if f.state.docker.inspect_container(&second_container,None).await.unwrap().state.unwrap().running==Some(true) {break;}
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert_eq!(f.state.docker.inspect_container(&second_container,None).await.unwrap().state.unwrap().running,Some(true));
        assert_eq!(request(&f,"POST","/rollback",json!({"deployment_id":first["id"]})).await.status,StatusCode::ACCEPTED);
        let rolled=wait(&f,"succeeded").await;
        assert_ne!(rolled["id"],second["id"]);
        assert_eq!(rolled["rollback_of"],first["id"]);
        let count=f.state.docker.list_containers(Some(bollard::query_parameters::ListContainersOptionsBuilder::default().filters(&HashMap::from([("volume".to_owned(),vec![volume_name.clone()])])).build())).await.unwrap().len();
        assert_eq!(count,1);
        let active=rolled["container_id"].as_str().unwrap();
        let output=tokio::process::Command::new("docker").args(["exec",active,"sh","-c","test \"$(cat /opt/data/identity)\" = original-identity && test \"$(stat -c %u:%g /opt/data/identity)\" = 123:456"]).output().await.unwrap(); assert!(output.status.success());
        let mut read_only=config.clone();read_only["storage"]["read_only"]=json!(true);
        assert_eq!(request(&f,"POST","/configuration",read_only).await.status,StatusCode::OK);
        assert_eq!(request(&f,"POST","/deploy",json!({})).await.status,StatusCode::ACCEPTED);
        let read_only=wait(&f,"succeeded").await;
        let output=tokio::process::Command::new("docker").args(["exec",read_only["container_id"].as_str().unwrap(),"sh","-c","touch /opt/data/must-not-write"]).output().await.unwrap();assert!(!output.status.success());
        // Default deletion retains owned data; explicit deletion is scoped to this service.
        assert_eq!(request(&f,"DELETE","",json!({"name":"Worker storage"})).await.status,StatusCode::NO_CONTENT);
        assert!(f.state.docker.inspect_volume(&volume_name).await.is_ok());
        // A second owned service can explicitly remove only its own volume.
        let env:Uuid=sqlx::query_scalar("SELECT id FROM environments LIMIT 1").fetch_one(&f.state.pool).await.unwrap();
        let created=call(&f.app,"POST",&format!("/api/v1/environments/{env}/services"),&f.cookie,json!({"kind":"application","name":"Disposable data","image":"nginx:alpine","container_port":80}),&[],true).await;
        assert_eq!(created.status,StatusCode::CREATED);
        let other:Uuid=created.body["id"].as_str().unwrap().parse().unwrap();let other_volume=volume(other);
        f.state.docker.create_volume(bollard::models::VolumeCreateRequest{name:Some(other_volume.clone()),labels:Some(HashMap::from([("io.ferrix.managed".into(),"true".into()),("io.ferrix.application".into(),other.to_string())])),..Default::default()}).await.unwrap();
        let removed=call(&f.app,"DELETE",&format!("/api/v1/services/{other}"),&f.cookie,json!({"name":"Disposable data","remove_data":true}),&[],true).await;
        assert_eq!(removed.status,StatusCode::NO_CONTENT);assert!(f.state.docker.inspect_volume(&other_volume).await.is_err());
        assert!(f.state.docker.inspect_volume(&volume_name).await.is_ok());
    }).catch_unwind().await;
    shutdown.send(true).unwrap();
    worker.await.unwrap();
    observe_stop.send(true).unwrap();
    let max_writers = observer.await.unwrap();
    // Scoped cleanup of only this disposable fixture, including retained test volume.
    let name = volume(f.service);
    let filters = HashMap::from([(
        "label".to_owned(),
        vec![format!("io.ferrix.application={}", f.service)],
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
                            bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                                .force(true)
                                .build(),
                        ),
                    )
                    .await;
            }
        }
    }
    let _ = f
        .state
        .docker
        .remove_volume(
            &name,
            None::<bollard::query_parameters::RemoveVolumeOptions>,
        )
        .await;
    f.finish().await;
    assert!(
        max_writers <= 1,
        "Observed {max_writers} simultaneous containers on one data volume"
    );
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker"]
async fn interrupted_stateful_replacement_cleans_candidate_before_recovery() {
    let f = Fixture::new().await;
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let result=AssertUnwindSafe(async {
        let config=json!({"name":"Recovery worker","image":"nginx:alpine","container_port":80,"worker_mode":true,"command":["sh","-c","sleep 1000"],"storage":{"target":"/opt/data","read_only":false}});
        assert_eq!(request(&f,"POST","/configuration",config).await.status,StatusCode::OK);
        assert_eq!(request(&f,"POST","/deploy",json!({})).await.status,StatusCode::ACCEPTED);
        let first=wait(&f,"succeeded").await;
        stop.send(true).unwrap();
        let active=first["container_id"].as_str().unwrap();
        // Wait for this worker to relinquish the schema-specific session lock.
        tokio::time::sleep(Duration::from_secs(3)).await;
        for was_running in [true,false] {
            f.state.docker.stop_container(active,None).await.or_else(|e|match e {bollard::errors::Error::DockerResponseServerError{status_code:304,..}=>Ok(()),e=>Err(e)}).unwrap();
            f.state.docker.update_container(active,bollard::models::ContainerUpdateBody{restart_policy:Some(bollard::models::RestartPolicy{name:Some(bollard::models::RestartPolicyNameEnum::NO),maximum_retry_count:None}),..Default::default()}).await.unwrap();
            let deployment=Uuid::new_v4();
            sqlx::query("INSERT INTO deployments(id,application_id,status,image,container_port,service_kind,worker_mode,service_cipher,restore_container_id,restore_was_running) SELECT $1,id,'running',image,container_port,service_kind,worker_mode,service_cipher,$2,$3 FROM applications WHERE id=$4").bind(deployment).bind(active).bind(was_running).bind(f.service).execute(&f.state.pool).await.unwrap();
            let info=f.state.docker.inspect_container(active,None).await.unwrap();
            let created=f.state.docker.create_container(None::<bollard::query_parameters::CreateContainerOptions>,bollard::models::ContainerCreateBody{
                image:info.image,cmd:Some(vec!["sh".into(),"-c".into(),"sleep 1000".into()]),labels:Some(HashMap::from([("io.ferrix.managed".into(),"true".into()),("io.ferrix.application".into(),f.service.to_string()),("io.ferrix.deployment".into(),deployment.to_string())])),
                host_config:Some(bollard::models::HostConfig{mounts:Some(vec![bollard::models::Mount{typ:Some(bollard::models::MountTypeEnum::VOLUME),source:Some(volume(f.service)),target:Some("/opt/data".into()),..Default::default()}]),..Default::default()}),..Default::default()
            }).await.unwrap();
            sqlx::query("UPDATE deployments SET container_id=$1 WHERE id=$2").bind(&created.id).bind(deployment).execute(&f.state.pool).await.unwrap();
            f.state.docker.start_container(&created.id,None).await.unwrap();
            let (stop_recovery,rx)=tokio::sync::watch::channel(false);
            let recovery=tokio::spawn(crate::deployments::run_worker(f.state.clone(),rx));
            for _ in 0..80 {
                let pending:bool=sqlx::query_scalar("SELECT restore_container_id IS NOT NULL FROM deployments WHERE id=$1").bind(deployment).fetch_one(&f.state.pool).await.unwrap();
                if !pending {break;}
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            stop_recovery.send(true).unwrap();recovery.await.unwrap();
            assert!(f.state.docker.inspect_container(&created.id,None).await.is_err());
            let info=f.state.docker.inspect_container(active,None).await.unwrap();
            assert_eq!(info.state.unwrap().running,Some(was_running));
            let pending:bool=sqlx::query_scalar("SELECT restore_container_id IS NOT NULL FROM deployments WHERE id=$1").bind(deployment).fetch_one(&f.state.pool).await.unwrap();assert!(!pending);
        }
        assert_eq!(request(&f,"DELETE","",json!({"name":"Recovery worker","remove_data":true})).await.status,StatusCode::NO_CONTENT);
        assert!(f.state.docker.inspect_volume(&volume(f.service)).await.is_err());
    }).catch_unwind().await;
    let _ = stop.send(true);
    worker.await.unwrap();
    // On failure remove only containers and volumes owned by this test service.
    let filters = HashMap::from([(
        "label".to_owned(),
        vec![format!("io.ferrix.application={}", f.service)],
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
                            bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                                .force(true)
                                .build(),
                        ),
                    )
                    .await;
            }
        }
    }
    let _ = f
        .state
        .docker
        .remove_volume(
            &volume(f.service),
            None::<bollard::query_parameters::RemoveVolumeOptions>,
        )
        .await;
    f.finish().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker"]
async fn application_does_not_adopt_or_delete_a_foreign_volume() {
    let f = Fixture::new().await;
    let name = volume(f.service);
    let foreign = Uuid::new_v4();
    f.state
        .docker
        .create_volume(bollard::models::VolumeCreateRequest {
            name: Some(name.clone()),
            labels: Some(HashMap::from([
                ("io.ferrix.managed".into(), "true".into()),
                ("io.ferrix.application".into(), foreign.to_string()),
            ])),
            ..Default::default()
        })
        .await
        .unwrap();
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let result=AssertUnwindSafe(async {
        let config=json!({"name":"Foreign volume test","image":"nginx:alpine","container_port":80,"worker_mode":true,"command":["sh","-c","sleep 1000"],"storage":{"target":"/opt/data","read_only":false}});
        assert_eq!(request(&f,"POST","/configuration",config).await.status,StatusCode::OK);
        assert_eq!(request(&f,"POST","/deploy",json!({})).await.status,StatusCode::ACCEPTED);
        let failed=wait(&f,"failed").await;assert!(failed["logs"].as_str().unwrap().contains("ownership"));
        assert_eq!(f.state.docker.inspect_volume(&name).await.unwrap().labels["io.ferrix.application"],foreign.to_string());
        assert_eq!(request(&f,"DELETE","",json!({"name":"Foreign volume test","remove_data":true})).await.status,StatusCode::NO_CONTENT);
        assert!(f.state.docker.inspect_volume(&name).await.is_ok());
    }).catch_unwind().await;
    stop.send(true).unwrap();
    worker.await.unwrap();
    // This foreign-labelled fixture was created by this test, never user data.
    f.state
        .docker
        .remove_volume(
            &name,
            None::<bollard::query_parameters::RemoveVolumeOptions>,
        )
        .await
        .unwrap();
    f.finish().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}
