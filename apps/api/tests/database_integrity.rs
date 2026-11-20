use ferrix_api::housekeeping;
use futures_util::FutureExt;
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};
use std::{borrow::Cow, future::Future, panic::AssertUnwindSafe};
use uuid::Uuid;

async fn isolated<F, Fut>(check: F)
where
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = ()>,
{
    let url = std::env::var("FERRIX_TEST_DATABASE_URL").expect("Use your own development database");
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("integrity_test_{}", Uuid::new_v4().simple());
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
    let result = AssertUnwindSafe(check(pool.clone())).catch_unwind().await;
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
async fn organization(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,'Integrity fixture')")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    id
}
async fn environment(pool: &PgPool, org: Uuid) -> (Uuid, Uuid) {
    let project = Uuid::new_v4();
    let env = Uuid::new_v4();
    sqlx::query("INSERT INTO projects(id,name,organization_id) VALUES($1,'Fixture',$2)")
        .bind(project)
        .bind(org)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'production')")
        .bind(env)
        .bind(project)
        .execute(pool)
        .await
        .unwrap();
    (project, env)
}
async fn application(pool: &PgPool, env: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port) VALUES($1,$2,$3,'nginx:alpine',80)").bind(id).bind(env).bind(id.to_string()).execute(pool).await.unwrap();
    id
}
async fn deployment(pool: &PgPool, app: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,status) VALUES($1,$2,'nginx:alpine',80,'succeeded')").bind(id).bind(app).execute(pool).await.unwrap();
    id
}
fn code(error: sqlx::Error, expected: &str) {
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some(expected)
    );
}

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn active_deployments_are_service_scoped_and_allow_atomic_activation_and_deletion() {
    isolated(|pool| async move {
        sqlx::migrate!().run(&pool).await.unwrap();
        let org=organization(&pool).await;
        code(sqlx::query("INSERT INTO projects(id,name) VALUES($1,'No owner')").bind(Uuid::new_v4()).execute(&pool).await.unwrap_err(),"23502");
        let (project,env)=environment(&pool,org).await;
        let app=application(&pool,env).await;let other=application(&pool,env).await;
        let active=deployment(&pool,app).await;let foreign=deployment(&pool,other).await;
        sqlx::query("UPDATE applications SET active_deployment_id=$1 WHERE id=$2").bind(active).bind(app).execute(&pool).await.unwrap();
        code(sqlx::query("UPDATE applications SET active_deployment_id=$1 WHERE id=$2").bind(foreign).bind(app).execute(&pool).await.unwrap_err(),"23503");
        code(sqlx::query("UPDATE applications SET active_deployment_id=$1 WHERE id=$2").bind(Uuid::new_v4()).bind(app).execute(&pool).await.unwrap_err(),"23503");
        code(sqlx::query("DELETE FROM deployments WHERE id=$1").bind(active).execute(&pool).await.unwrap_err(),"23503");
        // The constraint is deferred: the pointer and its target can be written
        // in either order, but must be consistent when the transaction commits.
        let next=Uuid::new_v4();let mut tx=pool.begin().await.unwrap();
        sqlx::query("UPDATE applications SET active_deployment_id=$1 WHERE id=$2").bind(next).bind(app).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,status) VALUES($1,$2,'nginx:alpine',80,'succeeded')").bind(next).bind(app).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        sqlx::query("DELETE FROM deployments WHERE id=$1").bind(active).execute(&pool).await.unwrap();
        sqlx::query("UPDATE applications SET active_deployment_id=$1 WHERE id=$2").bind(foreign).bind(other).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM applications WHERE id=$1").bind(app).execute(&pool).await.unwrap();
        let left:i64=sqlx::query_scalar("SELECT count(*) FROM deployments").fetch_one(&pool).await.unwrap();assert_eq!(left,1);
        sqlx::query("DELETE FROM projects WHERE id=$1").bind(project).execute(&pool).await.unwrap();
        let left:i64=sqlx::query_scalar("SELECT count(*) FROM deployments").fetch_one(&pool).await.unwrap();assert_eq!(left,0);
    }).await;
}

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn migration_rejects_bad_legacy_data_without_guessing_ownership_or_losing_projects() {
    isolated(|pool| async move {
        let mut all = sqlx::migrate!();
        // Retry deliberately failing migrations in this disposable schema only.
        // SQLx keeps its session advisory lock on error; production exits on
        // migration errors instead of reusing this test's pooled connections.
        all.set_locking(false);
        let previous = sqlx::migrate::Migrator {
            migrations: Cow::Owned(all.iter().filter(|m| m.version < 11).cloned().collect()),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        previous.run(&pool).await.unwrap();
        let legacy = Uuid::new_v4();
        sqlx::query("INSERT INTO projects(id,name) VALUES($1,'Legacy project')")
            .bind(legacy)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            all.run(&pool)
                .await
                .unwrap_err()
                .to_string()
                .contains("without organization ownership")
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM projects WHERE id=$1 AND organization_id IS NULL",
        )
        .bind(legacy)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        let org = organization(&pool).await;
        sqlx::query("UPDATE projects SET organization_id=$1 WHERE id=$2")
            .bind(org)
            .bind(legacy)
            .execute(&pool)
            .await
            .unwrap();
        let (_, env) = environment(&pool, org).await;
        let app = application(&pool, env).await;
        sqlx::query("UPDATE applications SET active_deployment_id=$1 WHERE id=$2")
            .bind(Uuid::new_v4())
            .bind(app)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            all.run(&pool)
                .await
                .unwrap_err()
                .to_string()
                .contains("applications_active_deployment_fkey")
        );
        sqlx::query("UPDATE applications SET active_deployment_id=NULL WHERE id=$1")
            .bind(app)
            .execute(&pool)
            .await
            .unwrap();
        all.run(&pool).await.unwrap();
        all.run(&pool).await.unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM projects WHERE id=$1 AND organization_id=$2")
                .bind(legacy)
                .bind(org)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires FERRIX_TEST_DATABASE_URL"]
async fn housekeeping_deletes_only_expired_records_skips_renewals_and_bounds_work() {
    isolated(|pool| async move {
        sqlx::migrate!().run(&pool).await.unwrap();
        let org=organization(&pool).await;let mut users=Vec::new();
        for index in 0..3 {
            let user=Uuid::new_v4();users.push(user);
            sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) VALUES($1,'Fixture',$2,'unused')").bind(user).bind(format!("fixture{index}@example.test")).execute(&pool).await.unwrap();
        }
        let user=users[0];
        sqlx::query("INSERT INTO auth_sessions(id,user_id,active_organization_id,token_hash,expires_at,absolute_expires_at) VALUES(gen_random_uuid(),$1,$2,'expired-inactivity',now()-interval '1 minute',now()+interval '1 hour'),(gen_random_uuid(),$1,$2,'expired-absolute',now()+interval '1 hour',now()-interval '1 minute'),(gen_random_uuid(),$1,$2,'valid',now()+interval '1 hour',now()+interval '2 hours')").bind(user).bind(org).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO auth_throttles(key,attempts,reset_at) VALUES('expired',1,now()-interval '1 minute'),('locked',2,now()-interval '1 minute'),('valid',3,now()+interval '1 hour')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO auth_invitations(id,organization_id,email,role,token_hash,invited_by,expires_at,accepted_at) VALUES(gen_random_uuid(),$1,'fixture@example.test','member','expired',$2,now()-interval '1 minute',NULL),(gen_random_uuid(),$1,'fixture@example.test','member','consumed-expired',$2,now()-interval '1 minute',now()-interval '2 minutes'),(gen_random_uuid(),$1,'fixture@example.test','member','valid',$2,now()+interval '1 hour',NULL)").bind(org).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO auth_password_resets(user_id,token_hash,expires_at) VALUES($1,'expired',now()-interval '1 minute'),($2,'valid',now()+interval '1 hour')").bind(user).bind(users[1]).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO auth_mfa(user_id,secret,enabled,pending_expires_at) VALUES($1,$4,false,now()-interval '1 minute'),($2,$4,false,now()+interval '1 hour'),($3,$4,true,now()-interval '1 minute')").bind(user).bind(users[1]).bind(users[2]).bind(vec![7_u8;40]).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO auth_recovery_codes(user_id,code_hash) VALUES($1,'keep')").bind(users[2]).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO auth_audit(id,user_id,organization_id,action,created_at) VALUES(gen_random_uuid(),$1,$2,'keep',now()-interval '1 year')").bind(user).bind(org).execute(&pool).await.unwrap();
        let (_,env)=environment(&pool,org).await;let app=application(&pool,env).await;deployment(&pool,app).await;
        let mut renewal=pool.begin().await.unwrap();
        sqlx::query("SELECT key FROM auth_throttles WHERE key='locked' FOR UPDATE").execute(&mut *renewal).await.unwrap();
        assert_eq!(housekeeping::run_once(&pool).await.unwrap(),7);
        sqlx::query("UPDATE auth_throttles SET reset_at=now()+interval '1 hour' WHERE key='locked'").execute(&mut *renewal).await.unwrap();
        renewal.commit().await.unwrap();
        assert_eq!(housekeeping::run_once(&pool).await.unwrap(),0);
        for (table,expected) in [("auth_users",3),("auth_sessions",1),("auth_invitations",1),("auth_password_resets",1),("auth_mfa",2),("auth_throttles",2),("auth_audit",1),("auth_recovery_codes",1),("applications",1),("deployments",1)] {
            let count:i64=sqlx::query_scalar(&format!("SELECT count(*) FROM {table}")).fetch_one(&pool).await.unwrap();assert_eq!(count,expected,"{table}");
        }
        sqlx::query("INSERT INTO auth_throttles(key,attempts,reset_at) SELECT 'bulk-' || n,1,now()-interval '1 hour' FROM generate_series(1,10005) n").execute(&pool).await.unwrap();
        assert_eq!(housekeeping::run_once(&pool).await.unwrap(),10000);
        let (first,second)=tokio::join!(housekeeping::run_once(&pool),housekeeping::run_once(&pool));
        assert_eq!(first.unwrap()+second.unwrap(),5);
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM auth_throttles").fetch_one(&pool).await.unwrap();assert_eq!(count,2);
        sqlx::query("INSERT INTO auth_throttles(key,attempts,reset_at) VALUES('startup-worker',1,now()-interval '1 hour')").execute(&pool).await.unwrap();
        let (stop,receiver)=tokio::sync::watch::channel(false);
        let worker=tokio::spawn(housekeeping::run_worker(pool.clone(),receiver));
        tokio::time::timeout(std::time::Duration::from_secs(5),async {
            loop {
                let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_throttles WHERE key='startup-worker')").fetch_one(&pool).await.unwrap();
                if !exists { break; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        stop.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2),worker).await.unwrap().unwrap();
    }).await;
}
