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
    let owner_id = identity["user_id"].as_str().unwrap();
    let org1 = identity["organization_id"].as_str().unwrap();
    assert_eq!(identity["email"], "owner@example.com");
    assert_eq!(identity["role"], "owner");
    assert_eq!(identity["system_role"], "admin");
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
    let member_id = member_id["user_id"].as_str().unwrap();
    let projects = get(&app, "/api/v1/projects", &member).await.1;
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["name"], "Organization one project");
    assert_eq!(
        get(&app, "/api/v1/auth/admin/users", &member).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/api/v1/containers", &member).await.0,
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
        json!({"banned":true}),
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
    let owner_login = json!({"email":"owner@example.com","password":PASSWORD});
    assert_eq!(
        post(&app, "/api/v1/auth/sign-in", owner_login.clone(), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/sign-in",
            json!({"email":"owner@example.com","password":PASSWORD,"code":code}),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let backup = recovery["recovery_codes"][0].as_str().unwrap();
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/sign-in",
            json!({"email":"owner@example.com","password":PASSWORD,"code":backup}),
            ""
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        post(
            &app,
            "/api/v1/auth/sign-in",
            json!({"email":"owner@example.com","password":PASSWORD,"code":backup}),
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
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
