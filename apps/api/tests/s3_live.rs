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
