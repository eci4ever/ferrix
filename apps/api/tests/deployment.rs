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
async fn remove_service(
    app: &Router,
    service: Uuid,
    cookie: &str,
    name: &str,
    remove_data: bool,
) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/services/{service}"))
                .header("origin", ORIGIN)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name":name,"remove_data":remove_data}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
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
async fn wait_database_job(app: &Router, cookie: &str, service: Uuid, job: &Value) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let (code, rows, _) = call(
            app,
            &format!("/api/v1/services/{service}/backups"),
            cookie,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == job["id"])
            .unwrap();
        if row["status"] == "succeeded" || row["status"] == "failed" {
            return row.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Database job timed out: {rows}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
async fn backup_roundtrip(
    app: &Router,
    state: &AppState,
    cookie: &str,
    service: Uuid,
    engine: &str,
    added: &mut Vec<Uuid>,
) {
    let base = format!("/api/v1/services/{service}/backups");
    let settings = call(
        app,
        &format!("/api/v1/services/{service}/configuration"),
        cookie,
        None,
    )
    .await
    .1;
    let source_runtime = call(
        app,
        &format!("/api/v1/services/{service}/runtime"),
        cookie,
        None,
    )
    .await
    .1;
    let source_container = source_runtime["container_id"].as_str().unwrap();
    let (code, backup, _) = call(app, &base, cookie, Some(json!({}))).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{backup}");
    assert_eq!(
        call(
            app,
            &format!("/api/v1/services/{service}/deploy"),
            cookie,
            Some(json!({}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            app,
            &format!("/api/v1/services/{service}/control"),
            cookie,
            Some(json!({"operation":"stop"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let backed_up = wait_database_job(app, cookie, service, &backup).await;
    assert_eq!(backed_up["status"], "succeeded", "{backed_up}");
    assert!(backed_up["size_bytes"].as_i64().unwrap() > 0);
    // Rows added after the backup distinguish a genuine snapshot from the live database.
    let write = if engine == "postgres" {
        "psql -U ferrix -d ferrix -v ON_ERROR_STOP=1 -c \"INSERT INTO persistence_check VALUES ('after-backup');\""
    } else {
        "mariadb -u\"$MARIADB_USER\" -p\"$MARIADB_PASSWORD\" \"$MARIADB_DATABASE\" -e \"INSERT INTO persistence_check VALUES ('after-backup');\""
    };
    let output = tokio::process::Command::new("docker")
        .args(["exec", source_container, "sh", "-c", write])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let restore_url = format!("{base}/{}/restore", backup["id"].as_str().unwrap());
    let (code, restore, _) = call(
        app,
        &restore_url,
        cookie,
        Some(json!({"name":format!("{engine} restored")})),
    )
    .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{restore}");
    let target: Uuid = restore["application_id"].as_str().unwrap().parse().unwrap();
    added.push(target);
    assert_ne!(target, service);
    let restored = wait_database_job(app, cookie, service, &restore).await;
    assert_eq!(restored["status"], "succeeded", "{restored}");
    let new_settings = call(
        app,
        &format!("/api/v1/services/{target}/configuration"),
        cookie,
        None,
    )
    .await
    .1;
    assert_ne!(new_settings["password"], settings["password"]);
    let target_runtime = call(
        app,
        &format!("/api/v1/services/{target}/runtime"),
        cookie,
        None,
    )
    .await
    .1;
    let read = if engine == "postgres" {
        "psql -U ferrix -d ferrix -At -c 'SELECT value FROM persistence_check ORDER BY value'"
    } else {
        "mariadb -u\"$MARIADB_USER\" -p\"$MARIADB_PASSWORD\" \"$MARIADB_DATABASE\" -N -e 'SELECT value FROM persistence_check ORDER BY value'"
    };
    let output = tokio::process::Command::new("docker")
        .args([
            "exec",
            target_runtime["container_id"].as_str().unwrap(),
            "sh",
            "-c",
            read,
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "restored database could not be read"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "survives-redeploy"
    );
    let output = tokio::process::Command::new("docker")
        .args(["exec", source_container, "sh", "-c", read])
        .output()
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("after-backup"));
    if engine == "postgres" {
        let backup_id: Uuid = backup["id"].as_str().unwrap().parse().unwrap();
        let original_hash: String =
            sqlx::query_scalar("SELECT sha256 FROM database_jobs WHERE id=$1")
                .bind(backup_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        sqlx::query("UPDATE database_jobs SET sha256=repeat('0',64) WHERE id=$1")
            .bind(backup_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let (code, rejected, _) = call(
            app,
            &restore_url,
            cookie,
            Some(json!({"name":"Checksum rejected restore"})),
        )
        .await;
        assert_eq!(code, StatusCode::ACCEPTED);
        let rejected_target: Uuid = rejected["application_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        added.push(rejected_target);
        let failure = wait_database_job(app, cookie, service, &rejected).await;
        assert_eq!(
            failure["status"], "failed",
            "corrupted backups must never restore successfully"
        );
        assert!(failure["message"].as_str().unwrap().contains("integrity"));
        assert_eq!(
            remove_service(
                app,
                rejected_target,
                cookie,
                "Checksum rejected restore",
                true
            )
            .await,
            StatusCode::NO_CONTENT
        );
        sqlx::query("UPDATE database_jobs SET sha256=$1 WHERE id=$2")
            .bind(original_hash)
            .bind(backup_id)
            .execute(&state.pool)
            .await
            .unwrap();
        // A helper left behind by a previous API process is stopped before the service unlocks.
        let interrupted = Uuid::new_v4();
        let helper = state
            .docker
            .create_container(
                Some(
                    bollard::query_parameters::CreateContainerOptionsBuilder::default()
                        .name(&format!("ferrix-backup-{}", interrupted.simple()))
                        .build(),
                ),
                bollard::models::ContainerCreateBody {
                    image: Some("postgres:17-alpine".into()),
                    entrypoint: Some(vec!["sh".into(), "-c".into()]),
                    cmd: Some(vec!["sleep 300".into()]),
                    labels: Some(std::collections::HashMap::from([
                        ("io.ferrix.managed".into(), "true".into()),
                        ("io.ferrix.application".into(), service.to_string()),
                        ("io.ferrix.backup-job".into(), interrupted.to_string()),
                    ])),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        state
            .docker
            .start_container(&helper.id, None)
            .await
            .unwrap();
        sqlx::query("INSERT INTO database_jobs(id,application_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,deployment_id,status) SELECT $1,application_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,deployment_id,'running' FROM database_jobs WHERE id=$2")
            .bind(interrupted).bind(backup_id).execute(&state.pool).await.unwrap();
        let recovered = wait_database_job(app, cookie, service, &json!({"id":interrupted})).await;
        assert_eq!(recovered["status"], "failed");
        assert!(
            state
                .docker
                .inspect_container(&helper.id, None)
                .await
                .is_err()
        );
    }
    assert_eq!(
        remove_service(app, target, cookie, &format!("{engine} restored"), true).await,
        StatusCode::NO_CONTENT
    );
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
        gateway: Default::default(),
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
    let mut added_services: Vec<Uuid> = Vec::new();
    let mut owned_environments: Vec<Uuid> = Vec::new();
    let mut sentinel = None;
    let outcome=AssertUnwindSafe(async {
        assert_eq!(call(&app,"/api/v1/applications/00000000-0000-0000-0000-000000000000/deploy","",Some(json!({}))).await.0,StatusCode::UNAUTHORIZED);
        let (code,_,owner)=call(&app,"/api/v1/auth/setup","",Some(json!({"name":"Deploy Owner","email":"owner@example.com","password":PASSWORD}))).await;
        assert_eq!(code,StatusCode::OK);
        for section in ["images", "volumes", "networks", "swarm", "events", "disk-usage", "health"] {
            let (code, body, _) = call(&app, &format!("/api/v1/docker/{section}"), &owner, None).await;
            assert_eq!(code, StatusCode::OK, "{section}: {body}");
            match section {
                "images" | "volumes" | "networks" | "events" => assert!(body.is_array(), "{section}"),
                "health" => assert_eq!(body["connected"], true),
                _ => assert!(body.is_object(), "{section}"),
            }
        }
        let identity=call(&app,"/api/v1/auth/session",&owner,None).await.1;
        let org=identity["organization_id"].as_str().unwrap();
        let (code,project,_)=call(&app,"/api/v1/projects",&owner,Some(json!({"name":"Docker test"}))).await;
        assert_eq!(code,StatusCode::CREATED);
        let project=project["id"].as_str().unwrap();
        let (code,envs,_)=call(&app,&format!("/api/v1/projects/{project}/environments"),&owner,None).await;
        assert_eq!(code,StatusCode::OK); assert_eq!(envs["environments"][0]["name"],"production");
        let environment=envs["environments"][0]["id"].as_str().unwrap();
        owned_environments.push(environment.parse().unwrap());
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
        for path in [format!("/api/v1/projects/{project}/environments"),format!("/api/v1/environments/{environment}/applications"),format!("/api/v1/applications/{application_id}"),format!("/api/v1/applications/{application_id}/deployments"),format!("/api/v1/applications/{application_id}/logs"),format!("/api/v1/services/{application_id}/metrics")] {
            assert_eq!(call(&app,&path,&owner,None).await.0,StatusCode::NOT_FOUND,"{path}");
        }
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/deploy"),&owner,Some(json!({}))).await.0,StatusCode::NOT_FOUND);
        call(&app,"/api/v1/auth/organizations/switch",&owner,Some(json!({"organization_id":org}))).await;
        let endpoint=format!("/api/v1/applications/{application_id}/deploy");
        assert_eq!(call(&app,&endpoint,&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
        assert_eq!(call(&app,&endpoint,&owner,Some(json!({}))).await.0,StatusCode::CONFLICT);
        let mut invalid_image=config.clone(); invalid_image["image"]=json!(format!("nginx:ferrix-missing-{}",Uuid::new_v4().simple()));
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/configuration"),&owner,Some(invalid_image)).await.0,StatusCode::OK);
        assert_eq!(remove_service(&app,application_id,&owner,config["name"].as_str().unwrap(),false).await,StatusCode::CONFLICT);
        assert_eq!(call(&app,&format!("/api/v1/services/{application_id}/control"),&owner,Some(json!({"operation":"stop"}))).await.0,StatusCode::CONFLICT);
        // Both workers start together. The session lock must prevent one from recovering the other's live job.
        workers.push(tokio::spawn(run_worker(state.clone(),receiver.clone())));
        workers.push(tokio::spawn(run_worker(state.clone(),receiver.clone())));
        let succeeded=wait_status(&app,&owner,application_id,"succeeded").await;
        assert_eq!(succeeded["image"],"nginx:alpine","queued configuration must be immutable");
        assert!(succeeded["image_id"].as_str().unwrap().starts_with("sha256:"));
        let running=call(&app,&format!("/api/v1/applications/{application_id}/runtime"),&owner,None).await.1;
        assert_eq!(running["state"],"running"); nginx_responds(&state,running["host_port"].as_i64().unwrap()).await;
        let old_container=running["container_id"].as_str().unwrap().to_owned();
        let metrics_url=format!("/api/v1/services/{application_id}/metrics");
        let (code,metrics,_)=call(&app,&metrics_url,&member,None).await;
        assert_eq!(code,StatusCode::OK,"{metrics}");
        assert_eq!(metrics["container_id"],running["container_id"]);
        assert_eq!(metrics["cpu_limit_cores"],1.0);
        assert_eq!(metrics["memory_limit_bytes"],512*1024*1024u64);
        assert!(metrics["cpu_cores_used"].is_number(),"{metrics}");
        assert!(metrics["memory_used_bytes"].as_u64().unwrap()>0);
        assert!(metrics["memory_used_bytes"].as_u64().unwrap() <= metrics["memory_raw_bytes"].as_u64().unwrap());
        let logs_url=format!("/api/v1/services/{application_id}/logs?tail=500&timestamps=true");
        let logs=call(&app,&logs_url,&member,None).await;
        assert_eq!(logs.0,StatusCode::OK);
        assert!(logs.1["logs"].as_str().unwrap().contains("GET / HTTP"));
        assert_eq!(logs.1["container_id"],running["container_id"]);
        let output=tokio::process::Command::new("docker").args(["exec",&old_container,"sh","-c","(head -c 70000 /dev/zero | tr '\\000' x; printf '\\nservice-tail-marker-✓\\n') > /proc/1/fd/1"]).output().await.unwrap();
        assert!(output.status.success());
        let bounded=call(&app,&format!("/api/v1/services/{application_id}/logs?tail=1000&timestamps=true"),&owner,None).await;
        assert_eq!(bounded.0,StatusCode::OK);
        assert_eq!(bounded.1["truncated"],true);
        let text=bounded.1["logs"].as_str().unwrap();
        assert!(text.len()<=65536);
        assert!(text.contains("service-tail-marker-✓"),"newest log output must survive truncation");

        assert_eq!(call(&app,&format!("/api/v1/services/{application_id}/logs?tail=1001"),&owner,None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        let control=format!("/api/v1/services/{application_id}/control");
        assert_eq!(call(&app,&control,&owner,Some(json!({"operation":"stop"}))).await.0,StatusCode::NO_CONTENT);
        let stopped_metrics=call(&app,&metrics_url,&member,None).await.1;
        assert_eq!(stopped_metrics["state"],"exited");
        assert!(stopped_metrics["cpu_cores_used"].is_null());
        assert!(stopped_metrics["memory_used_bytes"].is_null());
        assert_eq!(call(&app,&control,&owner,Some(json!({"operation":"start"}))).await.0,StatusCode::NO_CONTENT);
        assert_eq!(call(&app,&metrics_url,&member,None).await.1["state"],"running");

        let unrelated = state.docker.create_container(None, bollard::models::ContainerCreateBody {
            image: Some("nginx:alpine".into()),
            labels: Some(std::collections::HashMap::from([
                ("io.ferrix.test".into(), schema.clone()),
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
        let summary=call(&app,"/api/v1/projects/service-status",&owner,None).await;
        assert_eq!(summary.0,StatusCode::OK);
        assert_eq!(summary.1["projects"][0]["running"],1);
        assert_eq!(summary.1["projects"][0]["failed"],0);
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
        let rollback_url=format!("/api/v1/services/{application_id}/rollback");
        let rollback_body=json!({"deployment_id":succeeded["id"]});
        assert_eq!(call(&app,&rollback_url,&member,Some(rollback_body.clone())).await.0,StatusCode::FORBIDDEN);
        assert_eq!(call(&app,&rollback_url,&owner,Some(json!({"deployment_id":failed["id"]}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(call(&app,&rollback_url,&owner,Some(json!({"deployment_id":Uuid::new_v4()}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(call(&app,&rollback_url,&owner,Some(rollback_body)).await.0,StatusCode::ACCEPTED);
        let restored=wait_status(&app,&owner,application_id,"succeeded").await;
        assert_eq!(restored["rollback_of"],succeeded["id"]);
        assert_eq!(restored["image_id"],succeeded["image_id"]);
        let snapshot:(Option<Vec<u8>>,i32)=sqlx::query_as("SELECT env_cipher,container_port FROM deployments WHERE id=$1").bind(restored["id"].as_str().unwrap().parse::<Uuid>().unwrap()).fetch_one(&pool).await.unwrap();
        assert_eq!(snapshot.0,Some(cipher.clone())); assert_eq!(snapshot.1,80);
        let restored_runtime=call(&app,&format!("/api/v1/applications/{application_id}/runtime"),&owner,None).await.1;
        nginx_responds(&state,restored_runtime["host_port"].as_i64().unwrap()).await;
        assert_eq!(call(&app,&format!("/api/v1/applications/{application_id}/logs"),&owner,None).await.0,StatusCode::OK);
        let create_service=format!("/api/v1/environments/{environment}/services");
        assert_eq!(call(&app,&create_service,&member,Some(json!({"kind":"database","name":"forbidden"}))).await.0,StatusCode::FORBIDDEN);
        let (code,db,_) = call(&app,&create_service,&owner,Some(json!({"kind":"database","name":"PostgreSQL"}))).await;
        assert_eq!(code,StatusCode::CREATED,"{db}");
        let database:Uuid=db["id"].as_str().unwrap().parse().unwrap();added_services.push(database);
        let config=call(&app,&format!("/api/v1/services/{database}/configuration"),&owner,None).await.1;
        assert_eq!(config["password"].as_str().unwrap().len(),64);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/configuration"),&member,None).await.0,StatusCode::FORBIDDEN);
        assert_eq!(call(&app,&format!("/api/v1/applications/{database}/configuration"),&owner,Some(json!({"name":"bad","image":"nginx:alpine","container_port":80}))).await.0,StatusCode::NOT_FOUND);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/environment"),&owner,Some(json!({"environment":"TZ=UTC"}))).await.0,StatusCode::NO_CONTENT);
        let db_deploy=format!("/api/v1/services/{database}/deploy");
        assert_eq!(call(&app,&db_deploy,&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
        let first=wait_status(&app,&owner,database,"succeeded").await;
        let first_container=first["container_id"].as_str().unwrap();
        let inspected=state.docker.inspect_container(first_container,None).await.unwrap();
        assert!(inspected.config.unwrap().env.unwrap().contains(&"TZ=UTC".to_owned()),"saved database environment must reach the deployed container");
        let output=tokio::process::Command::new("docker").args(["exec",first_container,"psql","-U","ferrix","-d","ferrix","-v","ON_ERROR_STOP=1","-c","CREATE TABLE persistence_check(value text); INSERT INTO persistence_check VALUES ('survives-redeploy');"]).output().await.unwrap();
        assert!(output.status.success(),"database initialization failed");
        let mut edit=config.clone();edit["password"]=json!("changed-password-is-not-allowed");
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/configuration"),&owner,Some(edit)).await.0,StatusCode::CONFLICT);
        assert_eq!(call(&app,&db_deploy,&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
        let second=wait_status(&app,&owner,database,"succeeded").await;
        assert_ne!(first["container_id"],second["container_id"]);
        let output=tokio::process::Command::new("docker").args(["exec",second["container_id"].as_str().unwrap(),"psql","-U","ferrix","-d","ferrix","-At","-c","SELECT value FROM persistence_check"]).output().await.unwrap();
        assert!(output.status.success());assert_eq!(String::from_utf8_lossy(&output.stdout).trim(),"survives-redeploy");
        let info=state.docker.inspect_container(second["container_id"].as_str().unwrap(),None).await.unwrap();
        assert!(info.host_config.unwrap().port_bindings.is_none_or(|p|p.is_empty()),"database must not publish a host port");
        // Ordinary organization admins cannot create a host-level Compose project.
        for kind in ["compose","static_website"] {
            assert_eq!(call(&app,&create_service,&owner,Some(json!({"kind":kind,"name":"removed"}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        }
        let services=call(&app,&create_service,&member,None).await.1;
        assert_eq!(services.as_array().unwrap().len(),2);
        assert_eq!(call(&app,&format!("/api/v1/environments/{environment}/applications"),&owner,None).await.1.as_array().unwrap().len(),1);

        assert_eq!(remove_service(&app,database,&member,"PostgreSQL",true).await,StatusCode::FORBIDDEN);
        assert_eq!(remove_service(&app,database,&owner,"wrong-name",true).await,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/control"),&member,Some(json!({"operation":"stop"}))).await.0,StatusCode::FORBIDDEN);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/environment"),&member,None).await.0,StatusCode::FORBIDDEN);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/environment"),&owner,Some(json!({"environment":"POSTGRES_PASSWORD=overwrite"}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/environment"),&owner,Some(json!({"environment":"TZ=UTC"}))).await.0,StatusCode::NO_CONTENT);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/environment"),&owner,None).await.1["environment"],"TZ=UTC");
        let states=call(&app,&format!("/api/v1/environments/{environment}/service-status"),&member,None).await.1;
        assert_eq!(states.as_array().unwrap().len(),2);
        assert!(states.as_array().unwrap().iter().all(|s|s["state"]=="running"),"{states}");
        // Other organizations cannot delete or discover this service.
        let other=call(&app,"/api/v1/auth/organizations",&owner,Some(json!({"name":"Deletion isolation"}))).await.1;
        call(&app,"/api/v1/auth/organizations/switch",&owner,Some(json!({"organization_id":other["id"]}))).await;
        assert_eq!(remove_service(&app,database,&owner,"PostgreSQL",true).await,StatusCode::NOT_FOUND);
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/control"),&owner,Some(json!({"operation":"stop"}))).await.0,StatusCode::NOT_FOUND);
        assert_eq!(call(&app,&format!("/api/v1/environments/{environment}/service-status"),&owner,None).await.0,StatusCode::NOT_FOUND);
        call(&app,"/api/v1/auth/organizations/switch",&owner,Some(json!({"organization_id":org}))).await;
        assert_eq!(call(&app,&format!("/api/v1/services/{database}/backups"),&member,Some(json!({}))).await.0,StatusCode::FORBIDDEN);
        backup_roundtrip(&app,&state,&owner,database,"postgres",&mut added_services).await;
        assert_eq!(remove_service(&app,database,&owner,"PostgreSQL",false).await,StatusCode::NO_CONTENT);
        assert!(state.docker.inspect_volume(&format!("ferrix-db-{}",database.simple())).await.is_ok(),"data is retained by default");
        assert!(state.docker.inspect_container(second["container_id"].as_str().unwrap(),None).await.is_err());
        assert_eq!(call(&app,&format!("/api/v1/services/{database}"),&owner,None).await.0,StatusCode::NOT_FOUND);
        let remaining:i64=sqlx::query_scalar("SELECT count(*) FROM deployments WHERE application_id=$1").bind(database).fetch_one(&pool).await.unwrap();
        assert_eq!(remaining,0);
        assert!(state.docker.inspect_network(&format!("ferrix-env-{}",environment.replace('-', "")),None).await.is_ok(),"shared environment network remains");
        let app_name:String=sqlx::query_scalar("SELECT name FROM applications WHERE id=$1").bind(application_id).fetch_one(&pool).await.unwrap();
        assert_eq!(remove_service(&app,application_id,&owner,&app_name,false).await,StatusCode::NO_CONTENT);

        for engine in ["mongodb","mariadb","redis"] {
            let (code,service,_) = call(&app,&create_service,&owner,Some(json!({"kind":"database","engine":engine,"name":engine}))).await;
            assert_eq!(code,StatusCode::CREATED,"{service}");
            let service:Uuid=service["id"].as_str().unwrap().parse().unwrap(); added_services.push(service);
            let endpoint=format!("/api/v1/services/{service}");
            let settings=call(&app,&format!("{endpoint}/configuration"),&owner,None).await.1;
            assert_eq!(settings["engine"],engine);
            assert_eq!(settings["password"].as_str().unwrap().len(),64);
            if engine=="mariadb" { assert_ne!(settings["password"],settings["root_password"]); }
            let uri=settings["connection_uri"].as_str().unwrap();
            assert!(uri.starts_with(match engine {"mongodb"=>"mongodb://","mariadb"=>"mysql://",_=>"redis://"}));
            if engine=="mongodb" {assert!(uri.contains("authSource=admin"));}
            let reserved=match engine {"mongodb"=>"MONGO_INITDB_ROOT_PASSWORD","mariadb"=>"MARIADB_ALLOW_EMPTY_ROOT_PASSWORD",_=>"REDIS_PASSWORD"};
            assert_eq!(call(&app,&format!("{endpoint}/environment"),&owner,Some(json!({"environment":format!("{reserved}=bad")}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
            let script=match engine {
                "mongodb"=>"mongosh --quiet --host 127.0.0.1 --username \"$MONGO_INITDB_ROOT_USERNAME\" --password \"$MONGO_INITDB_ROOT_PASSWORD\" --authenticationDatabase admin --eval 'db.getSiblingDB(\"ferrix\").persistence_check.insertOne({value:\"survives-redeploy\"})'",
                "mariadb"=>"mariadb -u\"$MARIADB_USER\" -p\"$MARIADB_PASSWORD\" \"$MARIADB_DATABASE\" -e 'CREATE TABLE persistence_check(value TEXT); INSERT INTO persistence_check VALUES (\"survives-redeploy\");'",
                _=>"REDISCLI_AUTH=\"$REDIS_PASSWORD\" redis-cli --no-auth-warning SET persistence_check survives-redeploy",
            };
            assert_eq!(call(&app,&format!("{endpoint}/deploy"),&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
            let first=wait_status(&app,&owner,service,"succeeded").await;
            let container=first["container_id"].as_str().unwrap();
            let output=tokio::process::Command::new("docker").args(["exec",container,"sh","-c",script]).output().await.unwrap();
            assert!(output.status.success(),"{engine}: authenticated write must succeed");
            let anonymous=match engine {"mongodb"=>"mongosh --quiet --eval 'db.getSiblingDB(\"ferrix\").persistence_check.findOne()'","mariadb"=>"mariadb -uferrix ferrix -e 'SELECT * FROM persistence_check'",_=>"redis-cli --no-auth-warning GET persistence_check"};
            let output=tokio::process::Command::new("docker").args(["exec",container,"sh","-c",anonymous]).output().await.unwrap();
            assert!(!output.status.success() || String::from_utf8_lossy(&output.stdout).contains("NOAUTH"),"{engine}: unauthenticated data access must fail");
            assert_eq!(call(&app,&format!("{endpoint}/deploy"),&owner,Some(json!({}))).await.0,StatusCode::ACCEPTED);
            let second=wait_status(&app,&owner,service,"succeeded").await;
            assert_ne!(first["container_id"],second["container_id"]);
            let script=match engine {
                "mongodb"=>"mongosh --quiet --host 127.0.0.1 --username \"$MONGO_INITDB_ROOT_USERNAME\" --password \"$MONGO_INITDB_ROOT_PASSWORD\" --authenticationDatabase admin --eval 'print(db.getSiblingDB(\"ferrix\").persistence_check.findOne().value)'",
                "mariadb"=>"mariadb -u\"$MARIADB_USER\" -p\"$MARIADB_PASSWORD\" \"$MARIADB_DATABASE\" -N -e 'SELECT value FROM persistence_check'",
                _=>"REDISCLI_AUTH=\"$REDIS_PASSWORD\" redis-cli --no-auth-warning GET persistence_check",
            };
            let output=tokio::process::Command::new("docker").args(["exec",second["container_id"].as_str().unwrap(),"sh","-c",script]).output().await.unwrap();
            assert!(output.status.success(),"{engine}: authenticated read must succeed");
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(),"survives-redeploy","{engine}: data must survive replacement");
            let info=state.docker.inspect_container(second["container_id"].as_str().unwrap(),None).await.unwrap();
            assert!(info.host_config.unwrap().port_bindings.is_none_or(|p|p.is_empty()));
            if engine=="mariadb" { backup_roundtrip(&app,&state,&owner,service,engine,&mut added_services).await; }
            else { assert_eq!(call(&app,&format!("{endpoint}/backups"),&owner,None).await.0,StatusCode::UNPROCESSABLE_ENTITY); }
            assert_eq!(remove_service(&app,service,&owner,engine,true).await,StatusCode::NO_CONTENT);
            assert!(state.docker.inspect_volume(&format!("ferrix-db-{}",service.simple())).await.is_err());
            if engine=="mongodb" {assert!(state.docker.inspect_volume(&format!("ferrix-db-{}-config",service.simple())).await.is_err());}
        }
        for engine in ["mysql","libsql"] {
            assert_eq!(call(&app,&create_service,&owner,Some(json!({"kind":"database","engine":engine,"name":"unsupported"}))).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        }

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
    for service in added_services {
        let options = bollard::query_parameters::ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&std::collections::HashMap::from([(
                "label".to_owned(),
                vec![format!("io.ferrix.application={service}")],
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
        let volumes = state
            .docker
            .list_volumes(Some(
                bollard::query_parameters::ListVolumesOptionsBuilder::default()
                    .filters(&std::collections::HashMap::from([(
                        "label".to_owned(),
                        vec![
                            format!("io.ferrix.application={service}"),
                            "io.ferrix.managed=true".into(),
                        ],
                    )]))
                    .build(),
            ))
            .await
            .unwrap_or_default();
        for volume in volumes.volumes.unwrap_or_default() {
            let _ = state
                .docker
                .remove_volume(
                    &volume.name,
                    None::<bollard::query_parameters::RemoveVolumeOptions>,
                )
                .await;
        }
        let networks = state
            .docker
            .list_networks(Some(
                bollard::query_parameters::ListNetworksOptionsBuilder::default()
                    .filters(&std::collections::HashMap::from([(
                        "label".to_owned(),
                        vec![format!("io.ferrix.application={service}")],
                    )]))
                    .build(),
            ))
            .await
            .unwrap_or_default();
        for network in networks {
            if let Some(id) = network.id {
                let _ = state.docker.remove_network(&id).await;
            }
        }
    }
    for environment in owned_environments {
        let _ = state
            .docker
            .remove_network(&format!("ferrix-env-{}", environment.simple()))
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
