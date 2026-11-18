use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ferrix_api::{AppState, auth::AuthConfig, deployments::run_worker, router};
use futures_util::FutureExt;
use serde_json::{Value, json};
use sqlx::{Executor, postgres::PgPoolOptions};
use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;
const ORIGIN: &str = "http://127.0.0.1:5173";
const PASSWORD: &str = "A sufficiently long deployment test password!";
async fn call(
    app: &Router,
    path: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(if body.is_some() { "POST" } else { "GET" })
                .uri(path)
                .header("origin", ORIGIN)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cookie,
    )
}
async fn wait_status(app: &Router, cookie: &str, application: Uuid, status: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        let (code, rows, _) = call(
            app,
            &format!("/api/v1/applications/{application}/deployments"),
            cookie,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        if rows[0]["status"] == status {
            return rows[0].clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Timed out waiting for {status}: {rows}"
        );
        if status == "succeeded" {
            assert_ne!(rows[0]["status"], "failed", "{rows}");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
async fn nginx_responds(state: &AppState, port: i64) {
    // Verify the actual host loopback port, not just Docker container state.
    let result = tokio::process::Command::new("curl")
        .args([
            "--max-time",
            "5",
            "-fsS",
            &format!("http://127.0.0.1:{port}"),
        ])
        .output()
        .await
        .unwrap();
    assert!(result.status.success(), "nginx HTTP request failed");
    assert!(String::from_utf8_lossy(&result.stdout).contains("Welcome to nginx"));
    assert!(state.docker.ping().await.is_ok());
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and a Docker daemon; creates isolated schema and containers"]
async fn docker_deployment_flow_and_isolation() {
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("deploy_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let namespace = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |conn, _| {
            let query = format!("SET search_path TO {namespace}");
            Box::pin(async move {
                conn.execute(query.as_str()).await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let state = AppState {
        pool: pool.clone(),
        docker: bollard::Docker::connect_with_defaults().unwrap(),
        auth: Arc::new(AuthConfig {
            origin: ORIGIN.into(),
            secure: false,
            cookie_name: "deployment_test_session".into(),
            key: [42; 32],
        }),
    };
    let app = router(state.clone());
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let mut workers = Vec::new();
    let mut application = None;
    let mut sentinel = None;
    let outcome=AssertUnwindSafe(async {
        assert_eq!(call(&app,"/api/v1/applications/00000000-0000-0000-0000-000000000000/deploy","",Some(json!({}))).await.0,StatusCode::UNAUTHORIZED);
        let (code,_,owner)=call(&app,"/api/v1/auth/setup","",Some(json!({"name":"Deploy Owner","email":"owner@example.com","password":PASSWORD}))).await;
        assert_eq!(code,StatusCode::OK);
        let identity=call(&app,"/api/v1/auth/session",&owner,None).await.1;
        let org=identity["organization_id"].as_str().unwrap();
        let (code,project,_)=call(&app,"/api/v1/projects",&owner,Some(json!({"name":"Docker test"}))).await;
        assert_eq!(code,StatusCode::CREATED);
        let project=project["id"].as_str().unwrap();
        let (code,envs,_)=call(&app,&format!("/api/v1/projects/{project}/environments"),&owner,None).await;
        assert_eq!(code,StatusCode::OK); assert_eq!(envs["environments"][0]["name"],"production");
        let environment=envs["environments"][0]["id"].as_str().unwrap();
        assert_eq!(call(&app,&format!("/api/v1/projects/{project}/environments"),&owner,Some(json!({"name":"production"}))).await.0,StatusCode::CONFLICT);
        assert_eq!(call(&app,&format!("/api/v1/projects/{project}/environments"),&owner,Some(json!({"name":"staging"}))).await.0,StatusCode::CREATED);
        let config=json!({"name":"nginx","image":"nginx:alpine","container_port":80,"environment":"FERRIX_TEST_SECRET=literal-secret-123\nLITERAL=$(whoami)"});
        let (code,created,_)=call(&app,&format!("/api/v1/environments/{environment}/applications"),&owner,Some(config.clone())).await;
        assert_eq!(code,StatusCode::CREATED,"{created}");
        let application_id=Uuid::parse_str(created["id"].as_str().unwrap()).unwrap(); application=Some(application_id);
        assert!(created.get("env_cipher").is_none()); assert!(created.get("environment").is_none());
        let cipher:Vec<u8>=sqlx::query_scalar("SELECT env_cipher FROM applications WHERE id=$1").bind(application_id).fetch_one(&pool).await.unwrap();
        assert!(!String::from_utf8_lossy(&cipher).contains("literal-secret-123"));
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/configuration"),&owner,None).await.1["environment"],config["environment"]);
        let member_id=Uuid::new_v4();
        sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) SELECT $1,'Member','member@example.com',password_hash FROM auth_users WHERE email='owner@example.com'").bind(member_id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'member')").bind(org.parse::<Uuid>().unwrap()).bind(member_id).execute(&pool).await.unwrap();
        let (code,_,member)=call(&app,"/api/v1/auth/sign-in","",Some(json!({"email":"member@example.com","password":PASSWORD}))).await;
        assert_eq!(code,StatusCode::OK);
        assert_eq!(call(&app,&format!("/api/v1/environments/{environment}/applications"),&member,None).await.0,StatusCode::OK);
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/configuration"),&member,None).await.0,StatusCode::FORBIDDEN);
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/deploy"),&member,Some(json!({}))).await.0,StatusCode::FORBIDDEN);
        let foreign=call(&app,"/api/v1/auth/organizations",&owner,Some(json!({"name":"Other"}))).await.1;
        assert_eq!(call(&app,"/api/v1/auth/organizations/switch",&owner,Some(json!({"organization_id":foreign["id"]}))).await.0,StatusCode::NO_CONTENT);
        for path in [format!("/api/v1/projects/{project}/environments"),format!("/api/v1/environments/{environment}/applications"),format!("/api/v1/applications/{application_id}"),format!("/api/v1/applications/{application_id}/deployments"),format!("/api/v1/applications/{application_id}/logs")] {
            assert_eq!(call(&app,&path,&owner,None).await.0,StatusCode::NOT_FOUND,"{path}");
        }
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/deploy"),&owner,Some(json!({}))).await.0,StatusCode::NOT_FOUND);
        call(&app,"/api/v1/auth/organizations/switch",&owner,Some(json!({"organization_id":org}))).await;
        let endpoint=format!("/api/v1/applications/{application_id}/deploy");
        assert_eq!(call(&app,&endpoint,&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
        assert_eq!(call(&app,&endpoint,&owner,Some(json!({}))).await.0,StatusCode::CONFLICT);
        let mut invalid_image=config.clone(); invalid_image["image"]=json!(format!("nginx:ferrix-missing-{}",Uuid::new_v4().simple()));
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/configuration"),&owner,Some(invalid_image)).await.0,StatusCode::OK);
        // Both workers start together. The session lock must prevent one from recovering the other's live job.
        workers.push(tokio::spawn(run_worker(state.clone(),receiver.clone())));
        workers.push(tokio::spawn(run_worker(state.clone(),receiver.clone())));
        let succeeded=wait_status(&app,&owner,application_id,"succeeded").await;
        assert_eq!(succeeded["image"],"nginx:alpine","queued configuration must be immutable");
        assert!(succeeded["image_id"].as_str().unwrap().starts_with("sha256:"));
        let running=call(&app,&format!("/api/v1/applications/{application_id}/runtime"),&owner,None).await.1;
        assert_eq!(running["state"],"running"); nginx_responds(&state,running["host_port"].as_i64().unwrap()).await;
        let old_container=running["container_id"].as_str().unwrap().to_owned();
        let unrelated = state.docker.create_container(None, bollard::models::ContainerCreateBody {
            image: Some("nginx:alpine".into()),
            labels: Some(std::collections::HashMap::from([
                ("io.ferrix.managed".into(), "true".into()),
                ("io.ferrix.application".into(), Uuid::new_v4().to_string()),
                ("io.ferrix.deployment".into(), Uuid::new_v4().to_string()),
            ])),
            ..Default::default()
        }).await.unwrap();
        sentinel = Some(unrelated.id);

        assert_eq!(call(&app,&endpoint,&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
        let failed=wait_status(&app,&owner,application_id,"failed").await;
        assert!(failed["logs"].as_str().unwrap().contains("image"));
        let still_running=call(&app,&format!("/api/v1/applications/{application_id}/runtime"),&owner,None).await.1;
        assert_eq!(still_running["container_id"],old_container); nginx_responds(&state,still_running["host_port"].as_i64().unwrap()).await;
        // Persisted interrupted work is reconciled without removing the active container.
        let interrupted=Uuid::new_v4();
        sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,status) VALUES($1,$2,'nginx:alpine',80,'running')").bind(interrupted).bind(application_id).execute(&pool).await.unwrap();
        let deadline=tokio::time::Instant::now()+Duration::from_secs(10);
        loop {
            let status:String=sqlx::query_scalar("SELECT status FROM deployments WHERE id=$1").bind(interrupted).fetch_one(&pool).await.unwrap();
            if status=="failed" {break;} assert!(tokio::time::Instant::now()<deadline);tokio::time::sleep(Duration::from_millis(200)).await;
        }
        call(&app,&format!("/api/v1/applications/{application_id}/configuration"),&owner,Some(config)).await;
        assert_eq!(call(&app,&endpoint,&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
        let next=wait_status(&app,&owner,application_id,"succeeded").await;
        assert_ne!(next["container_id"],old_container);
        let running=call(&app,&format!("/api/v1/applications/{application_id}/runtime"),&owner,None).await.1;
        nginx_responds(&state,running["host_port"].as_i64().unwrap()).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(state.docker.inspect_container(&old_container,None).await.is_err(),"old owned container should be removed after activation");
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/logs"),&owner,None).await.0,StatusCode::OK);
    }).catch_unwind().await;
    if let Some(ref id) = sentinel {
        assert!(
            state.docker.inspect_container(id, None).await.is_ok(),
            "worker must preserve containers outside its database ownership"
        );
    }
    let _ = shutdown.send(true);
    for worker in workers {
        let _ = worker.await;
    }
    if let Some(application) = application {
        let options = bollard::query_parameters::ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&std::collections::HashMap::from([(
                "label".to_owned(),
                vec![format!("io.ferrix.application={application}")],
            )]))
            .build();
        for container in state
            .docker
            .list_containers(Some(options))
            .await
            .unwrap_or_default()
        {
            if let Some(id) = container.id {
                let _ = state
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
    if let Some(id) = sentinel {
        let _ = state
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
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    admin.close().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
