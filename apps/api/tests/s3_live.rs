//! Optional live S3 fixture, launched by scripts/tests/backup-s3.py.
#[tokio::test]
#[ignore = "requires the disposable S3 runner"]
async fn prepare_disposable_s3_bucket() {
    let Ok(endpoint) = std::env::var("FERRIX_TEST_S3_ENDPOINT") else {
        return;
    };
    assert_eq!(
        std::env::var("FERRIX_TEST_S3_DISPOSABLE").as_deref(),
        Ok("1")
    );
    let url = url::Url::parse(&endpoint).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let bucket = std::env::var("FERRIX_TEST_S3_BUCKET").unwrap();
    assert!(bucket.starts_with("ferrix-test-"));
    let credentials = s3::creds::Credentials::new(
        Some(&std::env::var("FERRIX_TEST_S3_ACCESS_KEY").unwrap()),
        Some(&std::env::var("FERRIX_TEST_S3_SECRET_KEY").unwrap()),
        None,
        None,
        None,
    )
    .unwrap();
    let response = s3::Bucket::create_with_path_style(
        &bucket,
        s3::Region::Custom {
            region: "us-east-1".into(),
            endpoint,
        },
        credentials,
        s3::BucketConfiguration::default(),
    )
    .await
    .unwrap();
    assert!((200..300).contains(&response.response_code));
}

/// Independent SDK reads prove logical retention removed only the selected remote object.
#[tokio::test]
#[ignore = "requires the disposable logical S3 runner and object assertions"]
async fn verify_disposable_s3_objects() {
    use sha2::{Digest, Sha256};
    let Ok(objects) = std::env::var("FERRIX_TEST_S3_OBJECTS") else {
        return;
    };
    assert_eq!(
        std::env::var("FERRIX_TEST_S3_DISPOSABLE").as_deref(),
        Ok("1")
    );
    let endpoint = std::env::var("FERRIX_TEST_S3_ENDPOINT").unwrap();
    assert_eq!(
        url::Url::parse(&endpoint).unwrap().host_str(),
        Some("127.0.0.1")
    );
    let name = std::env::var("FERRIX_TEST_S3_BUCKET").unwrap();
    assert!(name.starts_with("ferrix-test-"));
    let credentials = s3::creds::Credentials::new(
        Some(&std::env::var("FERRIX_TEST_S3_ACCESS_KEY").unwrap()),
        Some(&std::env::var("FERRIX_TEST_S3_SECRET_KEY").unwrap()),
        None,
        None,
        None,
    )
    .unwrap();
    let bucket = s3::Bucket::new(
        &name,
        s3::Region::Custom {
            region: "us-east-1".into(),
            endpoint,
        },
        credentials,
    )
    .unwrap()
    .with_path_style();
    let cases: Vec<(String, Option<String>)> = serde_json::from_str(&objects).unwrap();
    assert!(!cases.is_empty());
    for (key, checksum) in cases {
        assert!(key.starts_with("logical-audit/ferrix/"));
        let response = bucket.get_object(&key).await;
        if let Some(checksum) = checksum {
            let response = response.unwrap();
            assert_eq!(response.status_code(), 200);
            assert_eq!(hex::encode(Sha256::digest(response.as_slice())), checksum);
        } else {
            assert!(
                matches!(response, Err(s3::error::S3Error::HttpFailWithBody(404, _))),
                "Expired object must return 404, not a generic connection/permission error"
            );
        }
    }
}
