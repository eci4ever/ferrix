use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ferrix_api::{AppState, auth::AuthConfig, router};
use serde_json::{Value, json};
use sqlx::{Executor, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;
const ORIGIN: &str = "http://127.0.0.1:5174";
const PASSWORD: &str = "Resource permission test password!";
async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Value,
    credential: &str,
) -> (StatusCode, Value, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("origin", ORIGIN);
    if credential.starts_with("frx_") {
        req = req.header("authorization", format!("Bearer {credential}"))
    } else if !credential.is_empty() {
        req = req.header("cookie", credential)
    }
    let response = app
        .clone()
        .oneshot(
            req.body(if method == "GET" {
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
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cookie,
    )
}
async fn post(
    app: &axum::Router,
    path: &str,
    body: Value,
    cookie: &str,
) -> (StatusCode, Value, String) {
    call(app, "POST", path, body, cookie).await
}
async fn get(app: &axum::Router, path: &str, cookie: &str) -> (StatusCode, Value, String) {
    call(app, "GET", path, Value::Null, cookie).await
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL pointing to the current user's database"]
async fn classroom_assignments_enforce_live_resource_boundaries() {
    use futures_util::FutureExt;
    use std::panic::AssertUnwindSafe;
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("permissions_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let namespace = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |conn, _| {
            let q = format!("SET search_path TO {namespace}");
            Box::pin(async move {
                conn.execute(q.as_str()).await?;
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
            docker: bollard::Docker::connect_with_local_defaults().unwrap(),
            gateway: Default::default(),
            auth: std::sync::Arc::new(AuthConfig {
                origin: ORIGIN.into(),
                secure: false,
                cookie_name: "permission_session".into(),
                key: [42; 32],
            }),
        });
        scenario(&app, &pool).await;
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
async fn scenario(app: &axum::Router, pool: &sqlx::PgPool) {
    let (status, _, platform) = post(
        app,
        "/api/v1/auth/setup",
        json!({"name":"Platform","email":"platform@example.com","password":PASSWORD}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let invitation = post(
        app,
        "/api/v1/auth/invitations",
        json!({"email":"lecturer@example.com","role":"admin"}),
        &platform,
    )
    .await
    .1;
    let (status,_,admin)=post(app,"/api/v1/auth/accept-invitation",json!({"name":"Lecturer","email":"lecturer@example.com","password":PASSWORD,"token":invitation["token"]}),"").await;
    assert_eq!(status, StatusCode::OK);
    let lecturer = get(app, "/api/v1/auth/session", &admin).await.1;
    assert_eq!(lecturer["system_role"], "user");
    assert_eq!(lecturer["role"], "admin");
    assert_eq!(
        get(app, "/api/v1/auth/admin/users", &admin).await.0,
        StatusCode::FORBIDDEN
    );
    let mut students = Vec::new();
    for email in ["student-a@example.com", "student-b@example.com"] {
        let invite = post(
            app,
            "/api/v1/auth/invitations",
            json!({"email":email,"role":"member"}),
            &admin,
        )
        .await;
        assert_eq!(invite.0, StatusCode::OK);
        let signed = post(
            app,
            "/api/v1/auth/accept-invitation",
            json!({"name":email,"email":email,"password":PASSWORD,"token":invite.1["token"]}),
            "",
        )
        .await;
        assert_eq!(signed.0, StatusCode::OK);
        let identity = get(app, "/api/v1/auth/session", &signed.2).await.1;
        students.push((identity["user_id"].clone(), signed.2));
    }
    let project = post(app, "/api/v1/projects", json!({"name":"Class"}), &admin)
        .await
        .1;
    let project_id = project["id"].as_str().unwrap();
    let hidden_project = post(
        app,
        "/api/v1/projects",
        json!({"name":"Another class"}),
        &admin,
    )
    .await
    .1;
    let environments = get(
        app,
        &format!("/api/v1/projects/{project_id}/environments"),
        &admin,
    )
    .await
    .1;
    let env = environments["environments"][0]["id"].as_str().unwrap();
    let mut services = Vec::new();
    for name in ["Application A", "Application B"] {
        let created=post(app,&format!("/api/v1/environments/{env}/services"),json!({"kind":"application","name":name,"image":"nginx:alpine","container_port":80,"environment":"SECRET=private"}),&admin).await;
        assert_eq!(created.0, StatusCode::CREATED, "{}", created.1);
        services.push(created.1["id"].as_str().unwrap().to_owned());
    }
    let (a_user, a) = &students[0];
    let (b_user, b) = &students[1];
    let service_a = &services[0];
    let service_b = &services[1];
    let database = post(
        app,
        &format!("/api/v1/environments/{env}/services"),
        json!({"kind":"database","name":"Shared database","engine":"mariadb"}),
        &admin,
    )
    .await;
    assert_eq!(database.0, StatusCode::CREATED, "{}", database.1);
    let database = database.1["id"].as_str().unwrap();
    // Legacy organizations keep read-only member visibility, never secrets/mutation.
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_b}"), a)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_b}/environment"), a)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            app,
            "/api/v1/auth/organizations/access",
            json!({"restricted":true}),
            &admin
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    for (service, user, permission) in [
        (service_a, a_user, "developer"),
        (service_b, b_user, "developer"),
    ] {
        assert_eq!(
            post(
                app,
                &format!("/api/v1/services/{service}/assignments"),
                json!({"user_id":user,"permission":permission}),
                &admin
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    // Scope grants do not promote a member into organization administration.
    assert_eq!(
        get(app, "/api/v1/auth/session", a).await.1["role"],
        "member"
    );
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_a}"), a)
            .await
            .0,
        StatusCode::OK
    );
    let list = get(app, &format!("/api/v1/environments/{env}/services"), a)
        .await
        .1;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], *service_a);
    assert_eq!(
        get(app, "/api/v1/projects", a).await.1[0]["service_count"],
        1
    );
    assert_eq!(get(app, "/api/v1/home-summary", a).await.1["services"], 1);
    assert_eq!(
        get(app, "/api/v1/projects", a)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(post(app,"/api/v1/auth/api-keys",json!({"name":"Forbidden project","scopes":["read"],"expires_in_days":1,"project_id":hidden_project["id"]}),a).await.0,StatusCode::NOT_FOUND);
    for (service, title) in [(service_a, "A event"), (service_b, "B event")] {
        sqlx::query("INSERT INTO notifications(id,organization_id,project_id,service_id,kind,title,message) SELECT $1,p.organization_id,p.id,a.id,'resource_high',$3,$3 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$2")
            .bind(Uuid::new_v4()).bind(service.parse::<Uuid>().unwrap()).bind(title).execute(pool).await.unwrap();
    }
    let inbox = get(app, "/api/v1/notifications", a).await.1;
    assert_eq!(inbox["unread"], 1);
    assert_eq!(inbox["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(inbox["notifications"][0]["title"], "A event");
    assert_eq!(
        get(app, "/api/v1/organizations/quota", a).await.0,
        StatusCode::FORBIDDEN
    );
    for prefix in ["services", "applications"] {
        for suffix in [
            "",
            "/configuration",
            "/environment",
            "/runtime",
            "/logs",
            "/deployments",
            "/notes",
            "/source",
            "/source/upload",
            "/domains",
            "/backups",
            "/terminal",
            "/metrics",
            "/history",
            "/logs/history",
        ] {
            let result = get(app, &format!("/api/v1/{prefix}/{service_b}{suffix}"), a).await;
            // Unregistered aliases remain 404, and registered routes cannot reveal B.
            assert_eq!(
                result.0,
                StatusCode::NOT_FOUND,
                "{prefix}{suffix}: {}",
                result.1
            );
        }
    }
    for suffix in [
        "configuration",
        "environment",
        "deploy",
        "control",
        "notes",
        "source",
        "source/upload",
        "rollback",
        "move",
        "clone",
        "domains",
        "assignments",
    ] {
        assert_eq!(
            post(
                app,
                &format!("/api/v1/services/{service_b}/{suffix}"),
                json!({}),
                a
            )
            .await
            .0,
            StatusCode::NOT_FOUND,
            "{suffix}"
        );
    }
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_a}/environment"), a)
            .await
            .1["environment"],
        "SECRET=private"
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/environment"),
            json!({"environment":"SECRET=updated"}),
            a
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_b}/environment"), b)
            .await
            .1["environment"],
        "SECRET=private"
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/notes"),
            json!({"notes":"A's notes","revision":0}),
            a
        )
        .await
        .0,
        StatusCode::OK
    );
    // Approved runtime/resource configuration cannot be changed by a developer.
    let mut config = get(
        app,
        &format!("/api/v1/services/{service_a}/configuration"),
        a,
    )
    .await
    .1;
    config["name"] = json!("Application A renamed");
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/configuration"),
            config.clone(),
            a
        )
        .await
        .0,
        StatusCode::OK
    );
    config["memory_mib"] = json!(8192);
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/configuration"),
            config,
            a
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for path in [
        format!("/api/v1/services/{service_a}/rollback"),
        format!("/api/v1/services/{service_a}/clone"),
        format!("/api/v1/projects/{project_id}/quota"),
        "/api/v1/auth/organizations/access".into(),
    ] {
        assert_eq!(
            post(app, &path, json!({"restricted":true}), a).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/assignments"),
            json!({"user_id":b_user,"permission":"developer"}),
            a
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let key=post(app,"/api/v1/auth/api-keys",json!({"name":"A agent","scopes":["read","deploy","manage","secrets:read"],"expires_in_days":1,"project_id":project_id}),a).await;
    assert_eq!(key.0, StatusCode::CREATED, "{}", key.1);
    let key = key.1["key"].as_str().unwrap();
    assert_eq!(
        get(
            app,
            &format!("/api/v1/services/{service_a}/environment"),
            key
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        get(
            app,
            &format!("/api/v1/services/{service_b}/environment"),
            key
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/deploy"),
            json!({}),
            a
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    // Source configuration does not expose tokens or reuse one for another repository.
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/source"),
            json!({"mode":"github","repository":"classroom/app-a","token":"fixture-token"}),
            &admin
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let source = get(app, &format!("/api/v1/services/{service_a}/source"), a).await;
    assert_eq!(source.0, StatusCode::OK);
    assert!(source.1.get("token").is_none());
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/source"),
            json!({"mode":"github","repository":"classroom/other"}),
            a
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/source"),
            json!({"mode":"github","repository":"classroom/other","clear_token":true}),
            a
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // Downgrade/revoke takes effect for existing cookies and existing bearer keys.
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/assignments"),
            json!({"user_id":a_user,"permission":"viewer"}),
            &admin
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_a}"), a)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        get(
            app,
            &format!("/api/v1/services/{service_a}/environment"),
            key
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{service_a}/deploy"),
            json!({}),
            a
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            app,
            &format!("/api/v1/projects/{project_id}/assignments"),
            json!({"user_id":a_user,"permission":"developer"}),
            &admin
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_b}/environment"), a)
            .await
            .0,
        StatusCode::OK
    );
    for suffix in ["configuration", "environment", "terminal"] {
        assert_eq!(
            get(app, &format!("/api/v1/services/{database}/{suffix}"), a)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    post(
        app,
        &format!("/api/v1/projects/{project_id}/assignments"),
        json!({"user_id":a_user,"permission":null}),
        &admin,
    )
    .await;
    post(
        app,
        &format!("/api/v1/services/{service_a}/assignments"),
        json!({"user_id":a_user,"permission":null}),
        &admin,
    )
    .await;
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_a}"), key)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        get(app, "/api/v1/projects", a)
            .await
            .1
            .as_array()
            .unwrap()
            .is_empty()
    );
    let grants: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auth_audit WHERE action LIKE 'assignment.%'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(grants >= 6);
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_b}/environment"), b)
            .await
            .1["environment"],
        "SECRET=private"
    );
    // Member removal invalidates existing sessions and cascades grants.
    assert_eq!(
        post(
            app,
            &format!("/api/v1/auth/members/{}/remove", b_user.as_str().unwrap()),
            json!({}),
            &admin
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(app, &format!("/api/v1/services/{service_b}"), b)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM service_assignments WHERE user_id=$1")
            .bind(b_user.as_str().unwrap().parse::<Uuid>().unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    let other = post(
        app,
        "/api/v1/auth/organizations",
        json!({"name":"Other organization"}),
        &platform,
    )
    .await
    .1;
    post(
        app,
        "/api/v1/auth/organizations/switch",
        json!({"organization_id":other["id"]}),
        &platform,
    )
    .await;
    let foreign = post(
        app,
        "/api/v1/projects",
        json!({"name":"Foreign project"}),
        &platform,
    )
    .await
    .1;
    let foreign_env = get(
        app,
        &format!(
            "/api/v1/projects/{}/environments",
            foreign["id"].as_str().unwrap()
        ),
        &platform,
    )
    .await
    .1["environments"][0]["id"]
        .clone();
    let foreign_service=post(app,&format!("/api/v1/environments/{}/services",foreign_env.as_str().unwrap()),json!({"kind":"application","name":"Foreign app","image":"nginx:alpine","container_port":80}),&platform).await.1;
    let foreign_id = foreign_service["id"].as_str().unwrap();
    assert_eq!(
        post(
            app,
            &format!("/api/v1/services/{foreign_id}/assignments"),
            json!({"user_id":a_user,"permission":"developer"}),
            &admin
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(app, &format!("/api/v1/services/{foreign_id}"), a)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let forged=sqlx::query("INSERT INTO service_assignments(organization_id,service_id,user_id,permission) VALUES($1,$2,$3,'developer')")
        .bind(lecturer["organization_id"].as_str().unwrap().parse::<Uuid>().unwrap()).bind(foreign_id.parse::<Uuid>().unwrap()).bind(a_user.as_str().unwrap().parse::<Uuid>().unwrap()).execute(pool).await;
    assert!(
        forged.is_err(),
        "Cross-organization grants must be rejected by the database too"
    );
}
