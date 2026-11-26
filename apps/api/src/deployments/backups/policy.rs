use super::*;
use croner::Cron;
use std::str::FromStr;

pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/services/{id}/backup-policy", get(get_policy).post(save))
        .route("/services/{id}/backup-policy/test", post(test))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}
#[derive(sqlx::FromRow)]
pub(super) struct Policy {
    pub enabled: bool,
    pub application_offline: bool,
    pub cron: String,
    pub timezone: String,
    pub keep_last: i32,
    pub s3_cipher: Option<Vec<u8>>,
    pub configured_by: Option<Uuid>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_message: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    enabled: bool,
    #[serde(default)]
    application_offline: bool,
    cron: String,
    timezone: String,
    keep_last: i32,
    s3: Option<remote::EditDestination>,
}
pub(super) fn next(cron: &str, zone: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, ApiError> {
    if cron.split_whitespace().count() != 5 || cron.len() > 128 {
        return Err(invalid(
            "Use a five-field cron expression, for example 0 2 * * *.",
        ));
    }
    let tz = chrono_tz::Tz::from_str(zone).map_err(|_| invalid("Choose a valid IANA timezone."))?;
    let cron = Cron::from_str(cron).map_err(|_| invalid("Invalid backup cron expression."))?;
    cron.find_next_occurrence(&now.with_timezone(&tz), false)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| invalid("This schedule has no upcoming run."))
}
async fn get_policy(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    access(&state, &id, service).await?;
    id.manager()?;
    let policy: Option<Policy> =
        sqlx::query_as("SELECT * FROM backup_policies WHERE application_id=$1")
            .bind(service)
            .fetch_optional(&state.pool)
            .await?;
    if let Some(policy) = policy {
        let s3 = policy
            .s3_cipher
            .as_ref()
            .map(|cipher| remote::open(&state, service, cipher).map(|s3| s3.redacted()))
            .transpose()?;
        Ok(Json(
            json!({"application_offline":policy.application_offline,"enabled":policy.enabled,"cron":policy.cron,"timezone":policy.timezone,"keep_last":policy.keep_last,"s3":s3,"next_run_at":policy.next_run_at,"last_run_at":policy.last_run_at,"last_message":policy.last_message}),
        ))
    } else {
        Ok(Json(
            json!({"application_offline":false,"enabled":false,"cron":"0 2 * * *","timezone":"UTC","keep_last":10,"s3":null,"next_run_at":null,"last_run_at":null,"last_message":null}),
        ))
    }
}
async fn save(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(edit): Json<Edit>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    let app = access(&state, &id, service).await?;
    if app.kind == "application" && edit.enabled && !edit.application_offline {
        return Err(invalid(
            "Acknowledge offline downtime before enabling Application backup schedules.",
        ));
    }
    if !(1..=365).contains(&edit.keep_last) {
        return Err(invalid("Keep between 1 and 365 successful backups."));
    }
    let next = next(edit.cron.trim(), edit.timezone.trim(), Utc::now())?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .execute(&mut *tx)
        .await?;
    let previous: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT s3_cipher FROM backup_policies WHERE application_id=$1")
            .bind(service)
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
    let cipher = if let Some(edit) = edit.s3 {
        let previous = previous
            .as_deref()
            .map(|c| remote::open(&state, service, c))
            .transpose()?;
        let destination = remote::validate(&state, edit, previous)?;
        Some(remote::seal(&state, service, &destination)?)
    } else {
        None
    };
    sqlx::query("INSERT INTO backup_policies(application_id,enabled,cron,timezone,keep_last,s3_cipher,configured_by,next_run_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(application_id) DO UPDATE SET enabled=EXCLUDED.enabled,cron=EXCLUDED.cron,timezone=EXCLUDED.timezone,keep_last=EXCLUDED.keep_last,s3_cipher=EXCLUDED.s3_cipher,configured_by=EXCLUDED.configured_by,next_run_at=EXCLUDED.next_run_at,updated_at=now(),last_message=NULL")
 .bind(service).bind(edit.enabled).bind(edit.cron.trim()).bind(edit.timezone.trim()).bind(edit.keep_last).bind(cipher).bind(id.user_id).bind(if edit.enabled{Some(next)}else{None}).execute(&mut *tx).await?;
    sqlx::query("UPDATE backup_policies SET application_offline=$2 WHERE application_id=$1")
        .bind(service)
        .bind(edit.application_offline)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn test(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    access(&state, &id, service).await?;
    crate::auth::accounts::throttle(&state.pool, &format!("backup-s3-test:{}", id.user_id), 10)
        .await?;
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT s3_cipher FROM backup_policies WHERE application_id=$1")
            .bind(service)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let cipher = cipher.ok_or_else(|| invalid("Save an S3 destination before testing it."))?;
    let destination = remote::open(&state, service, &cipher)?;
    tokio::time::timeout(Duration::from_secs(30), destination.test(service))
        .await
        .map_err(|_| invalid("S3 connection test timed out."))?
        .map_err(|_| {
            invalid(
                "S3 test failed. Check endpoint, bucket, region and read/write/delete permissions.",
            )
        })?;
    Ok(Json(
        json!({"message":"S3 read, write and delete access verified."}),
    ))
}
pub(super) async fn snapshot(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    service: Uuid,
    job: Uuid,
) -> Result<(Option<Vec<u8>>, Option<String>), ApiError> {
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT s3_cipher FROM backup_policies WHERE application_id=$1")
            .bind(service)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();
    let key = cipher
        .as_ref()
        .map(|c| remote::open(state, service, c).map(|d| d.key(service, job)))
        .transpose()?;
    Ok((cipher, key))
}
pub(super) async fn schedule_due(state: &AppState, conn: &mut PgConnection) -> Result<(), String> {
    use sqlx::Acquire;
    let mut tx = conn
        .begin()
        .await
        .map_err(|_| "Could not start backup scheduler.")?;
    // Lock service rows first, matching configuration/deploy/restore lock order.
    let services:Vec<Uuid>=sqlx::query_scalar("SELECT a.id FROM applications a JOIN backup_policies p ON p.application_id=a.id WHERE NOT EXISTS(SELECT 1 FROM project_maintenance m JOIN environments e ON e.project_id=m.project_id WHERE e.id=a.environment_id) AND p.enabled AND p.next_run_at<=now() ORDER BY p.next_run_at LIMIT 32 FOR UPDATE OF a SKIP LOCKED").fetch_all(&mut *tx).await.map_err(|_|"Could not claim backup schedules.")?;
    for service in services {
        let policy: Policy =
            sqlx::query_as("SELECT * FROM backup_policies WHERE application_id=$1 FOR UPDATE")
                .bind(service)
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| "Could not read backup schedule.")?;
        if !policy.enabled || policy.next_run_at.is_none_or(|time| time > Utc::now()) {
            continue;
        }
        let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id JOIN auth_members m ON m.organization_id=p.organization_id JOIN auth_users u ON u.id=m.user_id WHERE a.id=$1 AND u.id=$2 AND m.role IN ('owner','admin') AND NOT auth_user_banned(u.banned,u.ban_expires_at))").bind(service).bind(policy.configured_by).fetch_one(&mut *tx).await.map_err(|_|"Could not verify schedule permissions.")?;
        if !allowed {
            sqlx::query("UPDATE backup_policies SET enabled=false,next_run_at=NULL,last_message='Schedule paused: its owner no longer has permission.' WHERE application_id=$1").bind(service).execute(&mut *tx).await.map_err(|_|"Could not pause invalid schedule.")?;
            continue;
        }
        let next = next(&policy.cron, &policy.timezone, Utc::now())
            .map_err(|_| "Invalid saved backup schedule.")?;
        let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND status IN ('queued','running')) OR EXISTS(SELECT 1 FROM database_jobs WHERE status IN ('queued','running') AND (application_id=$1 OR source_application_id=$1))").bind(service).fetch_one(&mut *tx).await.map_err(|_|"Could not check scheduled database state.")?;
        let mut message = "Skipped: a backup, restore or deployment is already pending.";
        if !busy {
            let job = Uuid::new_v4();
            let key = policy
                .s3_cipher
                .as_ref()
                .map(|c| remote::open(state, service, c).map(|d| d.key(service, job)))
                .transpose()
                .map_err(|_| "Could not read scheduled S3 destination.")?;
            let count=sqlx::query("INSERT INTO database_jobs(id,application_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,deployment_id,scheduled_for,s3_cipher,s3_key) SELECT $1,a.id,$2,'backup',CASE WHEN a.service_kind='application' THEN 'application' WHEN d.image LIKE 'mariadb:%' THEN 'mariadb' ELSE 'postgres' END,d.image,d.image_id,d.service_cipher,d.env_cipher,d.id,$4,$5,$6 FROM applications a JOIN deployments d ON d.id=a.active_deployment_id AND d.application_id=a.id WHERE a.id=$3 AND a.active_container_id IS NOT NULL AND d.image_id IS NOT NULL AND NOT a.restore_blocked AND ((a.service_kind='database' AND (d.image LIKE 'postgres:%' OR d.image LIKE 'mariadb:%')) OR (a.service_kind='application' AND $7)) ON CONFLICT DO NOTHING")
   .bind(job).bind(policy.configured_by).bind(service).bind(policy.next_run_at).bind(policy.s3_cipher).bind(key).bind(policy.application_offline).execute(&mut *tx).await.map_err(|_|"Could not queue scheduled backup.")?.rows_affected();
            if count == 1 {
                sqlx::query("UPDATE database_jobs SET application_port=(SELECT container_port FROM deployments WHERE id=database_jobs.deployment_id) WHERE id=$1").bind(job).execute(&mut *tx).await.map_err(|_|"Could not snapshot Application port.")?;
            }
            message = if count == 1 {
                "Scheduled backup queued."
            } else {
                "Skipped: service has no eligible deployed version."
            };
        }
        sqlx::query("UPDATE backup_policies SET next_run_at=$2,last_run_at=now(),last_message=$3 WHERE application_id=$1").bind(service).bind(next).bind(message).execute(&mut *tx).await.map_err(|_|"Could not advance backup schedule.")?;
    }
    tx.commit()
        .await
        .map_err(|_| "Could not persist scheduled backups.")?;
    Ok(())
}
pub(super) async fn mark_excess(conn: &mut PgConnection, service: Uuid) -> Result<(), String> {
    use sqlx::Acquire;
    let mut tx = conn
        .begin()
        .await
        .map_err(|_| "Could not start retention selection.")?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .execute(&mut *tx)
        .await
        .map_err(|_| "Could not lock backup retention.")?;
    sqlx::query("UPDATE database_jobs SET pruning=true,prune_retry_at=now() WHERE id IN (SELECT id FROM (SELECT id,row_number() OVER (PARTITION BY managed_database_id ORDER BY created_at DESC,id DESC) AS position FROM database_jobs WHERE application_id=$1 AND operation='backup' AND status='succeeded' AND pruned_at IS NULL) scoped WHERE position > (SELECT keep_last FROM backup_policies WHERE application_id=$1)) AND EXISTS(SELECT 1 FROM backup_policies WHERE application_id=$1) AND NOT EXISTS(SELECT 1 FROM database_jobs r WHERE r.source_backup_id=database_jobs.id AND r.status IN ('queued','running'))")
 .bind(service).execute(&mut *tx).await.map_err(|_|"Could not apply backup retention policy.")?;
    tx.commit()
        .await
        .map_err(|_| "Could not persist backup retention selection.")?;
    Ok(())
}
pub(super) async fn prune_one(state: &AppState, conn: &mut PgConnection) -> Result<(), String> {
    use sqlx::Acquire;
    let mut tx = conn
        .begin()
        .await
        .map_err(|_| "Could not start backup retention.")?;
    let service:Option<Uuid>=sqlx::query_scalar("SELECT a.id FROM applications a WHERE EXISTS(SELECT 1 FROM database_jobs j WHERE j.application_id=a.id AND j.pruning AND j.pruned_at IS NULL AND j.prune_retry_at<=now()) ORDER BY a.id LIMIT 1 FOR UPDATE OF a SKIP LOCKED").fetch_optional(&mut *tx).await.map_err(|_|"Could not claim backup retention.")?;
    let Some(service) = service else {
        return Ok(());
    };
    let job:Option<Job>=sqlx::query_as("SELECT * FROM database_jobs j WHERE j.application_id=$1 AND j.pruning AND j.pruned_at IS NULL AND j.prune_retry_at<=now() AND NOT EXISTS(SELECT 1 FROM database_jobs r WHERE r.source_backup_id=j.id AND r.status IN ('queued','running')) ORDER BY j.created_at,j.id LIMIT 1 FOR UPDATE").bind(service).fetch_optional(&mut *tx).await.map_err(|_|"Could not read expired backup.")?;
    let Some(job) = job else { return Ok(()) };
    let result = tokio::time::timeout(Duration::from_secs(30), remote::remove(state, &job))
        .await
        .unwrap_or_else(|_| Err("Backup retention timed out.".into()));
    let cleanup = tokio::time::timeout(Duration::from_secs(15), cleanup_helper(state, &job))
        .await
        .unwrap_or_else(|_| Err("Retention helper cleanup timed out.".into()));
    match result.and(cleanup) {
        Ok(()) => {
            sqlx::query("UPDATE database_jobs SET pruned_at=now(),pruning=false,retention_error=NULL,prune_retry_at=NULL WHERE id=$1").bind(job.id).execute(&mut *tx).await.map_err(|_|"Could not record backup retention.")?;
        }
        Err(message) => {
            sqlx::query("UPDATE database_jobs SET retention_error=$2,prune_retry_at=now()+interval '5 minutes' WHERE id=$1").bind(job.id).bind(message).execute(&mut *tx).await.map_err(|_|"Could not record retention retry.")?;
        }
    }
    tx.commit()
        .await
        .map_err(|_| "Could not persist backup retention.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cron_timezone_and_validation() {
        let now = DateTime::parse_from_rfc3339("2026-11-23T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            next("0 2 * * *", "Asia/Kuala_Lumpur", now)
                .unwrap()
                .to_rfc3339(),
            "2026-11-23T18:00:00+00:00"
        );
        assert!(next("* * * * * *", "UTC", now).is_err());
        assert!(next("0 0 * * *", "bad-zone", now).is_err());
        assert!(next("bad", "UTC", now).is_err());
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use futures_util::FutureExt;
    use sqlx::{Executor, postgres::PgPoolOptions};
    use std::{panic::AssertUnwindSafe, sync::Arc};
    #[tokio::test]
    #[ignore = "requires FERRIX_TEST_DATABASE_URL"]
    async fn schedules_are_durable_authorized_and_retention_protects_restore() {
        let url = std::env::var("FERRIX_TEST_DATABASE_URL").unwrap();
        let admin = PgPoolOptions::new().connect(&url).await.unwrap();
        let schema = format!("backup_policy_test_{}", Uuid::new_v4().simple());
        admin
            .execute(format!("CREATE SCHEMA {schema}").as_str())
            .await
            .unwrap();
        let namespace = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(8)
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
        let result=AssertUnwindSafe(async{
   sqlx::migrate!().run(&pool).await.unwrap();
   let state=AppState{pool:pool.clone(),gateway:Default::default(),docker:bollard::Docker::connect_with_local_defaults().unwrap(),auth:Arc::new(crate::auth::AuthConfig{origin:"http://localhost:5173".into(),secure:false,cookie_name:"policy_test".into(),key:[42;32]})};
   let user=Uuid::new_v4();let org=Uuid::new_v4();let project=Uuid::new_v4();let environment=Uuid::new_v4();let service=Uuid::new_v4();let deployment=Uuid::new_v4();
   sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) VALUES($1,'Schedule owner','schedule@example.com','not-a-login-password')").bind(user).execute(&pool).await.unwrap();
   sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,'Schedule organization')").bind(org).execute(&pool).await.unwrap();
   sqlx::query("INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'owner')").bind(org).bind(user).execute(&pool).await.unwrap();
   sqlx::query("INSERT INTO projects(id,name,organization_id) VALUES($1,'Schedule project',$2)").bind(project).bind(org).execute(&pool).await.unwrap();
   sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'production')").bind(environment).bind(project).execute(&pool).await.unwrap();
   sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind,service_cipher) VALUES($1,$2,'Database','postgres:17-alpine',5432,'database','\\x00')").bind(service).bind(environment).execute(&pool).await.unwrap();
   sqlx::query("INSERT INTO deployments(id,application_id,image,container_port,service_kind,service_cipher,image_id,status) VALUES($1,$2,'postgres:17-alpine',5432,'database','\\x00','sha256:fixture','succeeded')").bind(deployment).bind(service).execute(&pool).await.unwrap();
   sqlx::query("UPDATE applications SET active_deployment_id=$2,active_container_id='test-only' WHERE id=$1").bind(service).bind(deployment).execute(&pool).await.unwrap();
   let destination=remote::Destination{endpoint:"http://127.0.0.1:9000".into(),region:"us-east-1".into(),bucket:"test-bucket".into(),prefix:"test".into(),path_style:true,credentials:remote::Keys{access_key:"disposable-access".into(),secret_key:"disposable-secret".into(),session_token:None}};
   let cipher=remote::seal(&state,service,&destination).unwrap();assert!(remote::open(&state,Uuid::new_v4(),&cipher).is_err());assert!(!destination.redacted().to_string().contains("disposable"));
   sqlx::query("INSERT INTO backup_policies(application_id,enabled,cron,timezone,keep_last,configured_by,next_run_at,s3_cipher) VALUES($1,true,'0 2 * * *','Asia/Kuala_Lumpur',1,$2,now()-interval '2 days',$3)").bind(service).bind(user).bind(&cipher).execute(&pool).await.unwrap();
   let mut conn=pool.acquire().await.unwrap();schedule_due(&state,&mut conn).await.unwrap();schedule_due(&state,&mut conn).await.unwrap();
   type Queued=(Uuid,Option<DateTime<Utc>>,Vec<u8>,String);
   let jobs:Vec<Queued>=sqlx::query_as("SELECT id,scheduled_for,s3_cipher,s3_key FROM database_jobs WHERE application_id=$1").bind(service).fetch_all(&pool).await.unwrap();assert_eq!(jobs.len(),1);assert!(jobs[0].1.is_some());assert_eq!(jobs[0].2,cipher);assert_eq!(jobs[0].3,destination.key(service,jobs[0].0));
   sqlx::query("UPDATE backup_policies SET next_run_at=now()-interval '1 hour' WHERE application_id=$1").bind(service).execute(&pool).await.unwrap();schedule_due(&state,&mut conn).await.unwrap();
   assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM database_jobs WHERE application_id=$1").bind(service).fetch_one(&pool).await.unwrap(),1);
   let message:String=sqlx::query_scalar("SELECT last_message FROM backup_policies WHERE application_id=$1").bind(service).fetch_one(&pool).await.unwrap();assert!(message.contains("Skipped"));
   // The queued snapshot survives policy replacement or removal.
   sqlx::query("UPDATE backup_policies SET s3_cipher=NULL WHERE application_id=$1").bind(service).execute(&pool).await.unwrap();
   assert_eq!(sqlx::query_scalar::<_,Vec<u8>>("SELECT s3_cipher FROM database_jobs WHERE id=$1").bind(jobs[0].0).fetch_one(&pool).await.unwrap(),cipher);
   sqlx::query("UPDATE auth_members SET role='member' WHERE user_id=$1").bind(user).execute(&pool).await.unwrap();sqlx::query("UPDATE backup_policies SET next_run_at=now()-interval '1 hour' WHERE application_id=$1").bind(service).execute(&pool).await.unwrap();schedule_due(&state,&mut conn).await.unwrap();assert!(!sqlx::query_scalar::<_,bool>("SELECT enabled FROM backup_policies WHERE application_id=$1").bind(service).fetch_one(&pool).await.unwrap());
   sqlx::query("UPDATE database_jobs SET status='succeeded',created_at=now()-interval '3 days' WHERE id=$1").bind(jobs[0].0).execute(&pool).await.unwrap();
   let older=Uuid::new_v4();let newest=Uuid::new_v4();for(id,days)in[(older,2),(newest,1)]{sqlx::query("INSERT INTO database_jobs(id,application_id,operation,engine,image,image_id,config_cipher,status,created_at) VALUES($1,$2,'backup','postgres','postgres:17-alpine','sha256:fixture','\\x00','succeeded',now()-$3*interval '1 day')").bind(id).bind(service).bind(days as f64).execute(&pool).await.unwrap();}
   let restore=Uuid::new_v4();sqlx::query("INSERT INTO database_jobs(id,application_id,source_application_id,source_backup_id,operation,engine,image,image_id,config_cipher,status) VALUES($1,$2,$2,$3,'restore','postgres','postgres:17-alpine','sha256:fixture','\\x00','queued')").bind(restore).bind(service).bind(jobs[0].0).execute(&pool).await.unwrap();
   mark_excess(&mut conn,service).await.unwrap();
   for(id,expected)in[(jobs[0].0,false),(older,true),(newest,false)]{assert_eq!(sqlx::query_scalar::<_,bool>("SELECT pruning FROM database_jobs WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap(),expected)}
  }).catch_unwind().await;
        pool.close().await;
        admin
            .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
            .await
            .unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error)
        }
    }
}
