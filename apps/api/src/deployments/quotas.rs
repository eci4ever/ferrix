//! Atomic allocation accounting; all reservations include current, active and queued snapshots.
use super::*;
use std::collections::BTreeMap;
type Totals = BTreeMap<(String, Uuid), Usage>;
#[derive(Clone, Default, Serialize)]
struct Usage {
    memory_mib: i64,
    cpu_centi: i64,
    services: i64,
    databases: i64,
    deployments: i64,
    builds: i64,
    storage_bytes: i64,
    storage_samples: i64,
}
impl Usage {
    fn add(&mut self, v: &Self) {
        self.memory_mib += v.memory_mib;
        self.cpu_centi += v.cpu_centi;
        self.services += v.services;
        self.databases += v.databases;
        self.deployments += v.deployments;
        self.builds += v.builds;
        self.storage_bytes += v.storage_bytes;
        self.storage_samples += v.storage_samples;
    }
}
#[derive(Serialize, Deserialize, sqlx::FromRow, Default)]
#[serde(deny_unknown_fields)]
struct Limits {
    memory_mib: Option<i64>,
    cpu_centi: Option<i64>,
    services: Option<i64>,
    databases: Option<i64>,
    deployments: Option<i64>,
    builds: Option<i64>,
    storage_bytes: Option<i64>,
    #[serde(default)]
    reserved_memory_mib: i64,
    #[serde(default)]
    reserved_cpu_centi: i64,
}
#[derive(sqlx::FromRow)]
struct ScopeLimits {
    scope: String,
    scope_id: Uuid,
    #[sqlx(flatten)]
    limits: Limits,
}
#[derive(sqlx::FromRow)]
struct Allocation {
    id: Uuid,
    project: Uuid,
    organization: Uuid,
    kind: String,
    image: String,
    cipher: Option<Vec<u8>>,
    storage: Option<i64>,
}
fn accumulate(map: &mut Totals, p: Uuid, o: Uuid, u: &Usage) {
    for key in [("project", p), ("organization", o), ("host", Uuid::nil())] {
        map.entry((key.0.into(), key.1)).or_default().add(u);
    }
}
async fn usage(state: &AppState, conn: &mut sqlx::PgConnection) -> Result<Totals, ApiError> {
    let rows:Vec<Allocation>=sqlx::query_as("SELECT a.id,e.project_id AS project,p.organization_id AS organization,a.service_kind AS kind,a.image,a.service_cipher AS cipher,(SELECT size_bytes FROM service_storage_usage s WHERE s.application_id=a.id AND checked_at>now()-interval '5 minutes') AS storage FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id").fetch_all(&mut *conn).await?;
    let mut totals = Totals::new();
    for row in rows {
        let mut u = Usage {
            services: 1,
            databases: i64::from(row.kind == "database"),
            storage_bytes: row.storage.unwrap_or(0),
            storage_samples: i64::from(row.storage.is_some()),
            ..Default::default()
        };
        if row.kind == "application" {
            let runtime = application_runtime::decode(state, row.id, row.cipher.as_deref())?;
            u.memory_mib = runtime.memory_mib;
            u.cpu_centi = (runtime.cpus * 100.).round() as i64;
        } else {
            u.memory_mib = if row.image.starts_with("mongo:") {
                1024
            } else {
                512
            };
            u.cpu_centi = 100;
        }
        let snapshots:Vec<(Option<Vec<u8>>,String,bool)>=sqlx::query_as("SELECT d.service_cipher,d.status,(d.source_cipher IS NOT NULL OR d.source_upload_id IS NOT NULL) AND d.pinned_image_id IS NULL AS build FROM deployments d WHERE d.application_id=$1 AND (d.id=(SELECT active_deployment_id FROM applications WHERE id=$1) OR d.status IN ('queued','running'))").bind(row.id).fetch_all(&mut *conn).await?;
        let mut active = (0, 0);
        let mut pending = (0, 0);
        let mut pending_exclusive = false;
        for (cipher, status, build) in snapshots {
            if matches!(status.as_str(), "queued" | "running") {
                u.deployments += 1;
                u.builds += i64::from(build);
            }
            if row.kind == "application" {
                let runtime = application_runtime::decode(state, row.id, cipher.as_deref())?;
                if matches!(status.as_str(), "queued" | "running") {
                    pending.0 = pending.0.max(runtime.memory_mib);
                    pending.1 = pending.1.max((runtime.cpus * 100.).round() as i64);
                    pending_exclusive |= runtime.exclusive();
                } else {
                    active = (runtime.memory_mib, (runtime.cpus * 100.).round() as i64);
                }
                u.memory_mib = u.memory_mib.max(runtime.memory_mib);
                u.cpu_centi = u.cpu_centi.max((runtime.cpus * 100.).round() as i64);
            }
        }
        if !pending_exclusive {
            u.memory_mib = u.memory_mib.max(active.0 + pending.0);
            u.cpu_centi = u.cpu_centi.max(active.1 + pending.1);
        }
        accumulate(&mut totals, row.project, row.organization, &u);
    }
    let orphaned:Vec<(Option<Uuid>,Option<Uuid>,i64)>=sqlx::query_as("SELECT project_id,organization_id,size_bytes FROM storage_volume_usage WHERE application_id IS NULL AND size_bytes IS NOT NULL AND checked_at>now()-interval '5 minutes'").fetch_all(&mut *conn).await?;
    for (p, o, bytes) in orphaned {
        let u = Usage {
            storage_bytes: bytes,
            storage_samples: 1,
            ..Default::default()
        };
        if let (Some(p), Some(o)) = (p, o) {
            accumulate(&mut totals, p, o, &u);
        } else {
            totals
                .entry(("host".into(), Uuid::nil()))
                .or_default()
                .add(&u);
        }
    }
    Ok(totals)
}
pub(super) struct Reservation {
    before: Totals,
    enabled: bool,
}
pub(super) async fn reserve(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<Reservation, ApiError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_database()||current_schema()||':resource-quotas',0))").execute(&mut *conn).await?;
    let enabled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resource_quotas)")
        .fetch_one(&mut *conn)
        .await?;
    Ok(Reservation {
        before: if enabled {
            usage(state, conn).await?
        } else {
            Totals::new()
        },
        enabled,
    })
}
pub(super) async fn check(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    reserved: Reservation,
) -> Result<(), ApiError> {
    if !reserved.enabled {
        return Ok(());
    }
    let after = usage(state, conn).await?;
    let policies: Vec<ScopeLimits> = sqlx::query_as("SELECT * FROM resource_quotas")
        .fetch_all(&mut *conn)
        .await?;
    for policy in policies {
        let key = (policy.scope, policy.scope_id);
        let empty = Usage::default();
        let old = reserved.before.get(&key).unwrap_or(&empty);
        let new = after.get(&key).unwrap_or(&empty);
        let l = policy.limits;
        for (before, now, ceiling, message) in [
            (
                old.memory_mib,
                new.memory_mib,
                l.memory_mib.map(|v| (v - l.reserved_memory_mib).max(0)),
                "Memory allocation quota exhausted. Lower service limits or ask a platform administrator to review capacity.",
            ),
            (
                old.cpu_centi,
                new.cpu_centi,
                l.cpu_centi.map(|v| (v - l.reserved_cpu_centi).max(0)),
                "CPU allocation quota exhausted. Lower service limits or ask a platform administrator to review capacity.",
            ),
            (
                old.services,
                new.services,
                l.services,
                "Service count quota exhausted. Delete unused services or request a larger quota.",
            ),
            (
                old.databases,
                new.databases,
                l.databases,
                "Database count quota exhausted. Delete unused databases or request a larger quota.",
            ),
            (
                old.deployments,
                new.deployments,
                l.deployments,
                "Deployment reservation quota exhausted. Wait for queued/running deployments to finish.",
            ),
            (
                old.builds,
                new.builds,
                l.builds,
                "Build reservation quota exhausted. Wait for queued/running builds to finish.",
            ),
        ] {
            if ceiling.is_some_and(|limit| now > limit && now > before) {
                return Err(ApiError(StatusCode::CONFLICT, message));
            }
        }
    }
    Ok(())
}
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/projects/{id}/quota", get(project).post(save_project))
        .route(
            "/organizations/quota",
            get(organization).post(save_organization),
        )
        .route("/admin/capacity", get(host).post(save_host))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}
async fn view(state: &AppState, scope: &str, key: Uuid) -> Result<Json<Value>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    let totals = usage(state, &mut conn).await?;
    let limit: Option<Limits> =
        sqlx::query_as("SELECT * FROM resource_quotas WHERE scope=$1 AND scope_id=$2")
            .bind(scope)
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?;
    let measured:(i64,Option<i64>,Option<f64>)=sqlx::query_as("WITH latest AS (SELECT DISTINCT ON(s.service_id) s.service_id,s.data FROM telemetry_samples s JOIN applications a ON a.id=s.service_id JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE NOT s.error AND s.collected_at>now()-interval '2 minutes' AND s.container_id=a.active_container_id AND ($1='host' OR ($1='project' AND p.id=$2) OR ($1='organization' AND p.organization_id=$2)) ORDER BY s.service_id,s.collected_at DESC,s.id DESC) SELECT count(*)::bigint,sum((data->>'memory_used_bytes')::bigint)::bigint,sum((data->>'cpu_cores_used')::float) FROM latest").bind(scope).bind(key).fetch_one(&mut *conn).await?;
    Ok(Json(
        json!({"measured":{"services_sampled":measured.0,"memory_used_bytes":measured.1,"cpu_cores_used":measured.2,"partial":true},"scope":scope,"scope_id":key,"limits":limit,"allocated":totals.get(&(scope.into(),key)).cloned().unwrap_or_default(),"storage_enforcement":"advisory","storage_note":"Docker local volumes have no hard byte quota. Storage samples may be unavailable or delayed; stopped services retain storage. Alerts report measured overages.","reservation_policy":"Maximum configured/active/pending limits per service; queued and running jobs reserve slots. Existing over-quota allocations may be reduced but not increased."}),
    ))
}
async fn project(
    State(s): State<AppState>,
    id: Identity,
    Path(p): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    project_access(&s, &id, p).await?;
    view(&s, "project", p).await
}
async fn organization(State(s): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    view(&s, "organization", id.organization_id).await
}
async fn host(State(s): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    view(&s, "host", Uuid::nil()).await
}
async fn save(
    s: &AppState,
    id: &Identity,
    scope: &str,
    key: Uuid,
    l: Limits,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    if [
        l.memory_mib,
        l.cpu_centi,
        l.services,
        l.databases,
        l.deployments,
        l.builds,
        l.storage_bytes,
        Some(l.reserved_memory_mib),
        Some(l.reserved_cpu_centi),
    ]
    .iter()
    .flatten()
    .any(|v| *v < 0 || *v > 1_000_000_000_000_000)
        || (scope != "host" && (l.reserved_memory_mib != 0 || l.reserved_cpu_centi != 0))
    {
        return Err(invalid(
            "Use non-negative quotas. Host headroom applies only to host capacity.",
        ));
    }
    let mut tx = s.pool.begin().await?;
    let _ = reserve(s, &mut tx).await?;
    sqlx::query("INSERT INTO resource_quotas(scope,scope_id,memory_mib,cpu_centi,services,databases,deployments,builds,storage_bytes,reserved_memory_mib,reserved_cpu_centi,updated_by) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT(scope,scope_id) DO UPDATE SET memory_mib=EXCLUDED.memory_mib,cpu_centi=EXCLUDED.cpu_centi,services=EXCLUDED.services,databases=EXCLUDED.databases,deployments=EXCLUDED.deployments,builds=EXCLUDED.builds,storage_bytes=EXCLUDED.storage_bytes,reserved_memory_mib=EXCLUDED.reserved_memory_mib,reserved_cpu_centi=EXCLUDED.reserved_cpu_centi,updated_by=EXCLUDED.updated_by,updated_at=now()").bind(scope).bind(key).bind(l.memory_mib).bind(l.cpu_centi).bind(l.services).bind(l.databases).bind(l.deployments).bind(l.builds).bind(l.storage_bytes).bind(l.reserved_memory_mib).bind(l.reserved_cpu_centi).bind(id.user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn save_project(
    State(s): State<AppState>,
    id: Identity,
    Path(p): Path<Uuid>,
    Json(l): Json<Limits>,
) -> Result<StatusCode, ApiError> {
    project_access(&s, &id, p).await?;
    save(&s, &id, "project", p, l).await
}
async fn save_organization(
    State(s): State<AppState>,
    id: Identity,
    Json(l): Json<Limits>,
) -> Result<StatusCode, ApiError> {
    save(&s, &id, "organization", id.organization_id, l).await
}
async fn save_host(
    State(s): State<AppState>,
    id: Identity,
    Json(l): Json<Limits>,
) -> Result<StatusCode, ApiError> {
    save(&s, &id, "host", Uuid::nil(), l).await
}

#[cfg(test)]
mod tests;

mod storage;
pub use storage::run_worker;
