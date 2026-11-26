//! Per-logical-database scheduling; storage and execution reuse the shared service.
use super::*;

pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/services/{id}/databases/{database}/backup-policy",
            get(get_policy).post(save),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}

#[derive(Serialize, sqlx::FromRow)]
struct Policy {
    enabled: bool,
    cron: String,
    timezone: String,
    keep_last: i32,
    #[serde(skip)]
    configured_by: Option<Uuid>,
    next_run_at: Option<DateTime<Utc>>,
    last_run_at: Option<DateTime<Utc>>,
    last_message: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    enabled: bool,
    cron: String,
    timezone: String,
    keep_last: i32,
}
async fn authorize(
    state: &AppState,
    id: &Identity,
    service: Uuid,
    database: Uuid,
) -> Result<(), ApiError> {
    access(state, id, service).await?;
    id.manager()?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_databases WHERE id=$1 AND service_id=$2 AND organization_id=$3)")
        .bind(database).bind(service).bind(id.organization_id).fetch_one(&state.pool).await?;
    if !exists {
        return Err(missing());
    }
    Ok(())
}
async fn get_policy(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &id, service, database).await?;
    let saved: Option<Policy> =
        sqlx::query_as("SELECT * FROM managed_backup_policies WHERE managed_database_id=$1")
            .bind(database)
            .fetch_optional(&state.pool)
            .await?;
    Ok(Json(saved.map_or_else(||json!({"enabled":false,"cron":"0 2 * * *","timezone":"UTC","keep_last":10,"next_run_at":null,"last_run_at":null,"last_message":null}), |p|json!(p))))
}
async fn save(
    State(state): State<AppState>,
    id: Identity,
    Path((service, database)): Path<(Uuid, Uuid)>,
    Json(edit): Json<Edit>,
) -> Result<StatusCode, ApiError> {
    authorize(&state, &id, service, database).await?;
    if !(1..=365).contains(&edit.keep_last) {
        return Err(invalid("Keep between 1 and 365 successful backups."));
    }
    let next = policy::next(edit.cron.trim(), edit.timezone.trim(), Utc::now())?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(service)
        .execute(&mut *tx)
        .await?;
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM managed_databases WHERE id=$1 AND service_id=$2 FOR UPDATE",
    )
    .bind(database)
    .bind(service)
    .fetch_optional(&mut *tx)
    .await?;
    if status.is_none() {
        return Err(missing());
    }
    if edit.enabled && status.as_deref() != Some("ready") {
        return Err(invalid(
            "Logical database must be ready before enabling its schedule.",
        ));
    }
    sqlx::query("INSERT INTO managed_backup_policies(managed_database_id,enabled,cron,timezone,keep_last,configured_by,next_run_at) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(managed_database_id) DO UPDATE SET enabled=EXCLUDED.enabled,cron=EXCLUDED.cron,timezone=EXCLUDED.timezone,keep_last=EXCLUDED.keep_last,configured_by=EXCLUDED.configured_by,next_run_at=EXCLUDED.next_run_at,last_message=NULL,updated_at=now()")
        .bind(database).bind(edit.enabled).bind(edit.cron.trim()).bind(edit.timezone.trim()).bind(edit.keep_last).bind(id.user_id).bind(edit.enabled.then_some(next)).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

type DeploymentSnapshot = (String, String, Vec<u8>, Option<Vec<u8>>, Uuid);

pub(super) async fn schedule_due(state: &AppState, conn: &mut PgConnection) -> Result<(), String> {
    use sqlx::Acquire;
    let mut tx = conn
        .begin()
        .await
        .map_err(|_| "Could not start logical backup scheduler.")?;
    // Same lock order as manual backup, policy edits and database lifecycle operations.
    let services: Vec<Uuid> = sqlx::query_scalar("SELECT a.id FROM applications a WHERE EXISTS(SELECT 1 FROM managed_databases m JOIN managed_backup_policies p ON p.managed_database_id=m.id WHERE m.service_id=a.id AND p.enabled AND p.next_run_at<=now()) AND NOT EXISTS(SELECT 1 FROM project_maintenance pm JOIN environments e ON e.project_id=pm.project_id WHERE e.id=a.environment_id) ORDER BY a.id LIMIT 32 FOR UPDATE OF a SKIP LOCKED")
        .fetch_all(&mut *tx).await.map_err(|_|"Could not claim logical schedules.")?;
    for service in services {
        let due: Vec<(Uuid,String,String)> = sqlx::query_as("SELECT m.id,m.database_name,m.status FROM managed_databases m JOIN managed_backup_policies p ON p.managed_database_id=m.id WHERE m.service_id=$1 AND p.enabled AND p.next_run_at<=now() ORDER BY p.next_run_at,m.id FOR UPDATE OF m,p")
            .bind(service).fetch_all(&mut *tx).await.map_err(|_|"Could not read logical schedules.")?;
        for (database, name, status) in due {
            let p: Policy = sqlx::query_as(
                "SELECT * FROM managed_backup_policies WHERE managed_database_id=$1",
            )
            .bind(database)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "Could not load logical schedule.")?;
            let allowed: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects pr ON pr.id=e.project_id JOIN auth_members m ON m.organization_id=pr.organization_id JOIN auth_users u ON u.id=m.user_id WHERE a.id=$1 AND u.id=$2 AND m.role IN ('owner','admin') AND NOT auth_user_banned(u.banned,u.ban_expires_at))")
                .bind(service).bind(p.configured_by).fetch_one(&mut *tx).await.map_err(|_|"Could not verify logical schedule permissions.")?;
            if !allowed {
                sqlx::query("UPDATE managed_backup_policies SET enabled=false,next_run_at=NULL,last_message='Schedule paused: its owner no longer has permission.' WHERE managed_database_id=$1").bind(database).execute(&mut *tx).await.map_err(|_|"Could not pause logical schedule.")?;
                continue;
            }
            let next = policy::next(&p.cron, &p.timezone, Utc::now())
                .map_err(|_| "Invalid saved logical schedule.")?;
            let busy: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1 AND status IN ('queued','running')) OR EXISTS(SELECT 1 FROM database_jobs WHERE status IN ('queued','running') AND (application_id=$1 OR source_application_id=$1))")
                .bind(service).fetch_one(&mut *tx).await.map_err(|_|"Could not check shared service state.")?;
            // Leave the due time intact so databases sharing a minute run sequentially,
            // rather than silently skipping all but the first database every day.
            if busy {
                sqlx::query("UPDATE managed_backup_policies SET last_message='Waiting: shared service has a backup, restore or deployment pending.' WHERE managed_database_id=$1").bind(database).execute(&mut *tx).await.map_err(|_|"Could not defer logical schedule.")?;
                continue;
            }
            let snapshot: Option<DeploymentSnapshot>=sqlx::query_as("SELECT d.image,d.image_id,d.service_cipher,d.env_cipher,d.id FROM applications a JOIN deployments d ON d.id=a.active_deployment_id AND d.application_id=a.id WHERE a.id=$1 AND a.active_container_id IS NOT NULL AND d.image_id IS NOT NULL AND d.image LIKE 'mariadb:%' AND NOT a.restore_blocked AND NOT a.cutover_pending")
                .bind(service).fetch_optional(&mut *tx).await.map_err(|_|"Could not snapshot shared service.")?;
            let mut message =
                "Skipped: logical database is not ready or shared MariaDB is not deployed.";
            if let Some((image, image_id, cipher, env, deployment)) =
                snapshot.filter(|_| status == "ready")
            {
                let mut config: DatabaseConfig = services::open(state, service, &cipher)
                    .map_err(|_| "Could not read shared configuration.")?;
                config.database_name = name.clone();
                let cipher = services::seal(state, service, &config)
                    .map_err(|_| "Could not snapshot logical configuration.")?;
                let job = Uuid::new_v4();
                let (s3, key) = policy::snapshot(state, &mut tx, service, job)
                    .await
                    .map_err(|_| "Could not snapshot logical storage.")?;
                sqlx::query("INSERT INTO database_jobs(id,application_id,requested_by,operation,engine,image,image_id,config_cipher,env_cipher,deployment_id,managed_database_id,managed_database_name,scheduled_for,s3_cipher,s3_key) VALUES($1,$2,$3,'backup','mariadb',$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) ON CONFLICT DO NOTHING")
                    .bind(job).bind(service).bind(p.configured_by).bind(image).bind(image_id).bind(cipher).bind(env).bind(deployment).bind(database).bind(name).bind(p.next_run_at).bind(s3).bind(key).execute(&mut *tx).await.map_err(|_|"Could not queue logical backup.")?;
                message = "Scheduled logical database backup queued.";
            }
            sqlx::query("UPDATE managed_backup_policies SET next_run_at=$2,last_run_at=now(),last_message=$3 WHERE managed_database_id=$1").bind(database).bind(next).bind(message).execute(&mut *tx).await.map_err(|_|"Could not advance logical schedule.")?;
        }
    }
    tx.commit()
        .await
        .map_err(|_| "Could not persist logical schedules.")?;
    Ok(())
}
