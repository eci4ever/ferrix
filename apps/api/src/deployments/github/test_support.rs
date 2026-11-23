use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use sqlx::{Executor, postgres::PgPoolOptions};
use std::sync::Arc;
use tower::ServiceExt;
pub(super) const ORIGIN: &str = "http://127.0.0.1:5173";
pub(super) struct Fixture {
    pub state: AppState,
    pub app: Router,
    pub cookie: String,
    pub service: Uuid,
    pub org: Uuid,
    pub user: Uuid,
    pub admin: sqlx::PgPool,
    pub schema: String,
}
pub(super) struct Reply {
    pub status: StatusCode,
    pub body: Value,
    pub headers: HeaderMap,
}
pub(super) async fn call(
    app: &Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
    extra: &[(&str, String)],
    origin: bool,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if !cookie.is_empty() {
        request = request.header("cookie", cookie)
    }
    if origin {
        request = request.header("origin", ORIGIN)
    }
    for (name, value) in extra {
        request = request.header(*name, value)
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(Body::from(if let Some(body) = body.as_str() {
                    body.to_owned()
                } else {
                    body.to_string()
                }))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}
impl Fixture {
    pub(super) async fn new() -> Self {
        let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
        let admin = PgPoolOptions::new().connect(&url).await.unwrap();
        let schema = format!("github_test_{}", Uuid::new_v4().simple());
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
        sqlx::migrate!().run(&pool).await.unwrap();
        let state = AppState {
            pool,
            gateway: Default::default(),
            docker: bollard::Docker::connect_with_defaults().unwrap(),
            auth: Arc::new(crate::auth::AuthConfig {
                origin: ORIGIN.into(),
                secure: false,
                cookie_name: "github_test_session".into(),
                key: [42; 32],
            }),
        };
        let app = crate::router(state.clone());
        let reply=call(&app,"POST","/api/v1/auth/setup","",json!({"name":"Git owner","email":"git-owner@example.com","password":"Long disposable GitHub password!"}),&[],true).await;
        assert_eq!(reply.status, StatusCode::OK);
        let cookie = reply.headers["set-cookie"].to_str().unwrap().to_owned();
        let identity = call(
            &app,
            "GET",
            "/api/v1/auth/session",
            &cookie,
            Value::Null,
            &[],
            false,
        )
        .await
        .body;
        let org = Uuid::parse_str(identity["organization_id"].as_str().unwrap()).unwrap();
        let user = Uuid::parse_str(identity["user_id"].as_str().unwrap()).unwrap();
        let project = call(
            &app,
            "POST",
            "/api/v1/projects",
            &cookie,
            json!({"name":"Git project"}),
            &[],
            true,
        )
        .await
        .body;
        let project = Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
        let env = call(
            &app,
            "GET",
            &format!("/api/v1/projects/{project}/environments"),
            &cookie,
            Value::Null,
            &[],
            false,
        )
        .await
        .body;
        let environment = Uuid::parse_str(env["environments"][0]["id"].as_str().unwrap()).unwrap();
        let service=call(&app,"POST",&format!("/api/v1/environments/{environment}/services"),&cookie,json!({"kind":"application","name":"Git web","image":"nginx:alpine","container_port":80,"environment":""}),&[],true).await;
        assert_eq!(service.status, StatusCode::CREATED);
        let service = Uuid::parse_str(service.body["id"].as_str().unwrap()).unwrap();
        Self {
            state,
            app,
            cookie,
            service,
            org,
            user,
            admin,
            schema,
        }
    }
    pub(super) async fn request(&self, method: &str, path: &str, body: Value) -> Reply {
        call(
            &self.app,
            method,
            path,
            &self.cookie,
            body,
            &[],
            method != "GET",
        )
        .await
    }
    pub(super) async fn finish(self) {
        self.state.pool.close().await;
        self.admin
            .execute(format!("DROP SCHEMA {} CASCADE", self.schema).as_str())
            .await
            .unwrap();
    }
}
