use super::*;
use crate::deployments::test_support::*;
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
async fn app(f: &Fixture) -> Application {
    sqlx::query_as(&format!(
        "SELECT {APP_COLUMNS} FROM applications a WHERE id=$1"
    ))
    .bind(f.service)
    .fetch_one(&f.state.pool)
    .await
    .unwrap()
}
async fn deployed(f: &Fixture) -> Value {
    for _ in 0..240 {
        let data = f
            .request(
                "GET",
                &format!("/api/v1/services/{}/deployments", f.service),
                Value::Null,
            )
            .await
            .body;
        if data[0]["status"] == "succeeded" {
            return data[0].clone();
        }
        assert_ne!(data[0]["status"], "failed", "{data}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Deployment timed out")
}
fn range() -> String {
    format!(
        "from={}&to={}",
        (Utc::now() - chrono::Duration::hours(2))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        (Utc::now() + chrono::Duration::minutes(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )
}
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker"]
async fn retained_history_survives_redeploy_and_reopening_and_prunes() {
    let f = Fixture::new().await;
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(crate::deployments::run_worker(f.state.clone(), rx));
    let result=AssertUnwindSafe(async{
  let base=format!("/api/v1/services/{}",f.service);assert_eq!(f.request("POST",&format!("{base}/deploy"),json!({})).await.status,StatusCode::ACCEPTED);let first=deployed(&f).await;
  let config=settings(&f.state).await.unwrap();let cache=MetricsCache::default();
  let container=first["container_id"].as_str().unwrap();
  let output=tokio::process::Command::new("docker").args(["exec",container,"sh","-c","echo ferrix-retained-history-marker > /proc/1/fd/1"]).output().await.unwrap();assert!(output.status.success());
  collect::capture(&f.state,app(&f).await,&config,&cache).await.unwrap();
  let rows=f.request("GET",&format!("{base}/logs/history?{}&q=ferrix-retained-history-marker",range()),Value::Null).await;assert_eq!(rows.status,StatusCode::OK);assert!(rows.body["lines"].as_array().unwrap().iter().any(|v|v["message"].as_str().unwrap().contains("ferrix-retained-history-marker")));
  assert_eq!(f.request("POST",&format!("{base}/control"),json!({"operation":"reload"})).await.status,StatusCode::NO_CONTENT);
  collect::capture(&f.state,app(&f).await,&config,&cache).await.unwrap();
  assert_eq!(f.request("POST",&format!("{base}/deploy"),json!({})).await.status,StatusCode::ACCEPTED);let second=deployed(&f).await;assert_ne!(first["container_id"],second["container_id"]);
  collect::capture(&f.state,app(&f).await,&config,&cache).await.unwrap();
  let reopened=crate::router(f.state.clone());let rows=call(&reopened,"GET",&format!("{base}/history?{}",range()),&f.cookie,Value::Null,&[],false).await;assert_eq!(rows.status,StatusCode::OK);let samples=rows.body["samples"].as_array().unwrap();assert!(samples.iter().any(|p|p["deployment_id"]==first["id"]));assert!(samples.iter().any(|p|p["deployment_id"]==second["id"]&&p["marker"]==true));
  assert_eq!(f.request("GET",&format!("/api/v1/services/{}/history?{}",Uuid::new_v4(),range()),Value::Null).await.status,StatusCode::NOT_FOUND);
  sqlx::query("INSERT INTO telemetry_samples(service_id,collected_at,data) VALUES($1,now()-interval '5 days','{}')").bind(f.service).execute(&f.state.pool).await.unwrap();
  sqlx::query("INSERT INTO telemetry_samples(service_id,data) SELECT $1,jsonb_build_object('state','test','padding',repeat('x',1000000)) FROM generate_series(1,20)").bind(f.service).execute(&f.state.pool).await.unwrap();
  let mut capped=config.clone();capped.max_storage_mib=16;retention::prune(&f.state,&capped).await.unwrap();
  let bytes:i64=sqlx::query_scalar("SELECT COALESCE((SELECT sum(octet_length(data::text)+512) FROM telemetry_samples),0)::bigint+COALESCE((SELECT sum(octet_length(message)+256) FROM service_log_history),0)::bigint").fetch_one(&f.state.pool).await.unwrap();assert!(bytes<=16*1048576);
  let old:i64=sqlx::query_scalar("SELECT count(*) FROM telemetry_samples WHERE collected_at<now()-interval '3 days'").fetch_one(&f.state.pool).await.unwrap();assert_eq!(old,0);
  sqlx::query("UPDATE auth_users SET system_role='user' WHERE id=$1").bind(f.user).execute(&f.state.pool).await.unwrap();assert_eq!(f.request("GET","/api/v1/monitoring/settings",Value::Null).await.status,StatusCode::FORBIDDEN);
 }).catch_unwind().await;
    stop.send(true).unwrap();
    worker.await.unwrap();
    let _ = f
        .request(
            "DELETE",
            &format!("/api/v1/services/{}", f.service),
            json!({"name":"Git web","remove_data":true}),
        )
        .await;
    f.finish().await;
    if let Err(e) = result {
        std::panic::resume_unwind(e)
    }
}
