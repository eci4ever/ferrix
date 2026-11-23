use super::test_support::*;
use super::*;
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
fn signature(secret: &str, body: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}
async fn push(f: &Fixture, secret: &str, delivery: Uuid, body: Value) -> Reply {
    let body = body.to_string();
    call(
        &f.app,
        "POST",
        &format!("/api/v1/webhooks/github/{}", f.service),
        "",
        Value::String(body.clone()),
        &[
            ("x-hub-signature-256", signature(secret, &body)),
            ("x-github-event", "push".into()),
            ("x-github-delivery", delivery.to_string()),
        ],
        false,
    )
    .await
}
#[test]
fn source_paths_and_hmac_are_strict() {
    assert_eq!(
        repository("https://github.com/Owner/Repo.git").unwrap(),
        "owner/repo"
    );
    for repo in [
        "https://example.com/o/r",
        "https://user:token@github.com/o/r",
        "o/r/extra",
        "../repo",
    ] {
        assert!(repository(repo).is_err())
    }
    for value in [
        "../Dockerfile",
        "/Dockerfile",
        "dir/../Dockerfile",
        "x\\Dockerfile",
    ] {
        assert!(path(value, false).is_err())
    }
    assert_eq!(path(".", true).unwrap(), ".");
    assert!(branch("feature/test").is_ok());
    assert!(branch("main..bad").is_err());
    let sig = signature("secret", "payload");
    assert!(verify("secret", &sig, b"payload"));
    assert!(!verify("secret", &sig, b"changed"));
    assert!(!verify("wrong", &sig, b"payload"));
    assert!(!verify("secret", "sha1=abc", b"payload"));
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn private_source_snapshots_and_signed_webhooks_are_isolated_and_durable() {
    let f = Fixture::new().await;
    let result = AssertUnwindSafe(check(&f)).catch_unwind().await;
    f.finish().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error)
    }
}
async fn check(f: &Fixture) {
    let base = format!("/api/v1/services/{}/source", f.service);
    let settings = json!({"mode":"github","repository":"owner/repository","branch":"main","dockerfile":"Dockerfile","context":".","token":"ghp_disposable_private_token","auto_deploy":true});
    assert_eq!(
        f.request("POST", &base, settings.clone()).await.status,
        StatusCode::NO_CONTENT
    );
    let source = f.request("GET", &base, Value::Null).await;
    assert_eq!(source.body["token_saved"], true);
    assert!(!source.body.to_string().contains("ghp_disposable"));
    let cipher: Vec<u8> = sqlx::query_scalar(
        "SELECT config_cipher FROM application_git_sources WHERE application_id=$1",
    )
    .bind(f.service)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert!(open::<Source>(&f.state, Uuid::new_v4(), "source", &cipher).is_err());
    assert!(open::<Source>(&f.state, f.service, "webhook", &cipher).is_err());
    let secret = f
        .request("POST", &format!("{base}/webhook-secret"), json!({}))
        .await;
    assert_eq!(secret.status, StatusCode::OK);
    let secret = secret.body["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    assert!(
        !f.request("GET", &base, Value::Null)
            .await
            .body
            .to_string()
            .contains(secret)
    );
    let manual = f
        .request(
            "POST",
            &format!("/api/v1/services/{}/deploy", f.service),
            json!({}),
        )
        .await;
    assert_eq!(manual.status, StatusCode::ACCEPTED);
    let manual = Uuid::parse_str(manual.body["id"].as_str().unwrap()).unwrap();
    let mut edited = settings.clone();
    edited["token"] = json!("ghp_replacement_private_token");
    assert_eq!(
        f.request("POST", &base, edited).await.status,
        StatusCode::NO_CONTENT
    );
    let snapshot: Vec<u8> = sqlx::query_scalar("SELECT source_cipher FROM deployments WHERE id=$1")
        .bind(manual)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(
        open::<Source>(&f.state, f.service, "source", &snapshot)
            .unwrap()
            .token
            .as_deref(),
        Some("ghp_disposable_private_token")
    );
    let body = json!({"ref":"refs/heads/main","after":"1".repeat(40),"deleted":false,"repository":{"full_name":"owner/repository"},"sender":{"login":"octocat"}});
    assert_eq!(
        push(f, "wrong", Uuid::new_v4(), body.clone()).await.status,
        StatusCode::UNAUTHORIZED
    );
    let mut ignored = body.clone();
    ignored["ref"] = json!("refs/heads/other");
    assert_eq!(
        push(f, secret, Uuid::new_v4(), ignored).await.body["status"],
        "ignored"
    );
    let mut deleted = body.clone();
    deleted["deleted"] = json!(true);
    assert_eq!(
        push(f, secret, Uuid::new_v4(), deleted).await.body["status"],
        "ignored"
    );
    let mut foreign = body.clone();
    foreign["repository"]["full_name"] = json!("foreign/repository");
    assert_eq!(
        push(f, secret, Uuid::new_v4(), foreign).await.body["status"],
        "ignored"
    );
    let first = Uuid::new_v4();
    assert_eq!(
        push(f, secret, first, body.clone()).await.status,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        push(f, secret, first, body.clone()).await.body["status"],
        "duplicate"
    );
    assert_eq!(
        push(f, secret, Uuid::new_v4(), body.clone()).await.body["status"],
        "duplicate"
    );
    let mut second = body.clone();
    second["after"] = json!("2".repeat(40));
    let second_id = Uuid::new_v4();
    assert_eq!(
        push(f, secret, second_id, second.clone()).await.status,
        StatusCode::ACCEPTED
    );
    let mut conn = f.state.pool.acquire().await.unwrap();
    dispatch(&f.state, &mut conn).await.unwrap();
    let pending: Option<Uuid> = sqlx::query_scalar(
        "SELECT pending_delivery FROM application_git_sources WHERE application_id=$1",
    )
    .bind(f.service)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(pending, Some(second_id));
    sqlx::query("UPDATE deployments SET status='failed' WHERE id=$1")
        .bind(manual)
        .execute(&f.state.pool)
        .await
        .unwrap();
    dispatch(&f.state, &mut conn).await.unwrap();
    let jobs:Vec<(String,Option<String>,Option<String>)>=sqlx::query_as("SELECT trigger_kind,trigger_sender,requested_by_name FROM deployments WHERE application_id=$1 ORDER BY created_at,id").bind(f.service).fetch_all(&f.state.pool).await.unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(
        jobs[1],
        (
            "webhook".into(),
            Some("octocat".into()),
            Some("Git owner".into())
        )
    );
    let rotated = f
        .request("POST", &format!("{base}/webhook-secret"), json!({}))
        .await;
    let new_secret = rotated.body["secret"].as_str().unwrap();
    let mut third = body.clone();
    third["after"] = json!("3".repeat(40));
    assert_eq!(
        push(f, secret, Uuid::new_v4(), third.clone()).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        push(f, new_secret, Uuid::new_v4(), third).await.status,
        StatusCode::ACCEPTED
    );
    sqlx::query("UPDATE auth_members SET role='member' WHERE user_id=$1 AND organization_id=$2")
        .bind(f.user)
        .bind(f.org)
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE deployments SET status='failed' WHERE application_id=$1 AND status='queued'",
    )
    .bind(f.service)
    .execute(&f.state.pool)
    .await
    .unwrap();
    dispatch(&f.state, &mut conn).await.unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT auto_deploy FROM application_git_sources WHERE application_id=$1"
        )
        .bind(f.service)
        .fetch_one(&f.state.pool)
        .await
        .unwrap()
    );
    assert_eq!(
        f.request("GET", &base, Value::Null).await.status,
        StatusCode::FORBIDDEN
    );
    drop(conn);
}
