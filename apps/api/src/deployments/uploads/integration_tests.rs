use super::*;
use crate::deployments::test_support::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::FutureExt;
use std::{panic::AssertUnwindSafe, time::Duration};
use tower::ServiceExt;
fn archive(version: &str) -> Vec<u8> {
    super::tests::tar_bytes(&[
        (
            "Dockerfile",
            b"FROM nginx:alpine\nCOPY index.html /usr/share/nginx/html/index.html\n",
        ),
        ("index.html", version.as_bytes()),
    ])
}
async fn upload(f: &Fixture, app: Uuid, bytes: Vec<u8>) -> (StatusCode, Value) {
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/services/{app}/source/upload?context=.&dockerfile=Dockerfile"
                ))
                .header("cookie", &f.cookie)
                .header("origin", ORIGIN)
                .header("content-type", "application/octet-stream")
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    (
        response.status(),
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap_or(Value::Null),
    )
}
async fn wait(f: &Fixture, expected: &str) -> Value {
    for _ in 0..240 {
        let rows = f
            .request(
                "GET",
                &format!("/api/v1/services/{}/deployments", f.service),
                Value::Null,
            )
            .await
            .body;
        let d = &rows[0];
        if matches!(d["status"].as_str(), Some("succeeded" | "failed")) {
            assert_eq!(d["status"], expected, "{d}");
            return d.clone();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Deployment did not complete")
}
async fn content(f: &Fixture) -> String {
    let port: i32 = sqlx::query_scalar("SELECT host_port FROM applications WHERE id=$1")
        .bind(f.service)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    reqwest::get(format!("http://127.0.0.1:{port}"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker BuildKit"]
async fn uploaded_revisions_build_fail_safely_and_roll_back() {
    let f = Fixture::new().await;
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let result = AssertUnwindSafe(async {
        let base = format!("/api/v1/services/{}", f.service);
        assert_eq!(
            upload(&f, Uuid::new_v4(), archive("foreign")).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            upload(&f, f.service, b"invalid archive".to_vec()).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        for version in ["first-version", "second-version"] {
            let (status, preview) = upload(&f, f.service, archive(version)).await;
            assert_eq!(status, StatusCode::OK, "{preview}");
            assert_eq!(preview["sha256"].as_str().unwrap().len(), 64);
            assert_eq!(
                f.request(
                    "POST",
                    &format!("{base}/source"),
                    json!({"mode":"upload","upload_id":preview["upload_id"]})
                )
                .await
                .status,
                StatusCode::NO_CONTENT
            );
            assert_eq!(
                f.request("GET", &base, Value::Null).await.body["source_type"],
                "upload"
            );
            assert_eq!(
                f.request("POST", &format!("{base}/deploy"), json!({}))
                    .await
                    .status,
                StatusCode::ACCEPTED
            );
            let success = wait(&f, "succeeded").await;
            assert_eq!(success["source_archive_sha256"], preview["sha256"]);
            assert_eq!(content(&f).await, version);
        }
        let first = f
            .request("GET", &format!("{base}/deployments"), Value::Null)
            .await
            .body[1]
            .clone();
        let bad = super::tests::tar_bytes(&[("Dockerfile", b"FROM nginx:alpine\nRUN exit 42\n")]);
        let (_, preview) = upload(&f, f.service, bad).await;
        assert_eq!(
            f.request(
                "POST",
                &format!("{base}/source"),
                json!({"mode":"upload","upload_id":preview["upload_id"]})
            )
            .await
            .status,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            f.request("POST", &format!("{base}/deploy"), json!({}))
                .await
                .status,
            StatusCode::ACCEPTED
        );
        wait(&f, "failed").await;
        assert_eq!(content(&f).await, "second-version");
        assert_eq!(
            f.request(
                "POST",
                &format!("{base}/rollback"),
                json!({"deployment_id":first["id"]})
            )
            .await
            .status,
            StatusCode::ACCEPTED
        );
        wait(&f, "succeeded").await;
        assert_eq!(content(&f).await, "first-version");
    })
    .catch_unwind()
    .await;
    stop.send(true).unwrap();
    worker.await.unwrap();
    let _ = f
        .request(
            "DELETE",
            &format!("/api/v1/services/{}", f.service),
            json!({"name":"Git web","remove_data":true}),
        )
        .await;
    // Only this fixture's labelled build images are disposable.
    let images = f
        .state
        .docker
        .list_images(None::<bollard::query_parameters::ListImagesOptions>)
        .await
        .unwrap();
    for image in images {
        if image.labels.get("io.ferrix.application") == Some(&f.service.to_string()) {
            let _ = f
                .state
                .docker
                .remove_image(
                    &image.id,
                    Some(
                        bollard::query_parameters::RemoveImageOptionsBuilder::default()
                            .force(true)
                            .build(),
                    ),
                    None,
                )
                .await;
        }
    }
    f.finish().await;
    if let Err(e) = result {
        std::panic::resume_unwind(e)
    }
}
