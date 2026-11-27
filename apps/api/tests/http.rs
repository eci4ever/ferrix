use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ferrix_api::{AppState, auth::AuthConfig, router};
use serde_json::{Value, json};
use sqlx::{Executor, postgres::PgPoolOptions};
use std::sync::Arc;
use tower::ServiceExt;
const ORIGIN: &str = "http://127.0.0.1:5173";
const PASSWORD: &str = "A sufficiently long test password!";
fn config() -> Arc<AuthConfig> {
    Arc::new(AuthConfig {
        origin: ORIGIN.into(),
        secure: false,
        cookie_name: "ferrix_test_session".into(),
        key: [42; 32],
    })
}
fn app(pool: sqlx::PgPool) -> axum::Router {
    router(AppState {
        gateway: Default::default(),
        pool,
        docker: bollard::Docker::connect_with_local_defaults().unwrap(),
        auth: config(),
    })
}
async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Value,
    cookie: &str,
    origin: Option<&str>,
) -> (StatusCode, Value, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if !cookie.is_empty() {
        builder = builder.header("cookie", cookie)
    }
    if let Some(origin) = origin {
        builder = builder.header("origin", origin)
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
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body, cookie)
}
async fn post(
    app: &axum::Router,
    path: &str,
    body: Value,
    cookie: &str,
) -> (StatusCode, Value, String) {
    call(app, "POST", path, body, cookie, Some(ORIGIN)).await
}
async fn get(app: &axum::Router, path: &str, cookie: &str) -> (StatusCode, Value, String) {
    call(app, "GET", path, Value::Null, cookie, None).await
}
#[tokio::test]
async fn liveness_and_unauthenticated_access() {
    let app = app(PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap());
    assert_eq!(get(&app, "/health", "").await.0, StatusCode::OK);
    assert_eq!(
        get(&app, "/api/v1/releases/update", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&app, "/api/v1/releases/update", json!({}), "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, "/api/v1/releases/latest", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(
            &app,
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/logs",
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/restart",
            json!({"version":1}),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/restart",
            json!({"version":1}),
            "",
            Some("https://evil.example")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/scale",
            json!({"replicas":2,"version":1}),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/scale",
            json!({"replicas":2,"version":1}),
            "",
            Some("https://evil.example")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/api/v1/projects/service-status", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, "/api/v1/monitoring", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, "/api/v1/server-time", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, "/api/v1/projects", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, "/api/v1/containers", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&app, "/api/v1/projects", json!({"name":"test"}), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "POST", "/api/v1/projects", json!({}), "", None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL pointing to your own development PostgreSQL"]
async fn postgres_auth_security_and_tenant_isolation() {
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").expect("set FERRIX_TEST_DATABASE_URL");
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("auth_test_{}", uuid::Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let namespace = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(5)
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
    let app = app(pool.clone());
    assert_eq!(
        get(&app, "/api/v1/home-summary", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, setup, _) = get(&app, "/api/v1/auth/setup", "").await;
    assert_eq!(setup["setup_required"], true);
    let registration = json!({"name":"Owner","email":" OWNER@example.com ","password":PASSWORD});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/setup",
            registration.clone(),
            "",
            Some("https://evil.example")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (first, second) = tokio::join!(
        post(&app, "/api/v1/auth/setup", registration.clone(), ""),
        post(&app, "/api/v1/auth/setup", registration.clone(), "")
    );
    let ((status, _, owner), loser) = if first.0 == StatusCode::OK {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(loser.0, StatusCode::CONFLICT);
    assert_eq!(status, StatusCode::OK);
    assert!(owner.contains("HttpOnly"));
    assert!(owner.contains("SameSite=Lax"));
    assert!(owner.contains("Max-Age=259200"));
    assert_eq!(
        post(&app, "/api/v1/auth/setup", registration, "").await.0,
        StatusCode::CONFLICT
    );
    let (_, identity, _) = get(&app, "/api/v1/auth/session", &owner).await;
    assert_eq!(
        get(&app, "/api/v1/auth/profile", "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/profile",
            json!({"name":"Owner", "avatar":"initials", "email":"changed@example.com"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        get(&app, "/api/v1/auth/profile", &owner).await.1["email"],
        "owner@example.com"
    );
    assert_eq!(
        post(
            &app,
            &format!(
                "/api/v1/auth/sessions/{}/revoke",
                identity["session_id"].as_str().unwrap()
            ),
            json!({}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        get(&app, "/api/v1/auth/session", &owner).await.0,
        StatusCode::OK
    );
    let saved = post(
        &app,
        "/api/v1/auth/profile",
        json!({"name":"Updated Owner","avatar":"🚀"}),
        &owner,
    )
    .await;
    assert_eq!(saved.0, StatusCode::NO_CONTENT);
    assert_eq!(post(&app, "/api/v1/auth/profile", json!({"name":"Updated Owner","avatar":"🚀","avatar_url":"https://example.com/photo.png"}), &owner).await.0, StatusCode::NO_CONTENT);
    let profile = get(&app, "/api/v1/auth/profile", &owner).await.1;
    assert_eq!(profile["avatar_url"], "https://example.com/photo.png");
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/profile",
            json!({"name":"Owner","avatar":"initials","avatar_url":"javascript:alert(1)"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(profile["name"], "Updated Owner");
    assert_eq!(profile["avatar"], "🚀");
    assert_eq!(
        get(&app, "/api/v1/auth/session", &owner).await.1["name"],
        "Updated Owner"
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/profile",
            json!({"name":" ","avatar":"🚀"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/profile",
            json!({"name":"Owner","avatar":"bad"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    post(
        &app,
        "/api/v1/auth/profile",
        json!({"name":"Owner","avatar":"initials"}),
        &owner,
    )
    .await;
    let owner_id = identity["user_id"].as_str().unwrap();
    let org1 = identity["organization_id"].as_str().unwrap();
    assert_eq!(identity["email"], "owner@example.com");
    assert_eq!(identity["role"], "owner");
    assert_eq!(identity["system_role"], "admin");
    check_platform_settings(&app, &pool, &owner).await;
    check_platform_organizations(&app, &pool, &owner).await;
    let (monitor_status, monitoring, _) = get(&app, "/api/v1/monitoring", &owner).await;
    assert_eq!(monitor_status, StatusCode::OK);
    assert_eq!(monitoring["interval_seconds"], 5);
    assert!(monitoring["samples"].as_array().unwrap().len() <= 180);
    assert!(monitoring["scope"].as_str().unwrap().contains("namespace"));
    let admin_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auth_users WHERE system_role='admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(admin_count, 1);
    assert_eq!(
        get(&app, "/api/v1/auth/setup", "").await.1["setup_required"],
        false
    );
    let stored: String =
        sqlx::query_scalar("SELECT password_hash FROM auth_users WHERE email='owner@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(stored.starts_with("$argon2id$"));
    let token_hash: String = sqlx::query_scalar("SELECT token_hash FROM auth_sessions LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!owner.contains(&token_hash));
    assert_eq!(
        post(&app, "/api/v1/projects", json!({"name":" "}), &owner)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/projects",
            json!({"name":"Organization one project"}),
            &owner
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (status, org2, _) = post(
        &app,
        "/api/v1/auth/organizations",
        json!({"name":"Second organization"}),
        &owner,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let org2 = org2["id"].as_str().unwrap();
    let runtime_summary = get(&app, "/api/v1/projects/service-status", &owner).await;
    assert_eq!(runtime_summary.0, StatusCode::OK);
    assert_eq!(runtime_summary.1["projects"].as_array().unwrap().len(), 1);
    assert_eq!(runtime_summary.1["projects"][0]["available"], true);
    assert_eq!(runtime_summary.1["projects"][0]["running"], 0);
    check_home_summary(&app, &pool, &owner, org1, org2).await;
    check_project_deletion(&app, &pool, &owner, org1, org2).await;
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/organizations/switch",
            json!({"organization_id":org2}),
            &owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(&app, "/api/v1/projects", &owner)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/projects",
            json!({"name":"Organization two project"}),
            &owner
        )
        .await
        .0,
        StatusCode::CREATED
    );
    post(
        &app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":org1}),
        &owner,
    )
    .await;
    assert_eq!(
        post(
            &app,
            &format!("/api/v1/auth/members/{owner_id}/role"),
            json!({"role":"member"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &app,
            &format!("/api/v1/auth/members/{owner_id}/remove"),
            json!({}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, invite, _) = post(
        &app,
        "/api/v1/auth/invitations",
        json!({"email":"member@example.com","role":"member"}),
        &owner,
    )
    .await;
    assert_eq!(invite["email_delivery"], "not_configured");
    let invitation = invite["token"].as_str().unwrap();
    let acceptance = json!({"name":"Member","email":"member@example.com","password":PASSWORD,"token":invitation});
    let (status, _, member) = post(
        &app,
        "/api/v1/auth/accept-invitation",
        acceptance.clone(),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        post(&app, "/api/v1/auth/accept-invitation", acceptance, "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, member_id, _) = get(&app, "/api/v1/auth/session", &member).await;
    assert_eq!(member_id["system_role"], "user");
    assert_eq!(
        get(&app, "/api/v1/monitoring", &member).await.0,
        StatusCode::OK
    );
    for section in [
        "images",
        "volumes",
        "networks",
        "swarm",
        "events",
        "disk-usage",
        "health",
    ] {
        let path = format!("/api/v1/docker/{section}");
        assert_eq!(get(&app, &path, "").await.0, StatusCode::UNAUTHORIZED);
        assert!(matches!(
            get(&app, &path, &member).await.0,
            StatusCode::OK | StatusCode::SERVICE_UNAVAILABLE
        ));
    }
    assert_eq!(
        get(&app, "/api/v1/docker/disk-usage", &owner).await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(&app, "/api/v1/docker/unknown", &owner).await.0,
        StatusCode::NOT_FOUND
    );

    let member_id = member_id["user_id"].as_str().unwrap();
    assert_eq!(
        get(&app, "/api/v1/home-summary", &member).await.0,
        StatusCode::OK
    );
    let projects = get(&app, "/api/v1/projects", &member).await.1;
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["name"], "Organization one project");
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/projects/{}", projects[0]["id"].as_str().unwrap()),
            json!({"name":"Organization one project"}),
            &member,
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/api/v1/auth/admin/users", &member).await.0,
        StatusCode::FORBIDDEN
    );
    assert!(matches!(
        get(&app, "/api/v1/containers", &member).await.0,
        StatusCode::OK | StatusCode::SERVICE_UNAVAILABLE
    ));
    assert_eq!(
        get(&app, "/api/v1/auth/platform-settings", &member).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/api/v1/auth/admin/organizations", &member)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/admin/organizations",
            json!({"name":"forbidden"}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            &format!("/api/v1/auth/admin/organizations/{org1}"),
            json!({"name":"forbidden","logo_url":null}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            &format!("/api/v1/auth/admin/organizations/{org1}/delete"),
            json!({"confirmation":"forbidden"}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/platform-settings/timezone",
            json!({"revision":0,"values":{"timezone":"UTC"}}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/projects",
            json!({"name":"forbidden"}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/invitations",
            json!({"email":"x@example.com","role":"admin"}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/organizations/switch",
            json!({"organization_id":org2}),
            &member
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let login = json!({"email":"member@example.com","password":PASSWORD});
    let (status, _, member2) = post(&app, "/api/v1/auth/sign-in", login.clone(), "").await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(member, member2);
    assert_eq!(
        get(
            &app,
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/logs",
            &member2
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/restart",
            json!({"version":1}),
            &member2
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/docker/swarm/services/aaaaaaaaaaaaaaaaaaaaaaaaa/scale",
            json!({"replicas":2,"version":1}),
            &member2
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/api/v1/auth/sessions", &member)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/sessions/revoke-others",
            json!({}),
            &member
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(&app, "/api/v1/auth/session", &member2).await.0,
        StatusCode::UNAUTHORIZED
    );
    post(
        &app,
        &format!("/api/v1/auth/admin/users/{member_id}/ban"),
        json!({"banned":true,"reason":"Test ban"}),
        &owner,
    )
    .await;
    assert_eq!(
        get(&app, "/api/v1/auth/session", &member).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&app, "/api/v1/auth/sign-in", login.clone(), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    post(
        &app,
        &format!("/api/v1/auth/admin/users/{member_id}/ban"),
        json!({"banned":false}),
        &owner,
    )
    .await;
    let (_, _, member) = post(&app, "/api/v1/auth/sign-in", login.clone(), "").await;
    let new_password = "An entirely new long password!";
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/change-password",
            json!({"password":PASSWORD,"new_password":new_password}),
            &member
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(&app, "/api/v1/auth/session", &member).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&app, "/api/v1/auth/sign-in", login, "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/sign-in",
            json!({"email":"member@example.com","password":new_password}),
            ""
        )
        .await
        .0,
        StatusCode::OK
    );
    let reset_path = format!("/api/v1/auth/admin/users/{member_id}/password-reset");
    let (_, reset, _) = post(&app, &reset_path, json!({}), &owner).await;
    let recovery_token = reset["token"].as_str().unwrap();
    let reset_body =
        json!({"token": recovery_token, "new_password": "Recovered account password long enough!"});
    assert_eq!(
        post(&app, "/api/v1/auth/reset-password", reset_body.clone(), "")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(&app, "/api/v1/auth/reset-password", reset_body, "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, reset, _) = post(&app, &reset_path, json!({}), &owner).await;
    sqlx::query("UPDATE auth_password_resets SET expires_at=now()-interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/reset-password",
            json!({"token": reset["token"], "new_password": PASSWORD}),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // A session renewed after a day does not extend its authentication freshness.
    sqlx::query("UPDATE auth_sessions SET updated_at=now()-interval '2 days',expires_at=now()+interval '1 hour' WHERE user_id=$1").bind(owner_id.parse::<uuid::Uuid>().unwrap()).execute(&pool).await.unwrap();
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM auth_sessions WHERE user_id=$1")
            .bind(owner_id.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    let (clock_status, clock, clock_cookie) = get(&app, "/api/v1/server-time", &owner).await;
    assert_eq!(clock_status, StatusCode::OK);
    assert!(clock_cookie.is_empty());
    assert!(
        (clock["unix_ms"].as_i64().unwrap() - chrono::Utc::now().timestamp_millis()).abs() < 5000
    );
    assert!(
        clock["timezone"]
            .as_str()
            .unwrap()
            .parse::<chrono_tz::Tz>()
            .is_ok()
    );
    assert!(clock["utc_offset"].as_str().unwrap().starts_with("UTC"));
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM auth_sessions WHERE user_id=$1")
            .bind(owner_id.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after, "clock polling must not renew idle sessions");
    let (log_status, _, log_cookie) =
        get(&app, "/api/v1/docker/swarm/services/invalid/logs", &owner).await;
    assert_eq!(log_status, StatusCode::BAD_REQUEST);
    assert!(log_cookie.is_empty(), "log polling must not renew cookies");
    let after_logs: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM auth_sessions WHERE user_id=$1")
            .bind(owner_id.parse::<uuid::Uuid>().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        before, after_logs,
        "log polling must not renew idle sessions"
    );
    assert_eq!(
        get(&app, "/api/v1/projects", &owner).await.0,
        StatusCode::OK
    );
    let (_, renewed, _) = get(&app, "/api/v1/auth/session", &owner).await;
    let expiry =
        chrono::DateTime::parse_from_rfc3339(renewed["expires_at"].as_str().unwrap()).unwrap();
    assert!(expiry > chrono::Utc::now() + chrono::Duration::days(2));
    // Expiry and fresh-session enforcement use database timestamps, not client input.
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at=now()-interval '16 minutes' WHERE user_id=$1",
    )
    .bind(owner_id.parse::<uuid::Uuid>().unwrap())
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/organizations",
            json!({"name":"Denied"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/reauthenticate",
            json!({"password":PASSWORD}),
            &owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // Starting enrollment re-verifies an older browser session before confirmation.
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at=now()-interval '1 hour' WHERE user_id=$1",
    )
    .bind(owner_id.parse::<uuid::Uuid>().unwrap())
    .execute(&pool)
    .await
    .unwrap();
    let (status, enrollment, _) = post(
        &app,
        "/api/v1/auth/mfa/enroll",
        json!({"password":PASSWORD}),
        &owner,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let secret = totp_rs::Secret::Encoded(enrollment["secret"].as_str().unwrap().into())
        .to_bytes()
        .unwrap();
    let totp =
        totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 0, 30, secret, None, "".into()).unwrap();
    let code = totp.generate_current().unwrap();
    let (status, recovery, _) = post(
        &app,
        "/api/v1/auth/mfa/confirm",
        json!({"code":code}),
        &owner,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recovery["recovery_codes"].as_array().unwrap().len(), 8);
    check_mfa_login(&app, &pool, owner_id, &totp, &code, &recovery).await;
    let (_, reset, _) = post(
        &app,
        &format!("/api/v1/auth/admin/users/{owner_id}/password-reset"),
        json!({}),
        &owner,
    )
    .await;
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/reset-password",
            json!({"token": reset["token"], "new_password": PASSWORD}),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // A revoked/expired session never falls back to another organization or a bearer header.
    sqlx::query("UPDATE auth_sessions SET expires_at=now()-interval '1 minute' WHERE user_id=$1")
        .bind(owner_id.parse::<uuid::Uuid>().unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(&app, "/api/v1/projects", &owner).await.0,
        StatusCode::UNAUTHORIZED
    );
    for _ in 0..11 {
        post(
            &app,
            "/api/v1/auth/sign-in",
            json!({"email":"missing@example.com","password":"bad"}),
            "",
        )
        .await;
    }
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/sign-in",
            json!({"email":"missing@example.com","password":"bad"}),
            ""
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    admin.close().await;
}

async fn check_project_deletion(
    app: &axum::Router,
    pool: &sqlx::PgPool,
    owner: &str,
    org1: &str,
    org2: &str,
) {
    let project = post(
        app,
        "/api/v1/projects",
        json!({"name":"Delete test"}),
        owner,
    )
    .await
    .1;
    let project_id = project["id"].as_str().unwrap();
    let path = format!("/api/v1/projects/{project_id}");
    assert_eq!(
        call(
            app,
            "DELETE",
            &path,
            json!({"name":"Delete test"}),
            "",
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            app,
            "DELETE",
            &path,
            json!({"name":"Delete test "}),
            owner,
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let env: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM environments WHERE project_id=$1::uuid")
            .bind(project_id)
            .fetch_one(pool)
            .await
            .unwrap();
    let service = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port) VALUES($1,$2,'Keep service','nginx:alpine',80)").bind(service).bind(env).execute(pool).await.unwrap();
    assert_eq!(
        call(
            app,
            "DELETE",
            &path,
            json!({"name":"Delete test"}),
            owner,
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM applications WHERE id=$1)")
            .bind(service)
            .fetch_one(pool)
            .await
            .unwrap()
    );
    sqlx::query("DELETE FROM applications WHERE id=$1")
        .bind(service)
        .execute(pool)
        .await
        .unwrap();
    post(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":org2}),
        owner,
    )
    .await;
    assert_eq!(
        call(
            app,
            "DELETE",
            &path,
            json!({"name":"Delete test"}),
            owner,
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    post(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":org1}),
        owner,
    )
    .await;
    assert_eq!(
        call(
            app,
            "DELETE",
            &path,
            json!({"name":"Delete test"}),
            owner,
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM environments WHERE id=$1)")
            .bind(env)
            .fetch_one(pool)
            .await
            .unwrap()
    );
    assert_eq!(
        call(
            app,
            "DELETE",
            &path,
            json!({"name":"Delete test"}),
            owner,
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

async fn check_home_summary(
    app: &axum::Router,
    pool: &sqlx::PgPool,
    owner: &str,
    org1: &str,
    org2: &str,
) {
    let org = uuid::Uuid::parse_str(org1).unwrap();
    let (_, empty, _) = get(app, "/api/v1/home-summary", owner).await;
    assert_eq!(empty["projects"], 1);
    assert_eq!(empty["services"], 0);
    assert_eq!(empty["needs_attention"], 0);
    assert_eq!(empty["attention"], json!([]));
    sqlx::query("INSERT INTO projects (id,name,organization_id,created_at) SELECT gen_random_uuid(),'summary_fixture', $1, now()-interval '8 days' FROM generate_series(1,101)")
        .bind(org).execute(pool).await.unwrap();
    let project: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM projects WHERE name='summary_fixture' LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap();
    let env = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'production')")
        .bind(env)
        .bind(project)
        .execute(pool)
        .await
        .unwrap();
    let services: Vec<uuid::Uuid> = (0..4).map(|_| uuid::Uuid::new_v4()).collect();
    for (i, kind) in ["application", "database", "application", "application"]
        .iter()
        .enumerate()
    {
        sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind) VALUES($1,$2,$3,'test',80,$4)")
            .bind(services[i]).bind(env).bind(format!("service-{i}")).bind(kind).execute(pool).await.unwrap();
    }
    // Old failure followed by success must not require attention. A latest old
    // failure still does, even though it is outside the 24-hour activity window.
    for (i, status, hours) in [
        (0, "failed", 48),
        (0, "succeeded", 2),
        (1, "failed", 30),
        (2, "failed", 3),
        (2, "queued", 1),
    ] {
        sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,status,created_at) VALUES($1,$2,'test',80,$3,now()-make_interval(hours => $4))")
            .bind(uuid::Uuid::new_v4()).bind(services[i]).bind(status).bind(hours).execute(pool).await.unwrap();
    }
    let (status, summary, _) = get(app, "/api/v1/home-summary", owner).await;
    assert_eq!(status, StatusCode::OK);
    for (key, value) in [
        ("projects", 102),
        ("projects_new_7d", 1),
        ("services", 4),
        ("applications", 3),
        ("databases", 1),
        ("deployments_24h", 3),
        ("succeeded_24h", 1),
        ("failed_24h", 1),
        ("in_progress_24h", 1),
        ("needs_attention", 1),
    ] {
        assert_eq!(summary[key], value, "{key}");
    }
    assert_eq!(summary["attention"][0]["id"], services[1].to_string());
    assert_eq!(summary["attention"][0]["project_id"], project.to_string());
    for i in 0..6 {
        let service = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port) VALUES($1,$2,$3,'test',80)")
            .bind(service).bind(env).bind(format!("failed-{i}")).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,status) VALUES($1,$2,'test',80,'failed')")
            .bind(uuid::Uuid::new_v4()).bind(service).execute(pool).await.unwrap();
    }
    let (_, capped, _) = get(app, "/api/v1/home-summary", owner).await;
    assert_eq!(capped["needs_attention"], 7);
    assert_eq!(capped["attention"].as_array().unwrap().len(), 5);
    post(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":org2}),
        owner,
    )
    .await;
    let (_, other, _) = get(app, "/api/v1/home-summary", owner).await;
    for key in ["projects", "services", "deployments_24h", "needs_attention"] {
        assert_eq!(other[key], 0, "other org {key}");
    }
    assert_eq!(other["attention"], json!([]));
    post(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":org1}),
        owner,
    )
    .await;
    sqlx::query("DELETE FROM projects WHERE name='summary_fixture'")
        .execute(pool)
        .await
        .unwrap();
}

// All mutations below are fixtures inside this test's disposable schema.
async fn mfa_challenge(app: &axum::Router, pool: &sqlx::PgPool) -> String {
    sqlx::query("DELETE FROM auth_throttles")
        .execute(pool)
        .await
        .unwrap();
    let (status, body, cookie) = post(
        app,
        "/api/v1/auth/sign-in",
        json!({"email":"owner@example.com","password":PASSWORD}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["two_factor_required"], true);
    assert_eq!(body["expires_in_seconds"], 300);
    assert!(cookie.starts_with("ferrix_test_session_mfa="));
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Lax")
            && cookie.contains("Max-Age=300")
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &cookie).await.0,
        StatusCode::UNAUTHORIZED
    );
    cookie
}
async fn check_mfa_login(
    app: &axum::Router,
    pool: &sqlx::PgPool,
    owner: &str,
    totp: &totp_rs::TOTP,
    used_code: &str,
    recovery: &Value,
) {
    let verify = "/api/v1/auth/sign-in/verify";
    let backup = recovery["recovery_codes"][0].as_str().unwrap();
    let (status, body, cookie) = post(
        app,
        "/api/v1/auth/sign-in",
        json!({"email":"owner@example.com","password":"incorrect password"}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.get("two_factor_required").is_none());
    assert!(cookie.is_empty());
    // Codes alone, a forged browser proof, and foreign origins cannot complete login.
    assert_eq!(
        post(app, verify, json!({"code":backup}), "").await.0,
        StatusCode::GONE
    );
    assert_eq!(
        post(
            app,
            verify,
            json!({"code":backup}),
            &format!("ferrix_test_session_mfa={}", "a".repeat(64))
        )
        .await
        .0,
        StatusCode::GONE
    );
    let challenge = mfa_challenge(app, pool).await;
    assert_eq!(
        call(
            app,
            "POST",
            verify,
            json!({"code":backup}),
            &challenge,
            Some("https://evil.example")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            app,
            verify,
            json!({"code":backup}),
            &format!("{challenge}; {challenge}")
        )
        .await
        .0,
        StatusCode::GONE
    );
    // Confirmation already consumed this time step; recovery remains available.
    assert_eq!(
        post(app, verify, json!({"code":used_code}), &challenge)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (first, second) = tokio::join!(
        post(app, verify, json!({"code":backup}), &challenge),
        post(app, verify, json!({"code":backup}), &challenge)
    );
    let (success, loser) = if first.0 == StatusCode::OK {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(success.0, StatusCode::OK);
    assert_eq!(loser.0, StatusCode::GONE);
    assert!(success.2.starts_with("ferrix_test_session="));
    assert_eq!(
        get(app, "/api/v1/auth/session", &success.2).await.0,
        StatusCode::OK
    );
    let challenge = mfa_challenge(app, pool).await;
    assert_eq!(
        post(app, verify, json!({"code":backup}), &challenge)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // Actual authenticator code works and cannot be replayed on another proof.
    sqlx::query("UPDATE auth_mfa SET last_step=-1 WHERE user_id=$1")
        .bind(owner.parse::<uuid::Uuid>().unwrap())
        .execute(pool)
        .await
        .unwrap();
    let code = totp.generate_current().unwrap();
    assert_eq!(
        post(app, verify, json!({"code":code}), &challenge).await.0,
        StatusCode::OK
    );
    let challenge = mfa_challenge(app, pool).await;
    assert_eq!(
        post(app, verify, json!({"code":code}), &challenge).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Failed attempts persist across requests; even a valid recovery code cannot bypass the cap.
    for _ in 1..5 {
        assert_eq!(
            post(app, verify, json!({"code":"wrong"}), &challenge)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        post(
            app,
            verify,
            json!({"code":recovery["recovery_codes"][1]}),
            &challenge
        )
        .await
        .0,
        StatusCode::GONE
    );
    for _ in 0..4 {
        assert_eq!(
            post(app, verify, json!({"code":"wrong"}), &challenge)
                .await
                .0,
            StatusCode::GONE
        );
    }
    assert_eq!(
        post(app, verify, json!({"code":"wrong"}), &challenge)
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let challenge = mfa_challenge(app, pool).await;
    sqlx::query("UPDATE auth_login_challenges SET expires_at=now()-interval '1 second'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            verify,
            json!({"code":recovery["recovery_codes"][1]}),
            &challenge
        )
        .await
        .0,
        StatusCode::GONE
    );
    let challenge = mfa_challenge(app, pool).await;
    assert_eq!(
        post(app, "/api/v1/auth/sign-in/cancel", json!({}), &challenge)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            verify,
            json!({"code":recovery["recovery_codes"][1]}),
            &challenge
        )
        .await
        .0,
        StatusCode::GONE
    );
    // A password revision invalidates an already issued proof without consuming a recovery code.
    let challenge = mfa_challenge(app, pool).await;
    let id = owner.parse::<uuid::Uuid>().unwrap();
    let original: String = sqlx::query_scalar("SELECT password_hash FROM auth_users WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE auth_users SET password_hash='changed' WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            verify,
            json!({"code":recovery["recovery_codes"][1]}),
            &challenge
        )
        .await
        .0,
        StatusCode::GONE
    );
    sqlx::query("UPDATE auth_users SET password_hash=$1 WHERE id=$2")
        .bind(original)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    let challenge = mfa_challenge(app, pool).await;
    sqlx::query("UPDATE auth_mfa SET enabled=false WHERE user_id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            verify,
            json!({"code":recovery["recovery_codes"][1]}),
            &challenge
        )
        .await
        .0,
        StatusCode::GONE
    );
    sqlx::query("UPDATE auth_mfa SET enabled=true WHERE user_id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn postgres_user_and_organization_management() {
    use futures_util::FutureExt;
    use std::panic::AssertUnwindSafe;
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("management_test_{}", uuid::Uuid::new_v4().simple());
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
        check_activity(&app(pool.clone()), &pool).await;
        check_platform_org_detail(&app(pool.clone()), &pool).await;
        check_organization_lifecycle(&app(pool.clone()), &pool).await;
        check_management(&app(pool.clone()), &pool).await;
        check_organization_delete_without_default_membership(&app(pool.clone()), &pool).await;
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
async fn check_organization_delete_without_default_membership(
    app: &axum::Router,
    pool: &sqlx::PgPool,
) {
    let user = uuid::Uuid::new_v4();
    let fallback = uuid::Uuid::new_v4();
    let disposable = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) SELECT $1,'Fallback owner','fallback-owner@example.com',password_hash FROM auth_users WHERE email='manager@example.com'")
        .bind(user).execute(pool).await.unwrap();
    for (id, name) in [(fallback, "Fallback"), (disposable, "Disposable")] {
        sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,$2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'owner')")
            .bind(id)
            .bind(user)
            .execute(pool)
            .await
            .unwrap();
    }
    let (status, _, cookie) = post(
        app,
        "/api/v1/auth/sign-in",
        json!({"email":"fallback-owner@example.com","password":PASSWORD}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/switch",
            json!({"organization_id":disposable}),
            &cookie
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/delete",
            json!({"confirmation":"Disposable"}),
            &cookie
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let session = get(app, "/api/v1/auth/session", &cookie).await;
    assert_eq!(session.0, StatusCode::OK);
    assert_eq!(session.1["organization_id"], fallback.to_string());
    // The default organization exists, but this user must never gain access to it.
    assert_eq!(
        get(app, "/api/v1/auth/organizations", &cookie)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/delete",
            json!({"confirmation":"Fallback"}),
            &cookie
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &cookie).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM auth_users WHERE id=$1)")
            .bind(user)
            .fetch_one(pool)
            .await
            .unwrap()
    );
}

async fn check_management(app: &axum::Router, pool: &sqlx::PgPool) {
    use uuid::Uuid;
    let (_, _, owner) = post(
        app,
        "/api/v1/auth/setup",
        json!({"name":"Manager","email":"manager@example.com","password":PASSWORD}),
        "",
    )
    .await;
    let identity = get(app, "/api/v1/auth/session", &owner).await.1;
    let admin = identity["user_id"].as_str().unwrap();
    let org = identity["organization_id"].as_str().unwrap();
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/admin/users/{admin}/revoke-sessions"),
            json!({}),
            &owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &owner).await.0,
        StatusCode::OK
    );
    let other_org = post(
        app,
        "/api/v1/auth/organizations",
        json!({"name":"Other organization"}),
        &owner,
    )
    .await
    .1;
    let other_org = other_org["id"].as_str().unwrap();
    assert_eq!(
        get(app, "/api/v1/auth/organizations/current", &owner)
            .await
            .1["is_default"],
        true
    );
    let protected = post(
        app,
        "/api/v1/auth/organizations/delete",
        json!({"confirmation":"My Organization"}),
        &owner,
    )
    .await;
    assert_eq!(protected.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        protected.1["error"],
        "The default organization cannot be deleted."
    );
    let member = Uuid::new_v4();
    let stranger = Uuid::new_v4();
    for (user, email, organization) in [
        (member, "member-management@example.com", org),
        (stranger, "stranger-management@example.com", other_org),
    ] {
        sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) SELECT $1,$2,$2,password_hash FROM auth_users WHERE id=$3").bind(user).bind(email).bind(admin.parse::<Uuid>().unwrap()).execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO auth_members(user_id,organization_id,role) VALUES($1,$2,'member')",
        )
        .bind(user)
        .bind(organization.parse::<Uuid>().unwrap())
        .execute(pool)
        .await
        .unwrap();
    }
    let member_login = json!({"email":"member-management@example.com","password":PASSWORD});
    let (_, _, member_cookie) = post(app, "/api/v1/auth/sign-in", member_login.clone(), "").await;
    assert_eq!(
        get(app, "/api/v1/auth/admin/users", &member_cookie).await.0,
        StatusCode::FORBIDDEN
    );
    let page = get(app, "/api/v1/auth/admin/users?limit=1&page=1", &owner)
        .await
        .1;
    assert_eq!(page["total"], 3);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        get(
            app,
            "/api/v1/auth/admin/users?q=member-management&role=user&status=active",
            &owner
        )
        .await
        .1["total"],
        1
    );
    assert_eq!(
        get(app, "/api/v1/auth/admin/users?q=%25", &owner).await.1["total"],
        0
    );
    assert_eq!(
        get(app, "/api/v1/auth/admin/users?role=owner", &owner)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        get(app, "/api/v1/auth/admin/users?limit=1000", &owner)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let detail_path = format!("/api/v1/auth/admin/users/{member}");
    let detail = get(app, &detail_path, &owner).await.1;
    assert_eq!(detail["organizations"].as_array().unwrap().len(), 1);
    assert_eq!(detail["sessions"].as_array().unwrap().len(), 1);
    assert!(!detail.to_string().contains("password_hash"));
    assert!(!detail.to_string().contains("token_hash"));
    assert_eq!(
        get(app, &detail_path, &member_cookie).await.0,
        StatusCode::FORBIDDEN
    );
    let admin_role = format!("/api/v1/auth/admin/users/{admin}/role");
    let member_role = format!("{detail_path}/role");
    assert_eq!(
        post(app, &admin_role, json!({"role":"user"}), &owner)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(app, &member_role, json!({"role":"admin"}), &member_cookie)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(app, &member_role, json!({"role":"admin"}), &owner)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    // Simultaneous self-demotions must leave exactly one active platform admin.
    let (a, b) = tokio::join!(
        post(app, &admin_role, json!({"role":"user"}), &owner),
        post(app, &member_role, json!({"role":"user"}), &member_cookie)
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|&&status| status == StatusCode::NO_CONTENT)
            .count(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM auth_users WHERE system_role='admin'")
            .fetch_one(pool)
            .await
            .unwrap(),
        1
    );
    sqlx::query("UPDATE auth_users SET system_role=CASE WHEN id=$1 THEN 'admin' ELSE 'user' END")
        .bind(admin.parse::<Uuid>().unwrap())
        .execute(pool)
        .await
        .unwrap();
    // Last-owner, cross-tenant transfer, owner deletion and self deletion are rejected.
    assert_eq!(
        post(app, "/api/v1/auth/organizations/leave", json!({}), &owner)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/transfer",
            json!({"user_id":stranger}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/admin/users/{admin}/delete"),
            json!({"confirmation":"manager@example.com"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // A temporary ban immediately revokes sessions and expires without a worker sweep.
    let ban_path = format!("{detail_path}/ban");
    assert_eq!(
        post(
            app,
            &ban_path,
            json!({"banned":true,"reason":"界".repeat(501)}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(post(app,&ban_path,json!({"banned":true,"reason":"界".repeat(500),"expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339()}),&owner).await.0,StatusCode::NO_CONTENT);
    assert_eq!(
        get(app, "/api/v1/auth/session", &member_cookie).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(app, "/api/v1/auth/sign-in", member_login.clone(), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE auth_users SET ban_expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(member)
        .execute(pool)
        .await
        .unwrap();
    let (status, _, member_cookie) =
        post(app, "/api/v1/auth/sign-in", member_login.clone(), "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        get(app, "/api/v1/auth/admin/users?status=banned", &owner)
            .await
            .1["total"],
        0
    );
    assert_eq!(
        post(app, &ban_path, json!({"banned":false}), &owner)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/current",
            json!({"name":"Forbidden"}),
            &member_cookie
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/current",
            json!({"name":"Managed organization","logo_url":"javascript:bad"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/current",
            json!({"name":"Managed organization","logo_url":"https://example.com/logo.png"}),
            &owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, "/api/v1/auth/organizations/current", &member_cookie)
            .await
            .1["name"],
        "Managed organization"
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/delete",
            json!({"confirmation":"Managed organization"}),
            &owner
        )
        .await
        .1["error"],
        "The default organization cannot be deleted."
    );
    let invite = post(
        app,
        "/api/v1/auth/invitations",
        json!({"email":"invite-management@example.com","role":"member"}),
        &owner,
    )
    .await
    .1;
    let renewed = post(
        app,
        &format!(
            "/api/v1/auth/invitations/{}/renew",
            invite["id"].as_str().unwrap()
        ),
        json!({}),
        &owner,
    )
    .await;
    assert_eq!(renewed.0, StatusCode::OK);
    assert_ne!(invite["token"], renewed.1["token"]);
    let mut accept = json!({"name":"Invited","email":"invite-management@example.com","password":PASSWORD,"token":invite["token"]});
    assert_eq!(
        post(app, "/api/v1/auth/accept-invitation", accept.clone(), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    accept["token"] = renewed.1["token"].clone();
    assert_eq!(
        post(app, "/api/v1/auth/accept-invitation", accept, "")
            .await
            .0,
        StatusCode::OK
    );
    // Organizations with projects cannot be deleted, even with the exact name.
    let project = post(
        app,
        "/api/v1/projects",
        json!({"name":"Managed project"}),
        &owner,
    )
    .await
    .1;
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/delete",
            json!({"confirmation":"Managed organization"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            app,
            "DELETE",
            &format!("/api/v1/projects/{}", project["id"].as_str().unwrap()),
            json!({"name":"Managed project"}),
            &owner,
            Some(ORIGIN)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/transfer",
            json!({"user_id":member}),
            &owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &owner).await.1["role"],
        "admin"
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &member_cookie).await.1["role"],
        "owner"
    );
    // User deletion must not remove an organization owner or overwrite projects.
    assert_eq!(
        post(
            app,
            &format!("{detail_path}/delete"),
            json!({"confirmation":"member-management@example.com"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Preserve deployment history even when its requesting user is deleted.
    let history_project = Uuid::new_v4();
    let history_environment = Uuid::new_v4();
    let history_service = Uuid::new_v4();
    let history_deployment = Uuid::new_v4();
    let queued_deployment = Uuid::new_v4();
    sqlx::query("INSERT INTO projects(id,name,organization_id) VALUES($1,'History',$2)")
        .bind(history_project)
        .bind(Uuid::parse_str(org).unwrap())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'production')")
        .bind(history_environment)
        .bind(history_project)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port) VALUES($1,$2,'History','nginx',80)")
        .bind(history_service).bind(history_environment).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,container_port,status) VALUES($1,$2,$3,'nginx',80,'succeeded')")
        .bind(history_deployment).bind(history_service).bind(member).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,container_port) VALUES($1,$2,$3,'nginx',80)")
        .bind(queued_deployment).bind(history_service).bind(member).execute(pool).await.unwrap();
    // Transfer back so deletion can exercise its complete FK cleanup.
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/transfer",
            json!({"user_id":admin}),
            &member_cookie
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    post(
        app,
        "/api/v1/auth/invitations",
        json!({"email":"issued-by-member@example.com","role":"member"}),
        &member_cookie,
    )
    .await;
    assert_eq!(
        post(
            app,
            &format!("{detail_path}/delete"),
            json!({"confirmation":"incorrect"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    sqlx::query("UPDATE deployments SET status='running' WHERE id=$1")
        .bind(queued_deployment)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            &format!("{detail_path}/delete"),
            json!({"confirmation":"member-management@example.com"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    sqlx::query("UPDATE deployments SET status='queued' WHERE id=$1")
        .bind(queued_deployment)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            &format!("{detail_path}/delete"),
            json!({"confirmation":"member-management@example.com"}),
            &owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &member_cookie).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(app, "/api/v1/auth/organizations/current", &owner)
            .await
            .1["name"],
        "Managed organization"
    );
    let actor: Option<Uuid> =
        sqlx::query_scalar("SELECT requested_by FROM deployments WHERE id=$1")
            .bind(history_deployment)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(actor.is_none());
    let (queued_actor, queued_status, finished): (Option<Uuid>, String, bool) = sqlx::query_as(
        "SELECT requested_by,status,finished_at IS NOT NULL FROM deployments WHERE id=$1",
    )
    .bind(queued_deployment)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(queued_actor.is_none());
    assert_eq!(queued_status, "failed");
    assert!(finished);
    sqlx::query("DELETE FROM projects WHERE id=$1")
        .bind(history_project)
        .execute(pool)
        .await
        .unwrap();
    // Deletion preserves only the requesting session and returns it to the
    // default organization, without extending its verification or lifetime.
    let preserved_project = post(
        app,
        "/api/v1/projects",
        json!({"name":"Preserved project"}),
        &owner,
    )
    .await
    .1;
    post(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":other_org}),
        &owner,
    )
    .await;
    let owner_sessions = get(app, "/api/v1/auth/sessions", &owner).await.1;
    let session_id: Uuid = owner_sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["current"] == true)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let before: (String,String,String) = sqlx::query_as("SELECT authenticated_at::text,expires_at::text,absolute_expires_at::text FROM auth_sessions WHERE id=$1").bind(session_id).fetch_one(pool).await.unwrap();
    let (_, _, second_session) = post(
        app,
        "/api/v1/auth/sign-in",
        json!({"email":"manager@example.com","password":PASSWORD}),
        "",
    )
    .await;
    post(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":other_org}),
        &second_session,
    )
    .await;
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/delete",
            json!({"confirmation":"incorrect"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &owner).await.1["organization_id"],
        other_org
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/delete",
            json!({"confirmation":"Other organization"}),
            &owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let session = get(app, "/api/v1/auth/session", &owner).await;
    assert_eq!(session.0, StatusCode::OK);
    assert_eq!(session.1["organization_id"], org);
    assert_eq!(session.1["organization_name"], "Managed organization");
    let after: (String,String,String) = sqlx::query_as("SELECT authenticated_at::text,expires_at::text,absolute_expires_at::text FROM auth_sessions WHERE id=$1").bind(session_id).fetch_one(pool).await.unwrap();
    assert_eq!(before, after);
    assert_eq!(
        get(app, "/api/v1/auth/session", &second_session).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        !get(app, "/api/v1/auth/organizations", &owner)
            .await
            .1
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["id"] == other_org)
    );
    let projects = get(app, "/api/v1/projects", &owner).await.1;
    assert!(
        projects
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == preserved_project["id"])
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/delete",
            json!({"confirmation":"Managed organization"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // The member-removal endpoint must not bypass last-platform-membership protection.
    sqlx::query("INSERT INTO auth_members(user_id,organization_id,role) VALUES($1,$2,'owner')")
        .bind(stranger)
        .bind(org.parse::<Uuid>().unwrap())
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/members/{admin}/remove"),
            json!({}),
            &owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Expired sensitive-action verification is rejected without modifying the account.
    sqlx::query("UPDATE auth_sessions SET authenticated_at=now()-interval '1 hour'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/admin/users/{stranger}/role"),
            json!({"role":"admin"}),
            &owner
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

async fn check_platform_settings(app: &axum::Router, pool: &sqlx::PgPool, owner: &str) {
    let endpoint = "/api/v1/auth/platform-settings";
    assert_eq!(get(app, endpoint, "").await.0, StatusCode::UNAUTHORIZED);
    let (code, settings, _) = get(app, endpoint, owner).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(settings["email_delivery_available"], true);
    assert_eq!(settings["social_login_available"], false);
    let timezone = format!("{endpoint}/timezone");
    assert_eq!(
        post(
            app,
            &timezone,
            json!({"revision":0,"values":{"timezone":"invalid"}}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            &timezone,
            json!({"revision":0,"values":{"timezone":"Asia/Kathmandu"}}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let clock = get(app, "/api/v1/server-time", owner).await.1;
    assert_eq!(clock["timezone"], "Asia/Kathmandu");
    assert_eq!(clock["utc_offset"], "UTC+05:45");
    assert_eq!(
        post(
            app,
            &timezone,
            json!({"revision":0,"values":{"timezone":"UTC"}}),
            owner
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let email = format!("{endpoint}/email");
    let mut input = json!({"host":"smtp.example.com","port":587,"security":"starttls","username":"mail-user","from_address":"noreply@example.com","from_name":"Ferrix","password":"private-smtp-secret"});
    assert_eq!(
        post(app, &email, json!({"revision":0,"values":input}), owner)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let settings = get(app, endpoint, owner).await.1;
    assert_eq!(settings["email"]["password_configured"], true);
    assert!(!settings.to_string().contains("private-smtp-secret"));
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT config_cipher FROM platform_settings WHERE section='email'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        !stored
            .windows(b"private-smtp-secret".len())
            .any(|bytes| bytes == b"private-smtp-secret")
    );
    input.as_object_mut().unwrap().remove("password");
    assert_eq!(
        post(app, &email, json!({"revision":1,"values":input}), owner)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, endpoint, owner).await.1["email"]["password_configured"],
        true
    );
    input["password"] = json!("");
    assert_eq!(
        post(app, &email, json!({"revision":2,"values":input}), owner)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, endpoint, owner).await.1["email"]["password_configured"],
        false
    );

    assert_eq!(post(app,&email,json!({"revision":3,"values":{"provider":"google","from_address":"sender@gmail.com","password":"google-app-password"}}),owner).await.0,StatusCode::NO_CONTENT);
    let email_config = get(app, endpoint, owner).await.1;
    assert_eq!(email_config["email"]["provider"], "google");
    assert_eq!(email_config["email"]["host"], "smtp.gmail.com");
    assert_eq!(email_config["email"]["username"], "sender@gmail.com");
    assert_eq!(post(app,&email,json!({"revision":4,"values":{"provider":"resend","from_address":"sender@example.com","resend_api_key":"re_private-key"}}),owner).await.0,StatusCode::NO_CONTENT);
    let email_config = get(app, endpoint, owner).await.1;
    assert_eq!(email_config["email"]["resend_api_key_configured"], true);
    assert!(!email_config.to_string().contains("re_private-key"));
    assert_eq!(post(app,&email,json!({"revision":5,"values":{"provider":"resend","from_address":"sender@example.com"}}),owner).await.0,StatusCode::NO_CONTENT);
    assert_eq!(
        get(app, endpoint, owner).await.1["email"]["resend_api_key_configured"],
        true
    );
    assert_eq!(
        post(
            app,
            &email,
            json!({"revision":6,"values":{"provider":"resend","resend_api_key":""}}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, endpoint, owner).await.1["email"]["resend_api_key_configured"],
        false
    );
    let social = format!("{endpoint}/social");
    assert_eq!(post(app,&social,json!({"revision":0,"values":{"google_client_id":"google-id","github_client_id":"github-id","google_client_secret":"private-google","github_client_secret":"private-github"}}),owner).await.0,StatusCode::NO_CONTENT);
    let response = get(app, endpoint, owner).await.1;
    assert_eq!(response["social"]["google_client_secret_configured"], true);
    assert_eq!(response["social"]["github_client_secret_configured"], true);
    assert!(!response.to_string().contains("private-google"));
    assert!(!response.to_string().contains("private-github"));
    sqlx::query("UPDATE auth_sessions SET authenticated_at=now()-interval '16 minutes' WHERE token_hash IS NOT NULL AND user_id=(SELECT id FROM auth_users WHERE email='owner@example.com')").execute(pool).await.unwrap();
    assert_eq!(
        post(
            app,
            &timezone,
            json!({"revision":1,"values":{"timezone":"UTC"}}),
            owner
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE auth_sessions SET authenticated_at=now() WHERE user_id=(SELECT id FROM auth_users WHERE email='owner@example.com')").execute(pool).await.unwrap();
    // A refused local SMTP connection must not roll back the committed invitation.
    assert_eq!(post(app,&email,json!({"revision":7,"values":{"provider":"smtp","host":"127.0.0.1","port":1,"username":"qa","from_address":"qa@example.com","password":"test-only"}}),owner).await.0,StatusCode::NO_CONTENT);
    let (status, invitation, _) = post(
        app,
        "/api/v1/auth/invitations",
        json!({"email":"email-failure@example.com","role":"member"}),
        owner,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invitation["email_delivery"], "failed");
    let renewed = post(
        app,
        &format!(
            "/api/v1/auth/invitations/{}/renew",
            invitation["id"].as_str().unwrap()
        ),
        json!({}),
        owner,
    )
    .await;
    assert_eq!(renewed.0, StatusCode::OK);
    assert_eq!(renewed.1["email_delivery"], "failed");
    assert_ne!(invitation["token"], renewed.1["token"]);
    let recipient = json!({"name":"Email fallback","email":"email-failure@example.com","password":PASSWORD,"token":renewed.1["token"]});
    assert_eq!(
        post(app, "/api/v1/auth/accept-invitation", recipient, "")
            .await
            .0,
        StatusCode::OK
    );
    sqlx::query("DELETE FROM platform_settings")
        .execute(pool)
        .await
        .unwrap();
}

async fn check_platform_organizations(app: &axum::Router, pool: &sqlx::PgPool, owner: &str) {
    let base = "/api/v1/auth/admin/organizations";
    let initial = get(app, "/api/v1/auth/session", owner).await.1;
    let active = initial["organization_id"].as_str().unwrap();
    let foreign = uuid::Uuid::new_v4();
    let foreign_user = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,'Foreign organization')")
        .bind(foreign)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) SELECT $1,'Foreign owner',$2,password_hash FROM auth_users WHERE email='owner@example.com'").bind(foreign_user).bind(format!("{foreign_user}@example.com")).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'owner')")
        .bind(foreign)
        .bind(foreign_user)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(get(app, base, "").await.0, StatusCode::UNAUTHORIZED);
    assert!(
        get(app, base, owner)
            .await
            .1
            .as_array()
            .unwrap()
            .iter()
            .any(|org| org["id"] == foreign.to_string())
    );
    let edit = format!("{base}/{foreign}");
    assert_eq!(
        post(
            app,
            &edit,
            json!({"name":"Renamed foreign organization","logo_url":"http://example.com/logo.png"}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            &edit,
            json!({"name":"Renamed foreign organization","logo_url":null}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let admin_members:i64=sqlx::query_scalar("SELECT count(*) FROM auth_members WHERE organization_id=$1 AND user_id=(SELECT id FROM auth_users WHERE email='owner@example.com')").bind(foreign).fetch_one(pool).await.unwrap();
    assert_eq!(admin_members, 0);
    assert_eq!(
        post(
            app,
            &format!("{edit}/delete"),
            json!({"confirmation":"wrong"}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let project = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO projects(id,name,organization_id) VALUES($1,'Protected project',$2)")
        .bind(project)
        .bind(foreign)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            &format!("{edit}/delete"),
            json!({"confirmation":"Renamed foreign organization"}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    sqlx::query("DELETE FROM projects WHERE id=$1")
        .bind(project)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            &format!("{edit}/delete"),
            json!({"confirmation":"Renamed foreign organization"}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_users WHERE id=$1")
        .bind(foreign_user)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(retained, 1);
    sqlx::query("DELETE FROM auth_users WHERE id=$1")
        .bind(foreign_user)
        .execute(pool)
        .await
        .unwrap();
    let created = post(
        app,
        base,
        json!({"name":"Platform-created organization"}),
        owner,
    )
    .await;
    assert_eq!(created.0, StatusCode::OK);
    let org = created.1["id"].as_str().unwrap();
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/switch",
            json!({"organization_id":org}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            &format!("{base}/{org}/delete"),
            json!({"confirmation":"Platform-created organization"}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/switch",
            json!({"organization_id":active}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            &format!("{base}/{org}/delete"),
            json!({"confirmation":"Platform-created organization"}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            &format!("{base}/{active}/delete"),
            json!({"confirmation":"anything"}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

async fn check_activity(app: &axum::Router, pool: &sqlx::PgPool) {
    use sha2::{Digest, Sha256};
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    for (org, name) in [(a, "Audit A"), (b, "Audit B")] {
        sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,$2)")
            .bind(org)
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
    }
    let mut actors = Vec::new();
    for (n, role, system, org) in [
        (1, "member", "admin", a),
        (2, "owner", "user", a),
        (3, "admin", "user", a),
        (4, "member", "user", a),
        (5, "owner", "user", b),
    ] {
        let user = uuid::Uuid::new_v4();
        let token = format!("{n:064x}");
        sqlx::query("INSERT INTO auth_users(id,name,email,password_hash,system_role) VALUES($1,$2,$3,'unused',$4)").bind(user).bind(format!("Audit user {n}")).bind(format!("audit{n}@example.com")).bind(system).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,$3)")
            .bind(org)
            .bind(user)
            .bind(role)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_sessions(id,token_hash,user_id,active_organization_id) VALUES($1,$2,$3,$4)").bind(uuid::Uuid::new_v4()).bind(hex::encode(Sha256::digest(token.as_bytes()))).bind(user).bind(org).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action) VALUES($1,$2,$3,'audit.fixture')").bind(uuid::Uuid::new_v4()).bind(user).bind(org).execute(pool).await.unwrap();
        actors.push((user, format!("ferrix_test_session={token}")));
    }
    let base = "/api/v1/auth/activity";
    assert_eq!(get(app, base, "").await.0, StatusCode::UNAUTHORIZED);
    let all = get(app, base, &actors[0].1).await;
    assert_eq!(all.0, StatusCode::OK);
    assert_eq!(all.1["total"], 5);
    for index in [1, 2] {
        let result = get(app, base, &actors[index].1).await;
        assert_eq!(result.0, StatusCode::OK);
        assert_eq!(result.1["total"], 4);
        assert!(
            result.1["organizations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|org| org["id"] == a.to_string())
        );
        assert_eq!(
            get(
                app,
                &format!("{base}?organization_id={b}"),
                &actors[index].1
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(get(app, base, &actors[3].1).await.1["total"], 1);
    assert_eq!(
        get(
            app,
            &format!("{base}?user_id={}", actors[1].0),
            &actors[3].1
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(
            app,
            &format!("{base}?organization_id={b}&q=fixture"),
            &actors[0].1
        )
        .await
        .1["total"],
        1
    );
    assert_eq!(
        get(app, &format!("{base}?q=%25"), &actors[0].1).await.1["total"],
        0
    );
    // All mutating project requests have a durable pending record before the handler runs.
    assert_eq!(
        post(
            app,
            "/api/v1/projects",
            json!({"name":"Audit project"}),
            &actors[1].1
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        post(app, "/api/v1/projects", json!({"name":""}), &actors[1].1)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let events = get(app, &format!("{base}?q=request.POST"), &actors[0].1)
        .await
        .1;
    assert_eq!(events["total"], 2);
    assert!(
        events["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["outcome"] == "failed")
    );
    assert!(
        events["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["outcome"] == "succeeded")
    );
    sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action) VALUES($1,$2,$3,'platform.secret_test')").bind(uuid::Uuid::new_v4()).bind(actors[0].0).bind(a).execute(pool).await.unwrap();
    assert_eq!(
        get(app, &format!("{base}?q=secret_test"), &actors[1].1)
            .await
            .1["total"],
        0
    );
    // Pagination and deletion preserve historical identities, even after FKs become NULL.
    sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action) SELECT gen_random_uuid(),$1,$2,'audit.page' FROM generate_series(1,30)").bind(actors[4].0).bind(b).execute(pool).await.unwrap();
    assert_eq!(
        get(app, &format!("{base}?q=audit.page"), &actors[0].1)
            .await
            .1["items"]
            .as_array()
            .unwrap()
            .len(),
        25
    );
    assert_eq!(
        get(app, &format!("{base}?q=audit.page&page=1"), &actors[0].1)
            .await
            .1["items"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        get(app, &format!("{base}?q=audit.page&page=99"), &actors[0].1)
            .await
            .1["total"],
        30
    );
    sqlx::query("DELETE FROM auth_users WHERE id=$1")
        .bind(actors[4].0)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth_organizations WHERE id=$1")
        .bind(b)
        .execute(pool)
        .await
        .unwrap();
    let deleted = get(app, &format!("{base}?organization_id={b}"), &actors[0].1)
        .await
        .1;
    assert_eq!(deleted["total"], 31);
    assert_eq!(deleted["items"][0]["user_name"], "Audit user 5");
    assert_eq!(deleted["items"][0]["organization_name"], "Audit B");
    let environment:uuid::Uuid=sqlx::query_scalar("SELECT e.id FROM environments e JOIN projects p ON p.id=e.project_id WHERE p.organization_id=$1 LIMIT 1").bind(a).fetch_one(pool).await.unwrap();
    let service = uuid::Uuid::new_v4();
    let deployment = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port) VALUES($1,$2,'Audit service','nginx:alpine',80)").bind(service).bind(environment).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO deployments(id,application_id,requested_by,image,container_port) VALUES($1,$2,$3,'nginx:alpine',80)").bind(deployment).bind(service).bind(actors[1].0).execute(pool).await.unwrap();
    sqlx::query("UPDATE deployments SET status='failed' WHERE id=$1")
        .bind(deployment)
        .execute(pool)
        .await
        .unwrap();
    let lifecycle = get(app, &format!("{base}?q=deployment."), &actors[0].1)
        .await
        .1;
    assert_eq!(lifecycle["total"], 2);
    assert!(
        lifecycle["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["target_id"] == service.to_string())
    );
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO deployments(id,application_id,image,container_port) VALUES($1,$2,'nginx:alpine',80)").bind(uuid::Uuid::new_v4()).bind(service).execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        get(app, &format!("{base}?q=deployment."), &actors[0].1)
            .await
            .1["total"],
        2
    );
    pool.execute("CREATE FUNCTION reject_audit_fixture() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'audit unavailable'; END $$; CREATE TRIGGER reject_audit_fixture BEFORE INSERT ON auth_audit FOR EACH ROW EXECUTE FUNCTION reject_audit_fixture()").await.unwrap();
    assert_eq!(
        post(
            app,
            "/api/v1/projects",
            json!({"name":"Must not exist"}),
            &actors[1].1
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM projects WHERE name='Must not exist'")
            .fetch_one(pool)
            .await
            .unwrap(),
        0
    );
    pool.execute(
        "DROP TRIGGER reject_audit_fixture ON auth_audit; DROP FUNCTION reject_audit_fixture()",
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM projects WHERE organization_id=$1")
        .bind(a)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth_audit WHERE organization_ref=ANY($1)")
        .bind(vec![a, b])
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth_organizations WHERE id=$1")
        .bind(a)
        .execute(pool)
        .await
        .unwrap();
    for (user, _) in actors {
        sqlx::query("DELETE FROM auth_users WHERE id=$1")
            .bind(user)
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn check_platform_org_detail(app: &axum::Router, pool: &sqlx::PgPool) {
    use sha2::{Digest, Sha256};
    let active = uuid::Uuid::new_v4();
    let org = uuid::Uuid::new_v4();
    for (id, name) in [(active, "Detail active"), (org, "Detail target")] {
        sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,$2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
    }
    let mut actors = Vec::new();
    for (n, role, system, organization) in [
        (10, "member", "admin", active),
        (11, "owner", "user", org),
        (12, "admin", "user", org),
        (13, "member", "user", org),
    ] {
        let user = uuid::Uuid::new_v4();
        let token = format!("{n:064x}");
        sqlx::query("INSERT INTO auth_users(id,name,email,password_hash,system_role) VALUES($1,$2,$3,'unused',$4)").bind(user).bind(format!("Detail actor {n}")).bind(format!("detail{n}@example.com")).bind(system).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,$3)")
            .bind(organization)
            .bind(user)
            .bind(role)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_sessions(id,token_hash,user_id,active_organization_id) VALUES($1,$2,$3,$4)").bind(uuid::Uuid::new_v4()).bind(hex::encode(Sha256::digest(token.as_bytes()))).bind(user).bind(organization).execute(pool).await.unwrap();
        actors.push((user, format!("ferrix_test_session={token}")));
    }
    let base = format!("/api/v1/auth/admin/organizations/{org}");
    let admin = &actors[0].1;
    let original = get(app, "/api/v1/auth/session", admin).await.1;
    assert_eq!(get(app, &base, "").await.0, StatusCode::UNAUTHORIZED);
    for (_, cookie) in &actors[1..] {
        assert_eq!(get(app, &base, cookie).await.0, StatusCode::FORBIDDEN);
        assert_eq!(
            post(
                app,
                &format!("{base}/members/{}/role", actors[3].0),
                json!({"role":"admin"}),
                cookie
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            post(
                app,
                &format!("{base}/members/{}/remove", actors[3].0),
                json!({}),
                cookie
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            post(
                app,
                &format!("{base}/invitations"),
                json!({"email":"detail-invited@example.com","role":"member"}),
                cookie
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    for (_, cookie) in &actors[1..] {
        for suffix in ["policy", "members", "invitations"] {
            assert_eq!(
                get(app, &format!("{base}/{suffix}"), cookie).await.0,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            post(
                app,
                &format!("{base}/policy"),
                json!({"member_limit":3,"invitation_limit":1}),
                cookie
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        post(
            app,
            &format!("{base}/policy"),
            json!({"member_limit":3,"invitation_limit":1}),
            admin
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, &format!("{base}/policy"), admin).await.1,
        json!({"member_limit":3,"invitation_limit":1})
    );
    let read = get(app, &base, admin).await;
    assert_eq!(read.0, StatusCode::OK);
    assert_eq!(
        get(app, &format!("{base}/members"), admin).await.1["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        get(
            app,
            &format!("/api/v1/auth/admin/organizations/{}", uuid::Uuid::new_v4()),
            admin
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let owner_role = format!("{base}/members/{}/role", actors[1].0);
    let owner_remove = format!("{base}/members/{}/remove", actors[1].0);
    assert_eq!(
        post(app, &owner_role, json!({"role":"member"}), admin)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(app, &owner_remove, json!({}), admin).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            &format!("{base}/members/{}/remove", actors[0].0),
            json!({}),
            admin
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "UPDATE auth_sessions SET authenticated_at=now()-interval '16 minutes' WHERE user_id=$1",
    )
    .bind(actors[0].0)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(
        post(app, &owner_role, json!({"role":"member"}), admin)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE auth_sessions SET authenticated_at=now() WHERE user_id=$1")
        .bind(actors[0].0)
        .execute(pool)
        .await
        .unwrap();
    let promote = format!("{base}/members/{}/role", actors[3].0);
    assert_eq!(
        post(app, &promote, json!({"role":"owner"}), admin).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(app, &owner_role, json!({"role":"member"}), admin)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'member')")
        .bind(active)
        .bind(actors[1].0)
        .execute(pool)
        .await
        .unwrap();
    let alternate = "00000000000000000000000000000000000000000000000000000000000000ff";
    sqlx::query("INSERT INTO auth_sessions(id,token_hash,user_id,active_organization_id) VALUES($1,$2,$3,$4)").bind(uuid::Uuid::new_v4()).bind(hex::encode(Sha256::digest(alternate.as_bytes()))).bind(actors[1].0).bind(active).execute(pool).await.unwrap();
    assert_eq!(
        post(app, &owner_remove, json!({}), admin).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, "/api/v1/auth/session", &actors[1].1).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(
            app,
            "/api/v1/auth/session",
            &format!("ferrix_test_session={alternate}")
        )
        .await
        .0,
        StatusCode::OK
    );
    let invitation = post(
        app,
        &format!("{base}/invitations"),
        json!({"email":"detail-invited@example.com","role":"admin"}),
        admin,
    )
    .await;
    assert_eq!(invitation.0, StatusCode::OK);
    assert_eq!(invitation.1["email_delivery"], "not_configured");
    let invite = invitation.1["id"].as_str().unwrap();
    let renew = format!("{base}/invitations/{invite}/renew");
    let renewed = post(app, &renew, json!({}), admin).await;
    assert_eq!(renewed.0, StatusCode::OK);
    assert_ne!(renewed.1["token"], invitation.1["token"]);
    for (_, cookie) in &actors[2..] {
        assert_eq!(
            post(app, &renew, json!({}), cookie).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/admin/organizations/{active}/invitations/{invite}/revoke"),
            json!({}),
            admin
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut registration = json!({"name":"Detail invited","email":"detail-invited@example.com","password":PASSWORD,"token":invitation.1["token"]});
    assert_eq!(
        post(
            app,
            "/api/v1/auth/accept-invitation",
            registration.clone(),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    registration["token"] = renewed.1["token"].clone();
    assert_eq!(
        post(app, "/api/v1/auth/accept-invitation", registration, "")
            .await
            .0,
        StatusCode::OK
    );
    let second = post(
        app,
        &format!("{base}/invitations"),
        json!({"email":"detail-cancel@example.com","role":"member"}),
        admin,
    )
    .await
    .1;
    let revoke = format!(
        "{base}/invitations/{}/revoke",
        second["id"].as_str().unwrap()
    );
    for (_, cookie) in &actors[2..] {
        assert_eq!(
            post(app, &revoke, json!({}), cookie).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        post(app, &revoke, json!({}), admin).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, &format!("{base}/invitations?status=pending"), admin)
            .await
            .1["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let final_identity = get(app, "/api/v1/auth/session", admin).await.1;
    assert_eq!(
        final_identity["organization_id"],
        original["organization_id"]
    );
    assert_eq!(final_identity["session_id"], original["session_id"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM auth_members WHERE organization_id=$1 AND user_id=$2"
        )
        .bind(org)
        .bind(actors[0].0)
        .fetch_one(pool)
        .await
        .unwrap(),
        0
    );
    let events = get(
        app,
        &format!("/api/v1/auth/activity?organization_id={org}&q=platform."),
        admin,
    )
    .await
    .1;
    assert_eq!(events["users"].as_array().unwrap().len(), 2);
    assert_eq!(events["total"], 8);
    assert!(
        events["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["user_id"] == actors[0].0.to_string()
                && event["target_id"].is_string())
    );
    sqlx::query("UPDATE auth_users SET system_role='admin' WHERE id=$1")
        .bind(actors[2].0)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        post(
            app,
            &format!("{base}/members/{}/remove", actors[2].0),
            json!({}),
            admin
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    sqlx::query("UPDATE auth_users SET system_role='user' WHERE id=$1")
        .bind(actors[2].0)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth_audit WHERE organization_ref=ANY($1)")
        .bind(vec![active, org])
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth_organizations WHERE id=ANY($1)")
        .bind(vec![active, org])
        .execute(pool)
        .await
        .unwrap();
    for (user, _) in actors {
        sqlx::query("DELETE FROM auth_users WHERE id=$1")
            .bind(user)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM auth_users WHERE email='detail-invited@example.com'")
        .execute(pool)
        .await
        .unwrap();
}

async fn check_organization_lifecycle(app: &axum::Router, pool: &sqlx::PgPool) {
    use sha2::{Digest, Sha256};
    use uuid::Uuid;
    let org = Uuid::new_v4();
    sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,'Lifecycle limits lab')")
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    let mut actors = Vec::new();
    for (n, role, system) in [
        (70, "owner", "admin"),
        (71, "admin", "user"),
        (72, "member", "user"),
    ] {
        let user = Uuid::new_v4();
        let token = format!("{n:064x}");
        sqlx::query("INSERT INTO auth_users(id,name,email,password_hash,system_role) VALUES($1,$2,$3,'unused',$4)").bind(user).bind(format!("Lifecycle {n}")).bind(format!("lifecycle{n}@example.com")).bind(system).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,$3)")
            .bind(org)
            .bind(user)
            .bind(role)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_sessions(id,token_hash,user_id,active_organization_id) VALUES($1,$2,$3,$4)").bind(Uuid::new_v4()).bind(hex::encode(Sha256::digest(token.as_bytes()))).bind(user).bind(org).execute(pool).await.unwrap();
        actors.push((user, format!("ferrix_test_session={token}")));
    }
    let owner = &actors[0].1;
    let policy = "/api/v1/auth/organizations/policy";
    assert!(get(app, policy, owner).await.1["member_limit"].is_null());
    for cookie in [&actors[1].1, &actors[2].1] {
        assert_eq!(
            post(
                app,
                policy,
                json!({"member_limit":3,"invitation_limit":1}),
                cookie
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(app, "/api/v1/auth/admin/organizations/search", cookie)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        get(app, policy, &actors[2].1).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(app, "/api/v1/auth/invitations/search", &actors[2].1)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            app,
            policy,
            json!({"member_limit":2,"invitation_limit":1}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            policy,
            json!({"member_limit":3,"invitation_limit":1}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // All writers share a transaction lock, so concurrent creates cannot overfill a cap.
    let (a, b) = tokio::join!(
        post(
            app,
            "/api/v1/auth/invitations",
            json!({"email":"lifecycle-a@example.com","role":"member"}),
            owner
        ),
        post(
            app,
            "/api/v1/auth/invitations",
            json!({"email":"lifecycle-b@example.com","role":"member"}),
            owner
        )
    );
    assert!(a.0.is_success() ^ b.0.is_success());
    let (invite, email) = if a.0.is_success() {
        (a.1, "lifecycle-a@example.com")
    } else {
        (b.1, "lifecycle-b@example.com")
    };
    let registration = json!({"name":"Lifecycle recipient","email":email,"password":"lifecycle-password-123","token":invite["token"]});
    assert_eq!(
        post(
            app,
            "/api/v1/auth/accept-invitation",
            registration.clone(),
            ""
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM auth_users WHERE email=$1")
            .bind(email)
            .fetch_one(pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        post(
            app,
            policy,
            json!({"member_limit":4,"invitation_limit":1}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(app, "/api/v1/auth/accept-invitation", registration, "")
            .await
            .0,
        StatusCode::OK
    );
    let summary = get(
        app,
        &format!(
            "/api/v1/auth/admin/users/{}",
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM auth_users WHERE email=$1")
                .bind(email)
                .fetch_one(pool)
                .await
                .unwrap()
        ),
        owner,
    )
    .await
    .1;
    assert!(summary["summary"]["last_sign_in_at"].is_string());
    assert_eq!(summary["summary"]["mfa_enabled"], false);
    assert_eq!(summary["summary"]["api_keys"]["active"], 0);
    assert!(summary["organizations"][0]["joined_at"].is_string());
    let recipient = sqlx::query_scalar::<_, Uuid>("SELECT id FROM auth_users WHERE email=$1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap();
    for state in ["active", "revoked", "expired"] {
        let key = Uuid::new_v4();
        sqlx::query("INSERT INTO auth_api_keys(id,user_id,organization_id,name,key_hash,prefix,scopes,expires_at,revoked_at) VALUES($1,$2,$3,'Summary fixture',$4,'fixture',ARRAY['read'],CASE WHEN $5='expired' THEN now()-interval '1 day' ELSE now()+interval '1 day' END,CASE WHEN $5='revoked' THEN now() END)").bind(key).bind(recipient).bind(org).bind(format!("summary-{key}")).bind(state).execute(pool).await.unwrap();
    }
    let populated = get(app, &format!("/api/v1/auth/admin/users/{recipient}"), owner)
        .await
        .1;
    assert_eq!(
        populated["summary"]["api_keys"],
        json!({"active":1,"revoked":1,"expired":1})
    );
    assert!(!populated.to_string().contains("key_hash"));
    let delivered = get(
        app,
        "/api/v1/auth/invitations/search?status=accepted",
        owner,
    )
    .await
    .1;
    assert_eq!(delivered["items"][0]["delivery_status"], "not_configured");
    assert!(delivered["items"][0]["delivery_attempted_at"].is_string());
    // A rejected proof cannot subsequently accept, renew, or reject a second time.
    let rejected = post(
        app,
        "/api/v1/auth/invitations",
        json!({"email":"lifecycle-reject@example.com","role":"member"}),
        owner,
    )
    .await
    .1;
    let proof = json!({"email":"lifecycle-reject@example.com","token":rejected["token"]});
    assert_eq!(
        post(
            app,
            "/api/v1/auth/decline-invitation",
            json!({"email":"wrong@example.com","token":rejected["token"]}),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(app, "/api/v1/auth/decline-invitation", proof.clone(), "")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(app, "/api/v1/auth/decline-invitation", proof, "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(
            app,
            &format!(
                "/api/v1/auth/invitations/{}/renew",
                rejected["id"].as_str().unwrap()
            ),
            json!({}),
            owner
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(post(app,"/api/v1/auth/accept-invitation",json!({"name":"Rejected","email":"lifecycle-reject@example.com","password":"lifecycle-password-123","token":rejected["token"]}),"").await.0,StatusCode::UNAUTHORIZED);
    // History survives terminal states and inviter deletion; snapshots contain no token hashes.
    for (n, status) in [(0, "pending"), (1, "expired"), (2, "cancelled")] {
        let invite_id = Uuid::new_v4();
        sqlx::query("INSERT INTO auth_invitations(id,organization_id,email,role,token_hash,invited_by,expires_at,cancelled_at) VALUES($1,$2,$3,'member',$4,$5,CASE WHEN $6='expired' THEN now()-interval '1 day' ELSE now()+interval '1 day' END,CASE WHEN $6='cancelled' THEN now() END)").bind(invite_id).bind(org).bind(format!("lifecycle-history{n}@example.com")).bind(format!("history-{invite_id}")).bind(actors[1].0).bind(status).execute(pool).await.unwrap();
    }
    let expired = get(app, "/api/v1/auth/invitations/search?status=expired", owner)
        .await
        .1["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/invitations/{expired}/renew"),
            json!({}),
            owner
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let pending = get(app, "/api/v1/auth/invitations/search?status=pending", owner)
        .await
        .1["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/invitations/{pending}/renew"),
            json!({}),
            owner
        )
        .await
        .0,
        StatusCode::OK
    );
    for status in ["pending", "expired", "cancelled", "accepted", "rejected"] {
        let result = get(
            app,
            &format!("/api/v1/auth/invitations/search?status={status}"),
            owner,
        )
        .await;
        assert_eq!(result.0, StatusCode::OK, "{result:?}");
        assert_eq!(result.1["total"], 1, "{status}");
        assert!(result.1["items"][0].get("token_hash").is_none());
        assert_eq!(result.1["items"][0]["status"], status);
    }
    assert_eq!(
        post(
            app,
            policy,
            json!({"member_limit":null,"invitation_limit":null}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/admin/users/{}/delete", actors[1].0),
            json!({"confirmation":"lifecycle71@example.com"}),
            owner
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM auth_invitations WHERE organization_id=$1 AND invited_by IS NULL AND inviter_name='Lifecycle 71'").bind(org).fetch_one(pool).await.unwrap(),2);
    // Bounded pages, literal substring search, role filters, and tenant scope.
    for n in 0..28 {
        let user = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO auth_users(id,name,email,password_hash) VALUES($1,$2,$3,'unused')",
        )
        .bind(user)
        .bind(format!("Paged% member {n}"))
        .bind(format!("paged{n}@example.com"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'member')",
        )
        .bind(org)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }
    let first = get(app, "/api/v1/auth/members/search?q=Paged%25", owner).await;
    assert_eq!(first.0, StatusCode::OK, "{first:?}");
    assert_eq!(first.1["total"], 28);
    assert_eq!(first.1["items"].as_array().unwrap().len(), 25);
    let second = get(
        app,
        "/api/v1/auth/members/search?q=Paged%25&page=1&role=member",
        owner,
    )
    .await
    .1;
    assert_eq!(second["total"], 28);
    assert_eq!(second["items"].as_array().unwrap().len(), 3);
    assert!(first.1["items"].as_array().unwrap().iter().all(|a| {
        second["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| a["id"] != b["id"])
    }));
    assert_eq!(
        get(app, "/api/v1/auth/members/search?q=Detail", owner)
            .await
            .1["total"],
        0
    );
    assert_eq!(
        get(app, "/api/v1/auth/members/search?role=owner", owner)
            .await
            .1["total"],
        1
    );
    assert_eq!(
        get(app, "/api/v1/auth/members/search?role=bogus", owner)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        get(
            app,
            "/api/v1/auth/admin/organizations/search?q=Lifecycle%20limits",
            owner
        )
        .await
        .1["total"],
        1
    );
    let events = get(
        app,
        &format!("/api/v1/auth/activity?organization_id={org}&q=invitation.rejected"),
        owner,
    )
    .await
    .1;
    assert_eq!(events["total"], 1);
    assert_eq!(events["items"][0]["target_id"], rejected["id"]);
    sqlx::query("DELETE FROM auth_audit WHERE organization_ref=$1")
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth_organizations WHERE id=$1")
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth_users WHERE email LIKE 'lifecycle%@example.com' OR email LIKE 'paged%@example.com'").execute(pool).await.unwrap();
}
