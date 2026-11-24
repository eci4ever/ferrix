use super::*;
use std::time::Duration;
async fn collect(state: &AppState) -> Result<(), ApiError> {
    let items = crate::monitoring::volume_inventory()
        .await
        .map_err(|_| internal())?;
    let mut conn = state.pool.acquire().await?;
    let owned:Vec<(Uuid,Uuid,Uuid)>=sqlx::query_as("SELECT a.id,e.project_id,p.organization_id FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id").fetch_all(&mut *conn).await?;
    let mut seen = Vec::new();
    for item in items {
        let Ok(v) = serde_json::from_value::<bollard::models::Volume>(item) else {
            continue;
        };
        seen.push(v.name.clone());
        if v.driver != "local"
            || v.labels.get("io.ferrix.managed").map(String::as_str) != Some("true")
        {
            continue;
        }
        let size = v.usage_data.map(|s| s.size).filter(|s| *s >= 0);
        let app = v
            .labels
            .get("io.ferrix.application")
            .and_then(|v| Uuid::parse_str(v).ok());
        if let Some((a, p, o)) = owned.iter().find(|(a, _, _)| Some(*a) == app) {
            sqlx::query("INSERT INTO storage_volume_usage(volume_name,application_id,project_id,organization_id,size_bytes) VALUES($1,$2,$3,$4,$5) ON CONFLICT(volume_name) DO UPDATE SET application_id=EXCLUDED.application_id,project_id=EXCLUDED.project_id,organization_id=EXCLUDED.organization_id,size_bytes=EXCLUDED.size_bytes,checked_at=now()").bind(&v.name).bind(a).bind(p).bind(o).bind(size).execute(&mut *conn).await?;
        } else {
            sqlx::query("UPDATE storage_volume_usage SET size_bytes=$2,checked_at=now() WHERE volume_name=$1 AND application_id IS NULL").bind(&v.name).bind(size).execute(&mut *conn).await?;
        }
    }
    // A successful complete volume inventory can retire ledger entries for removed volumes.
    sqlx::query("DELETE FROM storage_volume_usage WHERE NOT(volume_name=ANY($1))")
        .bind(&seen)
        .execute(&mut *conn)
        .await?;
    let totals = usage(state, &mut conn).await?;
    let policies: Vec<ScopeLimits> =
        sqlx::query_as("SELECT * FROM resource_quotas WHERE storage_bytes IS NOT NULL")
            .fetch_all(&mut *conn)
            .await?;
    for policy in policies {
        let key = (policy.scope.clone(), policy.scope_id);
        let u = totals.get(&key).cloned().unwrap_or_default();
        if u.storage_samples == 0 {
            continue;
        }
        let high = u.storage_bytes > policy.limits.storage_bytes.unwrap();
        let incomplete:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM storage_volume_usage WHERE ($1='host' OR ($1='project' AND project_id=$2) OR ($1='organization' AND organization_id=$2)) AND (size_bytes IS NULL OR checked_at<=now()-interval '5 minutes'))").bind(&policy.scope).bind(policy.scope_id).fetch_one(&mut *conn).await?;
        if !high && incomplete {
            continue;
        }
        let mut tx = state.pool.begin().await?;
        sqlx::query(
            "INSERT INTO quota_storage_alerts(scope,scope_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(&policy.scope)
        .bind(policy.scope_id)
        .execute(&mut *tx)
        .await?;
        let previous: bool = sqlx::query_scalar(
            "SELECT active FROM quota_storage_alerts WHERE scope=$1 AND scope_id=$2 FOR UPDATE",
        )
        .bind(&policy.scope)
        .bind(policy.scope_id)
        .fetch_one(&mut *tx)
        .await?;
        if previous != high {
            sqlx::query("UPDATE quota_storage_alerts SET active=$3 WHERE scope=$1 AND scope_id=$2")
                .bind(&policy.scope)
                .bind(policy.scope_id)
                .bind(high)
                .execute(&mut *tx)
                .await?;
            let projects:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT id,organization_id FROM projects WHERE $1='host' OR ($1='organization' AND organization_id=$2) OR ($1='project' AND id=$2)").bind(&policy.scope).bind(policy.scope_id).fetch_all(&mut *tx).await?;
            for (project, org) in projects {
                sqlx::query("INSERT INTO notifications(id,organization_id,project_id,kind,title,message) VALUES($1,$2,$3,'storage_quota',$4,'Measured managed-volume storage crossed its advisory threshold. Review Capacity and quotas; Docker local volumes have no hard byte limit.')").bind(Uuid::new_v4()).bind(org).bind(project).bind(if high{"Storage advisory threshold exceeded"}else{"Storage usage recovered"}).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
    }
    Ok(())
}
pub async fn run_worker(state: AppState, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut timer = tokio::time::interval(Duration::from_secs(120));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {_=stop.changed()=>break,_=timer.tick()=>{}}
        if collect(&state).await.is_err() {
            tracing::debug!(
                "Managed volume storage measurement unavailable; previous samples will expire"
            );
        }
    }
}
