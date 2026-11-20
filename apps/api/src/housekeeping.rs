//! Bounded cleanup of expired authentication records; no deployment/audit retention.
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::watch;

const BATCH_SIZE: i64 = 1_000;
const MAX_BATCHES: usize = 10;
const INTERVAL: Duration = Duration::from_secs(15 * 60);

// These identifiers/expressions are fixed program constants, never request input.
const TARGETS: &[(&str, &str, &str, &str)] = &[
    (
        "auth_sessions",
        "id",
        "LEAST(expires_at, absolute_expires_at)",
        "true",
    ),
    ("auth_throttles", "key", "reset_at", "true"),
    ("auth_invitations", "id", "expires_at", "true"),
    ("auth_password_resets", "user_id", "expires_at", "true"),
    ("auth_mfa", "user_id", "pending_expires_at", "NOT enabled"),
];

/// Delete at most 10,000 expired rows per target, committing each batch separately.
/// Locked rows are skipped so token consumption/renewal and other cleaners can proceed.
pub async fn run_once(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let mut total = 0;
    for &(table, key, deadline, predicate) in TARGETS {
        let query = format!(
            "WITH expired AS (SELECT {key} FROM {table} WHERE {predicate} AND {deadline} <= now() \
             ORDER BY {deadline}, {key} LIMIT $1 FOR UPDATE SKIP LOCKED) \
             DELETE FROM {table} t USING expired e WHERE t.{key}=e.{key}"
        );
        for _ in 0..MAX_BATCHES {
            let mut tx = pool.begin().await?;
            sqlx::query("SET LOCAL statement_timeout='5s'")
                .execute(&mut *tx)
                .await?;
            sqlx::query("SET LOCAL lock_timeout='1s'")
                .execute(&mut *tx)
                .await?;
            let removed = sqlx::query(&query)
                .bind(BATCH_SIZE)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            tx.commit().await?;
            total += removed;
            if removed < BATCH_SIZE as u64 {
                break;
            }
            tokio::task::yield_now().await;
        }
    }
    Ok(total)
}

/// Run once at startup and then every fifteen minutes; idle time uses no connection.
pub async fn run_worker(pool: PgPool, mut shutdown: watch::Receiver<bool>) {
    let mut timer = tokio::time::interval(INTERVAL);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => return,
            _ = timer.tick() => {}
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => return,
            result = run_once(&pool) => match result {
                Ok(0) => {},
                Ok(removed) => tracing::info!(removed, "Expired authentication records cleaned"),
                Err(error) => tracing::warn!(%error, "Authentication housekeeping failed; retrying next interval"),
            }
        }
    }
}
