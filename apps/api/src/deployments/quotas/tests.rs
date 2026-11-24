use super::*;
use crate::deployments::test_support::*;
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn concurrent_reservations_and_grandfathered_capacity_are_enforced() {
    let f = Fixture::new().await;
    let result=AssertUnwindSafe(async{
  let (env,project):(Uuid,Uuid)=sqlx::query_as("SELECT a.environment_id,e.project_id FROM applications a JOIN environments e ON e.id=a.environment_id WHERE a.id=$1").bind(f.service).fetch_one(&f.state.pool).await.unwrap();
  let quota=format!("/api/v1/projects/{project}/quota");
  assert_eq!(f.request("POST",&quota,json!({"services":2,"memory_mib":1024,"cpu_centi":200,"deployments":1})).await.status,StatusCode::NO_CONTENT);
  let path=format!("/api/v1/environments/{env}/services");let a=json!({"kind":"application","name":"A","image":"nginx:alpine","container_port":80,"environment":""});let mut b=a.clone();b["name"]=json!("B");
  let (a,b)=tokio::join!(f.request("POST",&path,a),f.request("POST",&path,b));
  assert_eq!([a.status,b.status].into_iter().filter(|s|*s==StatusCode::CREATED).count(),1);assert_eq!([a.status,b.status].into_iter().filter(|s|*s==StatusCode::CONFLICT).count(),1);
  let other=if a.status==StatusCode::CREATED{a.body["id"].as_str().unwrap().to_owned()}else{b.body["id"].as_str().unwrap().to_owned()};
  let larger=json!({"name":"Git web","image":"nginx:alpine","container_port":80,"environment":"","memory_mib":768,"cpus":1});
  assert_eq!(f.request("POST",&format!("/api/v1/services/{}/configuration",f.service),larger).await.status,StatusCode::CONFLICT);
  let first_deploy=format!("/api/v1/services/{}/deploy",f.service);let other_deploy=format!("/api/v1/services/{other}/deploy");
  let (a,b)=tokio::join!(f.request("POST",&first_deploy,json!({})),f.request("POST",&other_deploy,json!({})));
  assert_eq!([a.status,b.status].into_iter().filter(|s|*s==StatusCode::ACCEPTED).count(),1);assert_eq!([a.status,b.status].into_iter().filter(|s|*s==StatusCode::CONFLICT).count(),1);
  // A new lower ceiling preserves current allocations and permits reductions.
  assert_eq!(f.request("POST",&quota,json!({"services":0,"memory_mib":256})).await.status,StatusCode::NO_CONTENT);
  let lower=json!({"name":"Git web","image":"nginx:alpine","container_port":80,"environment":"","memory_mib":256,"cpus":0.5});
  assert_eq!(f.request("POST",&format!("/api/v1/services/{}/configuration",f.service),lower).await.status,StatusCode::OK);
  sqlx::query("UPDATE auth_users SET system_role='user' WHERE id=$1").bind(f.user).execute(&f.state.pool).await.unwrap();
  assert_eq!(f.request("POST",&quota,json!({"services":100})).await.status,StatusCode::FORBIDDEN);
  assert_eq!(f.request("GET",&format!("/api/v1/projects/{}/quota",Uuid::new_v4()),Value::Null).await.status,StatusCode::NOT_FOUND);
 }).catch_unwind().await;
    f.finish().await;
    if let Err(e) = result {
        std::panic::resume_unwind(e)
    }
}
