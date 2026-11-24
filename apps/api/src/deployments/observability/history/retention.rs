use super::*;
pub(super) async fn prune(state: &AppState, settings: &Settings) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "DELETE FROM service_log_history WHERE collected_at<now()-make_interval(days=>$1::int)",
    )
    .bind(settings.log_retention_days as i32)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM telemetry_samples WHERE collected_at<now()-make_interval(days=>$1::int)",
    )
    .bind(settings.retention_days as i32)
    .execute(&mut *tx)
    .await?;
    // Retain boundary and peak observations in older buckets; do not erase error/state markers.
    sqlx::query("WITH buckets AS (SELECT id,first_value(id) OVER w AS first,last_value(id) OVER w AS last,first_value(id) OVER (PARTITION BY service_id,deployment_id,floor(extract(epoch FROM collected_at)/$2) ORDER BY COALESCE((data->>'cpu_percent_of_limit')::float,(data->>'cpu')::float) DESC NULLS LAST,id) AS cpu_peak,first_value(id) OVER (PARTITION BY service_id,deployment_id,floor(extract(epoch FROM collected_at)/$2) ORDER BY COALESCE((data->>'memory_used_bytes')::bigint,(data->>'memory_used')::bigint) DESC NULLS LAST,id) AS memory_peak FROM telemetry_samples WHERE collected_at<now()-make_interval(mins=>$1::int) WINDOW w AS (PARTITION BY service_id,deployment_id,floor(extract(epoch FROM collected_at)/$2) ORDER BY collected_at,id ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)) DELETE FROM telemetry_samples s USING buckets b WHERE s.id=b.id AND s.id NOT IN(b.first,b.last,b.cpu_peak,b.memory_peak) AND NOT s.error AND NOT s.marker").bind(settings.downsample_after_minutes as i32).bind(settings.downsample_seconds as f64).execute(&mut *tx).await?;
    // Bound retained logical payload across both tables. Index/WAL/physical PostgreSQL files need normal vacuum/operations.
    sqlx::query("WITH entries AS MATERIALIZED (SELECT 'log' AS kind,id,collected_at,octet_length(message)+256 AS bytes FROM service_log_history UNION ALL SELECT 'sample',id,collected_at,octet_length(data::text)+512 FROM telemetry_samples), ranked AS MATERIALIZED (SELECT *,sum(bytes) OVER (ORDER BY collected_at DESC,kind,id DESC) AS total FROM entries), removed AS (DELETE FROM service_log_history WHERE id IN(SELECT id FROM ranked WHERE kind='log' AND total>$1) RETURNING id) DELETE FROM telemetry_samples WHERE id IN(SELECT id FROM ranked WHERE kind='sample' AND total>$1)").bind(settings.max_storage_mib*1048576).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
