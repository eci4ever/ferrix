use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ferrix_api::{AppState, router};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn app() -> axum::Router {
    router(AppState {
        pool: PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap(),
        docker: bollard::Docker::connect_with_local_defaults().unwrap(),
    })
}

#[tokio::test]
async fn liveness_does_not_require_external_services() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["service"], "ferrix");
}

#[tokio::test]
async fn invalid_projects_are_rejected_before_database_access() {
    for name in ["   ".to_owned(), "a".repeat(81)] {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({"name":name}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn database_project_round_trip() {
    let Ok(url) = std::env::var("FERRIX_TEST_DATABASE_URL") else {
        eprintln!("Set FERRIX_TEST_DATABASE_URL to run the PostgreSQL integration test.");
        return;
    };
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        docker: bollard::Docker::connect_with_local_defaults().unwrap(),
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/projects")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"  Integration test  ","description":"Persistent project"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let project: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(project["name"], "Integration test");
    let id = project["id"]
        .as_str()
        .unwrap()
        .parse::<uuid::Uuid>()
        .unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let projects: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(projects.iter().any(|item| item["id"] == project["id"]));
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
