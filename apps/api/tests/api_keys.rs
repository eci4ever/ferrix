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
const PASSWORD: &str = "Long disposable API key password!";
struct Reply {
    status: StatusCode,
    body: Value,
    headers: HeaderMap,
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    body: Value,
    cookie: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if !cookie.is_empty() {
        builder = builder.header("cookie", cookie);
    }
    if let Some(value) = bearer {
        builder = builder.header("authorization", format!("Bearer {value}"));
    }
    if let Some(value) = origin {
        builder = builder.header("origin", value);
    }
    let response = app
        .clone()
        .oneshot(
            builder
                .body(if method == "GET" {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1_048_576).await.unwrap();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}
async fn session(app: &Router, path: &str, body: Value, cookie: &str) -> Reply {
    request(app, "POST", path, body, cookie, None, Some(ORIGIN)).await
}
async fn key(app: &Router, method: &str, path: &str, body: Value, value: &str) -> Reply {
    request(app, method, path, body, "", Some(value), None).await
}
async fn mint(app: &Router, cookie: &str, scopes: Value) -> (Uuid, String) {
    let response = session(
        app,
        "/api/v1/auth/api-keys",
        json!({"name":"Test agent","scopes":scopes,"expires_in_days":7}),
        cookie,
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED);
    (
        response.body["id"].as_str().unwrap().parse().unwrap(),
        response.body["key"].as_str().unwrap().to_owned(),
    )
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn scoped_agent_keys_lifecycle_permissions_and_security() {
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("api_key_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let namespace = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _| {
            let query = format!("SET search_path TO {namespace}");
            Box::pin(async move {
                connection.execute(query.as_str()).await?;
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
                cookie_name: "api_key_test_session".into(),
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
        std::panic::resume_unwind(error);
    }
}
async fn check(app: &Router, pool: &PgPool) {
    let setup = session(
        app,
        "/api/v1/auth/setup",
        json!({"name":"Key owner","email":"keys@example.com","password":PASSWORD}),
        "",
    )
    .await;
    assert_eq!(setup.status, StatusCode::OK);
    let cookie = setup.headers["set-cookie"].to_str().unwrap();
    let identity = request(
        app,
        "GET",
        "/api/v1/auth/session",
        Value::Null,
        cookie,
        None,
        None,
    )
    .await
    .body;
    let user: Uuid = identity["user_id"].as_str().unwrap().parse().unwrap();
    let org: Uuid = identity["organization_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        session(
            app,
            "/api/v1/auth/api-keys",
            json!({"name":"Unknown","scopes":["read","admin"],"expires_in_days":7}),
            cookie
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        session(
            app,
            "/api/v1/auth/api-keys",
            json!({"name":"Never expires","scopes":["read"],"expires_in_days":0}),
            cookie
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(session(app,"/api/v1/auth/api-keys",json!({"name":"Foreign org","scopes":["read"],"expires_in_days":7,"organization_id":Uuid::new_v4()}),cookie).await.status,StatusCode::UNPROCESSABLE_ENTITY);
    sqlx::query("UPDATE auth_sessions SET authenticated_at=now()-interval '1 hour'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        session(
            app,
            "/api/v1/auth/api-keys",
            json!({"name":"Stale","scopes":["read"],"expires_in_days":7}),
            cookie
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        session(
            app,
            "/api/v1/auth/reauthenticate",
            json!({"password":PASSWORD}),
            cookie
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    let (read_id, read) = mint(app, cookie, json!(["read"])).await;
    let (write_id, write) = mint(app, cookie, json!(["read", "manage", "deploy"])).await;
    let stored: String = sqlx::query_scalar("SELECT key_hash FROM auth_api_keys WHERE id=$1")
        .bind(write_id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(stored != write && stored.len() == 64);
    let listed = request(
        app,
        "GET",
        "/api/v1/auth/api-keys",
        Value::Null,
        cookie,
        None,
        None,
    )
    .await;
    assert_eq!(listed.body["keys"].as_array().unwrap().len(), 2);
    assert!(!listed.body.to_string().contains(&write));
    assert!(!listed.body.to_string().contains("key_hash"));
    let discovery = key(app, "GET", "/api/v1/agent", Value::Null, &write).await;
    assert_eq!(discovery.status, StatusCode::OK);
    assert_eq!(discovery.body["organization_id"], org.to_string());
    assert!(!discovery.headers.contains_key("set-cookie"));
    assert_eq!(discovery.headers["cache-control"], "no-store");
    for path in [
        "/api/v1/auth/session",
        "/api/v1/auth/admin/users",
        "/api/v1/auth/api-keys",
        "/api/v1/containers",
        "/api/v1/docker/images",
        "/api/v1/monitoring",
    ] {
        assert_eq!(
            key(app, "GET", path, Value::Null, &write).await.status,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        key(app, "POST", "/api/v1/auth/api-keys", json!({}), &write)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, "not-a-key")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            app,
            "GET",
            "/api/v1/projects",
            Value::Null,
            cookie,
            Some(&write),
            None
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            app,
            "POST",
            "/api/v1/projects",
            json!({"name":"Wrong origin"}),
            "",
            Some(&write),
            Some("https://attacker.example")
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            app,
            "POST",
            "/api/v1/projects",
            json!({"name":"Cookie needs origin"}),
            cookie,
            None,
            None
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        key(
            app,
            "POST",
            "/api/v1/projects",
            json!({"name":"Read only"}),
            &read
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let duplicate = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/projects")
                .header("authorization", format!("Bearer {write}"))
                .header("authorization", format!("Bearer {read}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        request(
            app,
            "GET",
            &format!("/api/v1/projects?api_key={write}"),
            Value::Null,
            "",
            None,
            None
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    let project = key(
        app,
        "POST",
        "/api/v1/projects",
        json!({"name":"Agent project"}),
        &write,
    )
    .await;
    assert_eq!(project.status, StatusCode::CREATED);
    assert!(!project.headers.contains_key("set-cookie"));
    let pid = project.body["id"].as_str().unwrap();
    let environments = key(
        app,
        "GET",
        &format!("/api/v1/projects/{pid}/environments"),
        Value::Null,
        &write,
    )
    .await;
    assert_eq!(environments.status, StatusCode::OK);
    let env = environments.body["environments"][0]["id"].as_str().unwrap();
    let app_config = json!({"kind":"application","name":"Agent web","image":"nginx:alpine","container_port":80,"environment":"PRIVATE_VALUE=test-secret-do-not-log"});
    let application = key(
        app,
        "POST",
        &format!("/api/v1/environments/{env}/services"),
        app_config,
        &write,
    )
    .await;
    assert_eq!(application.status, StatusCode::CREATED);
    let service = application.body["id"].as_str().unwrap();
    assert_eq!(
        key(
            app,
            "GET",
            &format!("/api/v1/services/{service}/configuration"),
            Value::Null,
            &write
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        key(
            app,
            "GET",
            &format!("/api/v1/services/{service}/environment"),
            Value::Null,
            &read
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let (_, secrets) = mint(app, cookie, json!(["read", "secrets:read"])).await;
    assert_eq!(
        key(
            app,
            "GET",
            &format!("/api/v1/services/{service}/configuration"),
            Value::Null,
            &secrets
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        key(
            app,
            "POST",
            &format!("/api/v1/services/{service}/deploy"),
            json!({}),
            &read
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        key(
            app,
            "POST",
            &format!("/api/v1/services/{service}/deploy"),
            json!({}),
            &write
        )
        .await
        .status,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        key(
            app,
            "DELETE",
            &format!("/api/v1/projects/{pid}"),
            json!({"name":"Agent project"}),
            &write
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let (_, delete_key) = mint(app, cookie, json!(["read", "manage", "delete"])).await;
    let disposable = key(
        app,
        "POST",
        "/api/v1/projects",
        json!({"name":"Agent disposable"}),
        &write,
    )
    .await
    .body;
    assert_eq!(
        key(
            app,
            "DELETE",
            &format!("/api/v1/projects/{}", disposable["id"].as_str().unwrap()),
            json!({"name":"Agent disposable"}),
            &delete_key
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    // A bound key does not follow the browser's active organization.
    let other = session(
        app,
        "/api/v1/auth/organizations",
        json!({"name":"Another org"}),
        cookie,
    )
    .await
    .body;
    let other_id = other["id"].as_str().unwrap();
    session(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":other_id}),
        cookie,
    )
    .await;
    let other_project = session(
        app,
        "/api/v1/projects",
        json!({"name":"Other project"}),
        cookie,
    )
    .await
    .body;
    let foreign = other_project["id"].as_str().unwrap();
    let visible = key(app, "GET", "/api/v1/projects", Value::Null, &write).await;
    assert_eq!(visible.body.as_array().unwrap().len(), 1);
    assert_eq!(visible.body[0]["id"], pid);
    assert_eq!(
        key(
            app,
            "GET",
            &format!("/api/v1/projects/{foreign}/environments"),
            Value::Null,
            &write
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    session(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":org}),
        cookie,
    )
    .await;
    sqlx::query("INSERT INTO auth_api_key_requests(id,key_id,user_id,organization_id,method,route,status,ip_address,expires_at) VALUES($1,$2,$3,$4,'GET','/expired-record',200,'local',now()-interval '1 day')").bind(Uuid::new_v4()).bind(write_id).bind(user).bind(org).execute(pool).await.unwrap();
    let usage = request(
        app,
        "GET",
        &format!("/api/v1/auth/api-keys/{write_id}/usage"),
        Value::Null,
        cookie,
        None,
        None,
    )
    .await;
    assert_eq!(usage.status, StatusCode::OK);
    let text = usage.body.to_string();
    assert!(!text.contains(&write));
    assert!(!text.contains("test-secret-do-not-log"));
    assert!(!text.contains("authorization"));
    assert!(!text.contains("/expired-record"));
    assert!(usage.body["requests"].as_array().unwrap().iter().any(|event|event["route"]=="/api/v1/services/{id}/deploy" && event["status"]==202));
    sqlx::query(
        "UPDATE auth_api_keys SET rate_count=119,rate_window=now()+interval '1 hour' WHERE id=$1",
    )
    .bind(read_id)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &read)
            .await
            .status,
        StatusCode::OK
    );
    sqlx::query(
        "UPDATE auth_api_keys SET rate_count=120,rate_window=now()+interval '1 hour' WHERE id=$1",
    )
    .bind(read_id)
    .execute(pool)
    .await
    .unwrap();
    let limited = key(app, "GET", "/api/v1/projects", Value::Null, &read).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers.contains_key("retry-after"));
    sqlx::query("UPDATE auth_api_keys SET rate_window=now()-interval '2 minutes' WHERE id=$1")
        .bind(read_id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &read)
            .await
            .status,
        StatusCode::OK
    );
    // Current membership role is authoritative, not the role at key creation.
    sqlx::query("UPDATE auth_members SET role='member' WHERE user_id=$1 AND organization_id=$2")
        .bind(user)
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        key(
            app,
            "POST",
            "/api/v1/projects",
            json!({"name":"No escalation"}),
            &write
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &write)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        key(app, "GET", "/api/v1/agent", Value::Null, &write)
            .await
            .body["scopes"],
        json!(["read"])
    );
    assert_eq!(session(app,"/api/v1/auth/api-keys",json!({"name":"Member cannot mint write","scopes":["read","manage"],"expires_in_days":7}),cookie).await.status,StatusCode::FORBIDDEN);
    let (_, member_read) = mint(app, cookie, json!(["read"])).await;
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &member_read)
            .await
            .status,
        StatusCode::OK
    );
    sqlx::query("UPDATE auth_members SET role='owner' WHERE user_id=$1 AND organization_id=$2")
        .bind(user)
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE auth_api_keys SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(read_id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &read)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        session(
            app,
            &format!("/api/v1/auth/api-keys/{write_id}/revoke"),
            json!({}),
            cookie
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &write)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let (_, password_key) = mint(app, cookie, json!(["read"])).await;
    assert_eq!(
        session(
            app,
            "/api/v1/auth/change-password",
            json!({"password":PASSWORD,"new_password":PASSWORD}),
            cookie
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &password_key)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // Removing and restoring membership cannot resurrect a key.
    let login = session(
        app,
        "/api/v1/auth/sign-in",
        json!({"email":"keys@example.com","password":PASSWORD}),
        "",
    )
    .await;
    let cookie = login.headers["set-cookie"].to_str().unwrap();
    session(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":org}),
        cookie,
    )
    .await;
    let (_, member_key) = mint(app, cookie, json!(["read"])).await;
    sqlx::query("DELETE FROM auth_members WHERE user_id=$1 AND organization_id=$2")
        .bind(user)
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &member_key)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("INSERT INTO auth_members(user_id,organization_id,role) VALUES($1,$2,'owner')")
        .bind(user)
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &member_key)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // Platform users who are organization admins can deploy; keys never require system-admin status.
    let delegated = Uuid::new_v4();
    sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) SELECT $1,'Delegated admin','delegated@example.com',password_hash FROM auth_users WHERE id=$2").bind(delegated).bind(user).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO auth_members(user_id,organization_id,role) VALUES($1,$2,'admin')")
        .bind(delegated)
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    let delegated_login = session(
        app,
        "/api/v1/auth/sign-in",
        json!({"email":"delegated@example.com","password":PASSWORD}),
        "",
    )
    .await;
    let delegated_cookie = delegated_login.headers["set-cookie"].to_str().unwrap();
    let (delegated_id, delegated_key) =
        mint(app, delegated_cookie, json!(["read", "manage", "deploy"])).await;
    assert_eq!(
        key(
            app,
            "POST",
            "/api/v1/projects",
            json!({"name":"Delegated project"}),
            &delegated_key
        )
        .await
        .status,
        StatusCode::CREATED
    );
    assert_eq!(
        request(
            app,
            "GET",
            &format!("/api/v1/auth/api-keys/{delegated_id}/usage"),
            Value::Null,
            cookie,
            None,
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        session(
            app,
            &format!("/api/v1/auth/api-keys/{delegated_id}/revoke"),
            json!({}),
            cookie
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(session(app,&format!("/api/v1/auth/admin/users/{delegated}/ban"),json!({"banned":true,"reason":"Revoke automation","expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339()}),cookie).await.status,StatusCode::NO_CONTENT);
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &delegated_key)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE auth_users SET ban_expires_at=now()-interval '1 minute' WHERE id=$1")
        .bind(delegated)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &delegated_key)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let delegated_login = session(
        app,
        "/api/v1/auth/sign-in",
        json!({"email":"delegated@example.com","password":PASSWORD}),
        "",
    )
    .await;
    let delegated_cookie = delegated_login.headers["set-cookie"].to_str().unwrap();
    let (_, reset_key) = mint(app, delegated_cookie, json!(["read"])).await;
    let reset = session(
        app,
        &format!("/api/v1/auth/admin/users/{delegated}/password-reset"),
        json!({}),
        cookie,
    )
    .await;
    assert_eq!(
        session(
            app,
            "/api/v1/auth/reset-password",
            json!({"token":reset.body["token"],"new_password":PASSWORD}),
            ""
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &reset_key)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let (_, admin_revoked) = mint(app, cookie, json!(["read"])).await;
    assert_eq!(
        session(
            app,
            &format!("/api/v1/auth/admin/users/{user}/revoke-api-keys"),
            json!({}),
            cookie
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        key(app, "GET", "/api/v1/projects", Value::Null, &admin_revoked)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..20 {
        mint(app, cookie, json!(["read"])).await;
    }
    assert_eq!(
        session(
            app,
            "/api/v1/auth/api-keys",
            json!({"name":"Over limit","scopes":["read"],"expires_in_days":7}),
            cookie
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
