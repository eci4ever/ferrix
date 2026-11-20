use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ferrix_api::{AppState, auth::AuthConfig, router};
use futures_util::FutureExt;
use sha2::{Digest, Sha256};
use sqlx::{Executor, postgres::PgPoolOptions};
use std::{borrow::Cow, panic::AssertUnwindSafe, sync::Arc};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL pointing to your own PostgreSQL"]
async fn retirement_then_cleanup_preserves_docker_accounts_and_services() {
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("retire_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let namespace = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(3)
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
    let result=AssertUnwindSafe(async {
        let all=sqlx::migrate!("./migrations");
        let previous=sqlx::migrate::Migrator {
            migrations:Cow::Owned(all.iter().filter(|m| m.version<9).cloned().collect()),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        previous.run(&pool).await.unwrap();
        let org=Uuid::new_v4();
        sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,'Preserved workspace')").bind(org).execute(&pool).await.unwrap();
        let mut tokens=Vec::new();
        for (index,role,hosting) in [(1,"admin",true),(2,"user",false),(3,"user",true)] {
            let user=Uuid::new_v4();
            let token=format!("{index:064x}");
            sqlx::query("INSERT INTO auth_users(id,name,email,password_hash,system_role) VALUES($1,'Legacy account',$2,'unused',$3)").bind(user).bind(format!("user{index}@example.test")).bind(role).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'owner')").bind(org).bind(user).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO auth_sessions(id,token_hash,user_id,active_organization_id) VALUES($1,$2,$3,$4)").bind(Uuid::new_v4()).bind(hex::encode(Sha256::digest(token.as_bytes()))).bind(user).bind(org).execute(&pool).await.unwrap();
            if hosting {
                sqlx::query("INSERT INTO student_hostings(user_id,organization_id,status,database_name,database_user,password_cipher) VALUES($1,$2,'active',$3,$4,$5)").bind(user).bind(org).bind(format!("db{index}")).bind(format!("user{index}")).bind(vec![7_u8;40]).execute(&pool).await.unwrap();
            }
            tokens.push((user,token));
        }
        sqlx::query("UPDATE student_hosting_settings SET enabled=true,registration_open=true,root_cipher=$1").bind(vec![9_u8;40]).execute(&pool).await.unwrap();
        let retired=sqlx::migrate::Migrator {
            migrations:Cow::Owned(all.iter().filter(|m| m.version<=9).cloned().collect()),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        retired.run(&pool).await.unwrap();
        // Re-running startup is safe and does not change preserved accounts or data.
        retired.run(&pool).await.unwrap();
        for (index,(user,_)) in tokens.iter().enumerate() {
            let banned:bool=sqlx::query_scalar("SELECT banned FROM auth_users WHERE id=$1").bind(user).fetch_one(&pool).await.unwrap();
            assert_eq!(banned,index==2);
        }
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM auth_sessions").fetch_one(&pool).await.unwrap();assert_eq!(count,2);
        let rows:i64=sqlx::query_scalar("SELECT count(*) FROM student_hostings WHERE status='suspended' AND password_cipher=$1").bind(vec![7_u8;40]).fetch_one(&pool).await.unwrap();assert_eq!(rows,2);
        let retired:bool=sqlx::query_scalar("SELECT NOT enabled AND NOT registration_open AND server_state='retired' AND root_cipher=$1 FROM student_hosting_settings").bind(vec![9_u8;40]).fetch_one(&pool).await.unwrap();assert!(retired);
        let project=Uuid::new_v4();let environment=Uuid::new_v4();
        sqlx::query("INSERT INTO projects(id,name,organization_id) VALUES($1,'Kept project',$2)").bind(project).bind(org).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'production')").bind(environment).bind(project).execute(&pool).await.unwrap();
        for kind in ["application","database","compose"] {
            sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind) VALUES($1,$2,$3,'nginx:alpine',80,$3)").bind(Uuid::new_v4()).bind(environment).bind(kind).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO static_websites(id,environment_id,name) VALUES($1,$2,'Removed site')").bind(Uuid::new_v4()).bind(environment).execute(&pool).await.unwrap();
        all.run(&pool).await.unwrap();all.run(&pool).await.unwrap();
        let kept:i64=sqlx::query_scalar("SELECT count(*) FROM applications").fetch_one(&pool).await.unwrap();assert_eq!(kept,2);
        let removed:bool=sqlx::query_scalar("SELECT to_regclass(current_schema() || '.static_websites') IS NULL AND to_regclass(current_schema() || '.student_hostings') IS NULL").fetch_one(&pool).await.unwrap();assert!(removed);
        let gone:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM auth_users WHERE id=$1)").bind(tokens[2].0).fetch_one(&pool).await.unwrap();assert!(gone);
        let app=router(AppState { pool:pool.clone(),gateway:Default::default(),docker:bollard::Docker::connect_with_local_defaults().unwrap(),auth:Arc::new(AuthConfig { origin:"http://127.0.0.1:5173".into(),secure:false,cookie_name:"test_session".into(),key:[42;32] }) });
        for (index,(_,token)) in tokens.iter().enumerate() {
            let response=app.clone().oneshot(Request::builder().uri("/api/v1/projects").header("cookie",format!("test_session={token}")).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(),if index==2 { StatusCode::UNAUTHORIZED } else { StatusCode::OK });
        }
        for (method,path) in [("GET","/api/v1/student-hosting"),("POST","/api/v1/auth/student-register"),("GET","/api/v1/auth/student-registration")] {
            let response=app.clone().oneshot(Request::builder().method(method).uri(path).header("origin","http://127.0.0.1:5173").body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(),StatusCode::NOT_FOUND);
        }
    }).catch_unwind().await;
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
