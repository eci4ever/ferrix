use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use ferrix_api::{AppState, auth::AuthConfig, router};
use futures_util::FutureExt;
use serde_json::{Value, json};
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};
use std::{panic::AssertUnwindSafe, sync::Arc};
use tower::ServiceExt;
use uuid::Uuid;
const ORIGIN: &str = "http://127.0.0.1:5173";
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, Value, HeaderMap) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("cookie", cookie)
                .header("origin", ORIGIN)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1_048_576).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
    )
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn outcome_notifications_are_atomic_scoped_and_individually_read() {
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("notifications_test_{}", Uuid::new_v4().simple());
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
    let result = AssertUnwindSafe(async {
        sqlx::migrate!().run(&pool).await.unwrap();
        let app = router(AppState {
            pool: pool.clone(),
            gateway: Default::default(),
            docker: bollard::Docker::connect_with_local_defaults().unwrap(),
            auth: Arc::new(AuthConfig {
                origin: ORIGIN.into(),
                secure: false,
                cookie_name: "notifications_test_session".into(),
                key: [42; 32],
            }),
        });
        check(&app, &pool).await;
    })
    .catch_unwind()
    .await;
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}
async fn check(app: &Router, pool: &PgPool) {
    let(status,_,headers)=call(app,"POST","/api/v1/auth/setup","",json!({"name":"Notification owner","email":"notify@example.com","password":"Long disposable notification password!"})).await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers["set-cookie"].to_str().unwrap();
    let (_, identity, _) = call(app, "GET", "/api/v1/auth/session", cookie, Value::Null).await;
    let org: Uuid = identity["organization_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let (_, project, _) = call(
        app,
        "POST",
        "/api/v1/projects",
        cookie,
        json!({"name":"Notification project"}),
    )
    .await;
    let (_, environments, _) = call(
        app,
        "GET",
        &format!(
            "/api/v1/projects/{}/environments",
            project["id"].as_str().unwrap()
        ),
        cookie,
        Value::Null,
    )
    .await;
    let (_,service,_)=call(app,"POST",&format!("/api/v1/environments/{}/services",environments["environments"][0]["id"].as_str().unwrap()),cookie,json!({"kind":"application","name":"Notify service","image":"nginx:alpine","container_port":80,"environment":""})).await;
    let service: Uuid = service["id"].as_str().unwrap().parse().unwrap();
    let deployment = Uuid::new_v4();
    sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,status) VALUES($1,$2,'nginx:alpine',80,'queued')").bind(deployment).bind(service).execute(pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE deployments SET status='failed' WHERE id=$1")
        .bind(deployment)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let (_, inbox, _) = call(app, "GET", "/api/v1/notifications", cookie, Value::Null).await;
    assert_eq!(inbox["unread"], 0);
    sqlx::query("UPDATE deployments SET status='succeeded' WHERE id=$1")
        .bind(deployment)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE deployments SET status='succeeded' WHERE id=$1")
        .bind(deployment)
        .execute(pool)
        .await
        .unwrap();
    let (_, inbox, _) = call(app, "GET", "/api/v1/notifications", cookie, Value::Null).await;
    assert_eq!(inbox["unread"], 1);
    assert_eq!(inbox["notifications"][0]["kind"], "deployment_succeeded");
    assert!(!inbox.to_string().contains("nginx:alpine"));
    let event = inbox["notifications"][0]["id"].as_str().unwrap();
    assert_eq!(
        call(
            app,
            "POST",
            &format!("/api/v1/notifications/{event}/read"),
            cookie,
            json!({})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(app, "GET", "/api/v1/notifications", cookie, Value::Null)
            .await
            .1["unread"],
        0
    );
    let foreign = Uuid::new_v4();
    let foreign_project = Uuid::new_v4();
    let foreign_event = Uuid::new_v4();
    sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,'Foreign organization')")
        .bind(foreign)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects(id,name,organization_id) VALUES($1,'Foreign project',$2)")
        .bind(foreign_project)
        .bind(foreign)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO notifications(id,organization_id,project_id,kind,title,message) VALUES($1,$2,$3,'resource_high','Private event','Must not leak')").bind(foreign_event).bind(foreign).bind(foreign_project).execute(pool).await.unwrap();
    assert_eq!(
        call(
            app,
            "POST",
            &format!("/api/v1/notifications/{foreign_event}/read"),
            cookie,
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert!(
        !call(app, "GET", "/api/v1/notifications", cookie, Value::Null)
            .await
            .1
            .to_string()
            .contains("Must not leak")
    );
    for operation in ["backup", "restore"] {
        let job = Uuid::new_v4();
        sqlx::query("INSERT INTO database_jobs(id,application_id,operation,engine,image,image_id,config_cipher,status) VALUES($1,$2,$3,'postgres','postgres:17','sha256:fixture','\\x00','running')").bind(job).bind(service).bind(operation).execute(pool).await.unwrap();
        sqlx::query("UPDATE database_jobs SET status='failed',message='secret-error-not-for-inbox' WHERE id=$1").bind(job).execute(pool).await.unwrap();
    }
    let (_, inbox, _) = call(app, "GET", "/api/v1/notifications", cookie, Value::Null).await;
    assert_eq!(inbox["unread"], 2);
    assert!(!inbox.to_string().contains("secret-error"));
    assert_eq!(
        call(
            app,
            "POST",
            "/api/v1/notifications/read-all",
            cookie,
            json!({})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(app, "GET", "/api/v1/notifications", cookie, Value::Null)
            .await
            .1["unread"],
        0
    );
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM notification_reads r JOIN notifications n ON n.id=r.notification_id WHERE n.organization_id=$1").bind(org).fetch_one(pool).await.unwrap();
    assert_eq!(count, 3);
    sqlx::query(
        "UPDATE notifications SET expires_at=now()-interval '1 second' WHERE organization_id=$1",
    )
    .bind(org)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(
        call(app, "GET", "/api/v1/notifications", cookie, Value::Null)
            .await
            .1["notifications"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}
