use super::*;
use crate::deployments::test_support::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::FutureExt;
use std::{panic::AssertUnwindSafe, time::Duration};
use tower::ServiceExt;
async fn deploy(f: &Fixture, service: Uuid) -> Value {
    assert_eq!(
        f.request(
            "POST",
            &format!("/api/v1/services/{service}/deploy"),
            json!({})
        )
        .await
        .status,
        StatusCode::ACCEPTED
    );
    for _ in 0..300 {
        let rows = f
            .request(
                "GET",
                &format!("/api/v1/services/{service}/deployments"),
                Value::Null,
            )
            .await
            .body;
        if rows[0]["status"] == "succeeded" {
            return rows[0].clone();
        }
        assert_ne!(rows[0]["status"], "failed", "Deployment failed");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Deploy timed out")
}
async fn wait(f: &Fixture, id: Uuid, expected: &str) -> Value {
    for _ in 0..600 {
        let row = f
            .request("GET", &format!("/api/v1/project-backups/{id}"), Value::Null)
            .await;
        assert_eq!(row.status, StatusCode::OK);
        if row.body["status"] == expected {
            return row.body;
        }
        assert_ne!(row.body["status"], "failed", "{}", row.body["message"]);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Project operation timed out")
}
async fn execute(f: &Fixture, service: Uuid, args: Vec<String>) {
    let container: String =
        sqlx::query_scalar("SELECT active_container_id FROM applications WHERE id=$1")
            .bind(service)
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    let out = tokio::process::Command::new("docker")
        .arg("exec")
        .arg(container)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "Fixture command failed");
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker"]
async fn coordinated_project_bundle_restores_isolated_data_and_cuts_over_workers() {
    let f = Fixture::new().await;
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let result=AssertUnwindSafe(async{
  let (env,project):(Uuid,Uuid)=sqlx::query_as("SELECT a.environment_id,e.project_id FROM applications a JOIN environments e ON e.id=a.environment_id WHERE a.id=$1").bind(f.service).fetch_one(&f.state.pool).await.unwrap();
  let config=json!({"name":"Bundle web","image":"nginx:alpine","container_port":80,"environment":"APP_SECRET=bundle-fixture-private","storage":{"target":"/data","read_only":false},"command":["sh","-c","test -f /data/identity || printf web-data > /data/identity; chown 123:456 /data/identity; exec nginx -g 'daemon off;'"]});
  assert_eq!(f.request("POST",&format!("/api/v1/services/{}/configuration",f.service),config).await.status,StatusCode::OK);deploy(&f,f.service).await;
  let db=f.request("POST",&format!("/api/v1/environments/{env}/services"),json!({"kind":"database","name":"Bundle MariaDB","engine":"mariadb","database_name":"bundle_db","username":"bundle_app"})).await;assert_eq!(db.status,StatusCode::CREATED);let db=Uuid::parse_str(db.body["id"].as_str().unwrap()).unwrap();deploy(&f,db).await;
  let config:Vec<u8>=sqlx::query_scalar("SELECT service_cipher FROM applications WHERE id=$1").bind(db).fetch_one(&f.state.pool).await.unwrap();let credentials:services::DatabaseConfig=services::open(&f.state,db,&config).unwrap();
  execute(&f,db,vec!["sh".into(),"-c".into(),"MYSQL_PWD=\"$MARIADB_ROOT_PASSWORD\" mariadb -uroot bundle_db -e 'CREATE TABLE records(id int primary key, value varchar(40)); INSERT INTO records VALUES(1,\"bundle-data\");'".into()]).await;
  let agent=f.request("POST",&format!("/api/v1/environments/{env}/services"),json!({"kind":"application","name":"Bundle worker","image":"alpine:3.22","container_port":80,"environment":""})).await;assert_eq!(agent.status,StatusCode::CREATED);let agent=Uuid::parse_str(agent.body["id"].as_str().unwrap()).unwrap();
  assert_eq!(f.request("POST",&format!("/api/v1/services/{agent}/configuration"),json!({"name":"Bundle worker","image":"alpine:3.22","container_port":80,"environment":"","worker_mode":true,"storage":{"target":"/data","read_only":false},"command":["sh","-c","test -f /data/identity || printf worker-data > /data/identity; chown 123:456 /data/identity; exec sleep 10000"]})).await.status,StatusCode::OK);deploy(&f,agent).await;
  let domain=Uuid::new_v4();sqlx::query("INSERT INTO application_domains(id,application_id,hostname,verification_token,verified_at,dns_status,canonical) VALUES($1,$2,'bundle.example.com','fixture',now(),'ready',true)").bind(domain).bind(f.service).execute(&f.state.pool).await.unwrap();
  let root=f.request("POST",&format!("/api/v1/projects/{project}/backups"),json!({"offline":true})).await;assert_eq!(root.status,StatusCode::OK);let job=Uuid::parse_str(root.body["id"].as_str().unwrap()).unwrap();let key=root.body["recovery_key"].as_str().unwrap();
  assert_eq!(f.request("POST",&format!("/api/v1/services/{agent}/control"),json!({"operation":"start"})).await.status,StatusCode::CONFLICT);
  wait(&f,job,"succeeded").await;
  for service in [f.service,agent]{let container:String=sqlx::query_scalar("SELECT active_container_id FROM applications WHERE id=$1").bind(service).fetch_one(&f.state.pool).await.unwrap();assert_eq!(worker::inspect_owned(&f.state,service,&container).await.unwrap().state.unwrap().running,Some(true));}
  let response=f.app.clone().oneshot(Request::builder().uri(format!("/api/v1/projects/{project}/backups/{job}/download")).header("cookie",&f.cookie).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);let bytes=to_bytes(response.into_body(),format::LIMIT+128).await.unwrap();assert!(!String::from_utf8_lossy(&bytes).contains("bundle-fixture-private"));
  let response=f.app.clone().oneshot(Request::builder().method("POST").uri(format!("/api/v1/project-backups/import?project={project}")).header("cookie",&f.cookie).header("origin",ORIGIN).header("x-ferrix-bundle-key",key).header("content-type","application/octet-stream").body(Body::from(bytes)).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);let preview:Value=serde_json::from_slice(&to_bytes(response.into_body(),1024*1024).await.unwrap()).unwrap();assert!(preview["missing_images"].as_array().unwrap().is_empty());let restore=Uuid::parse_str(preview["id"].as_str().unwrap()).unwrap();
  assert_eq!(f.request("POST",&format!("/api/v1/project-backups/{restore}/restore"),json!({"name":"Recovered project"})).await.status,StatusCode::ACCEPTED);let ready=wait(&f,restore,"restored").await;let target_project=Uuid::parse_str(ready["target_project_id"].as_str().unwrap()).unwrap();assert_ne!(target_project,project);
  let maps:Vec<Mapping>=serde_json::from_value(ready["mapping"].clone()).unwrap();
  for map in &maps{let (running,traffic,blocked):(Option<String>,bool,bool)=sqlx::query_as("SELECT active_container_id,traffic_enabled,cutover_pending FROM applications WHERE id=$1").bind(map.target).fetch_one(&f.state.pool).await.unwrap();assert!(!traffic);if map.source!=db{let volume=application_runtime::volume(map.target);let expected=if map.worker{"worker-data"}else{"web-data"};let out=tokio::process::Command::new("docker").args(["run","--rm","--network","none","-v",&format!("{volume}:/data:ro"),"alpine:3.22","sh","-c",&format!("test \"$(cat /data/identity)\" = {expected} && test \"$(stat -c %u:%g /data/identity)\" = 123:456")]).output().await.unwrap();assert!(out.status.success(),"Stopped target file ownership/data mismatch");assert_eq!(worker::inspect_owned(&f.state,map.target,running.as_deref().unwrap()).await.unwrap().state.unwrap().running,Some(false));assert_eq!(blocked,map.worker);}
   if map.source==db{let cipher:Vec<u8>=sqlx::query_scalar("SELECT service_cipher FROM applications WHERE id=$1").bind(map.target).fetch_one(&f.state.pool).await.unwrap();let imported:services::DatabaseConfig=services::open(&f.state,map.target,&cipher).unwrap();assert_eq!(imported.password,credentials.password);assert!(services::open::<services::DatabaseConfig>(&f.state,db,&cipher).is_err());execute(&f,map.target,vec!["sh".into(),"-c".into(),"test \"$(MYSQL_PWD=\"$MARIADB_ROOT_PASSWORD\" mariadb -uroot bundle_db -N -e 'SELECT value FROM records WHERE id=1')\" = bundle-data".into()]).await;}
  }
  let target_agent=maps.iter().find(|m|m.source==agent).unwrap().target;assert_eq!(f.request("POST",&format!("/api/v1/services/{target_agent}/control"),json!({"operation":"start"})).await.status,StatusCode::CONFLICT);
  assert_eq!(f.request("POST",&format!("/api/v1/project-backups/{restore}/cutover"),json!({"source_writers_stopped":true})).await.status,StatusCode::ACCEPTED);wait(&f,restore,"active").await;
  let domain_owner:Uuid=sqlx::query_scalar("SELECT application_id FROM application_domains WHERE id=$1").bind(domain).fetch_one(&f.state.pool).await.unwrap();assert_eq!(domain_owner,maps.iter().find(|m|m.source==f.service).unwrap().target);
  for map in maps{if map.source!=db{execute(&f,map.target,vec!["sh".into(),"-c".into(),format!("test \"$(cat /data/identity)\" = {} && test \"$(stat -c %u:%g /data/identity)\" = 123:456",if map.worker{"worker-data"}else{"web-data"})]).await;let source:String=sqlx::query_scalar("SELECT active_container_id FROM applications WHERE id=$1").bind(map.source).fetch_one(&f.state.pool).await.unwrap();assert_eq!(worker::inspect_owned(&f.state,map.source,&source).await.unwrap().state.unwrap().running,Some(false));}}
 }).catch_unwind().await;
    stop.send(true).unwrap();
    worker.await.unwrap();
    sqlx::query("DELETE FROM project_maintenance")
        .execute(&f.state.pool)
        .await
        .unwrap();
    let services: Vec<(Uuid, String)> = sqlx::query_as("SELECT id,name FROM applications")
        .fetch_all(&f.state.pool)
        .await
        .unwrap();
    for (service, label) in services {
        let _ = f
            .request(
                "DELETE",
                &format!("/api/v1/services/{service}"),
                json!({"name":label,"remove_data":true}),
            )
            .await;
    }
    f.finish().await;
    if let Err(e) = result {
        std::panic::resume_unwind(e)
    }
}

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker"]
async fn maintenance_blocks_deletion_before_touching_owned_data() {
    use bollard::models::VolumeCreateRequest;
    let f = Fixture::new().await;
    let volume = application_runtime::volume(f.service);
    let result=AssertUnwindSafe(async{
  let project:Uuid=sqlx::query_scalar("SELECT e.project_id FROM applications a JOIN environments e ON e.id=a.environment_id WHERE a.id=$1").bind(f.service).fetch_one(&f.state.pool).await.unwrap();
  let deployment=Uuid::new_v4();sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,status,image_id,service_kind) VALUES($1,$2,'nginx:alpine',80,'succeeded','sha256:fixture','application')").bind(deployment).bind(f.service).execute(&f.state.pool).await.unwrap();sqlx::query("UPDATE applications SET active_deployment_id=$2 WHERE id=$1").bind(f.service).bind(deployment).execute(&f.state.pool).await.unwrap();
  f.state.docker.create_volume(VolumeCreateRequest{name:Some(volume.clone()),labels:Some(std::collections::HashMap::from([("io.ferrix.managed".into(),"true".into()),("io.ferrix.application".into(),f.service.to_string())])),..Default::default()}).await.unwrap();
  assert_eq!(f.request("POST",&format!("/api/v1/projects/{project}/backups"),json!({"offline":true})).await.status,StatusCode::OK);
  assert_eq!(f.request("DELETE",&format!("/api/v1/services/{}",f.service),json!({"name":"Git web","remove_data":true})).await.status,StatusCode::CONFLICT);
  assert!(f.state.docker.inspect_volume(&volume).await.is_ok(),"Maintenance rejection must happen before Docker data deletion");
 }).catch_unwind().await;
    sqlx::query("DELETE FROM project_maintenance")
        .execute(&f.state.pool)
        .await
        .unwrap();
    let _ = f
        .request(
            "DELETE",
            &format!("/api/v1/services/{}", f.service),
            json!({"name":"Git web","remove_data":true}),
        )
        .await;
    let _ = f
        .state
        .docker
        .remove_volume(
            &volume,
            None::<bollard::query_parameters::RemoveVolumeOptions>,
        )
        .await;
    f.finish().await;
    if let Err(e) = result {
        std::panic::resume_unwind(e)
    }
}
