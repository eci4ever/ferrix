use crate::{ApiError, AppState, auth::Identity};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use bollard::query_parameters::ListContainersOptionsBuilder;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct RuntimeService {
    id: Uuid,
    project_id: Uuid,
    environment_id: Uuid,
    active_container_id: Option<String>,
    deployed: bool,
    last_failed: bool,
}

// A failed redeploy must not turn a still-running active container red.
fn bucket(
    state: Option<&str>,
    health: Option<&str>,
    deployed: bool,
    last_failed: bool,
) -> &'static str {
    match state {
        Some("dead") => "failed",
        Some("running") if health.is_some_and(|s| s.contains("(unhealthy)")) => "failed",
        Some("running") if health.is_some_and(|s| s.contains("(health: starting)")) => "other",
        Some("running") => "running",
        Some("exited" | "created" | "stopped") => "stopped",
        None if deployed || last_failed => "failed",
        _ => "other",
    }
}

pub(crate) async fn statuses(
    State(state): State<AppState>,
    id: Identity,
    key: Option<axum::Extension<crate::auth::api_keys::KeyAccess>>,
) -> Result<Json<Value>, ApiError> {
    let projects: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM projects WHERE organization_id=$1 AND ($2::UUID IS NULL OR id=$2) AND ferrix_project_visible($3,$1,id) ORDER BY created_at DESC,id DESC LIMIT 100")
        .bind(id.organization_id).bind(key.and_then(|key| key.project_id)).bind(id.user_id).fetch_all(&state.pool).await?;
    let services: Vec<RuntimeService> = sqlx::query_as("SELECT a.id,e.project_id,a.environment_id,a.active_container_id,(a.active_deployment_id IS NOT NULL) AS deployed,COALESCE((SELECT status='failed' FROM deployments WHERE application_id=a.id ORDER BY created_at DESC,id DESC LIMIT 1),false) AS last_failed FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE p.organization_id=$1 AND p.id=ANY($2) AND a.service_kind IN ('application','database') AND ferrix_service_permission($3,$1,a.id) IS NOT NULL")
        .bind(id.organization_id).bind(&projects).bind(id.user_id).fetch_all(&state.pool).await?;
    let filters = HashMap::from([(
        "label".to_owned(),
        vec![
            "io.ferrix.managed=true".to_owned(),
            format!("io.ferrix.organization={}", id.organization_id),
        ],
    )]);
    let inventory = if services.is_empty() {
        Some(vec![])
    } else {
        tokio::time::timeout(
            Duration::from_secs(5),
            state.docker.list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            )),
        )
        .await
        .ok()
        .and_then(Result::ok)
    };
    let mut summaries: HashMap<Uuid, Value> = projects
        .iter()
        .map(|p| {
            (
                *p,
                json!({"id":p,"running":0,"stopped":0,"failed":0,"other":0,"available":true}),
            )
        })
        .collect();
    for service in services {
        let summary = summaries
            .get_mut(&service.project_id)
            .expect("scoped project");
        let Some(containers) = inventory.as_ref() else {
            summary["available"] = json!(false);
            continue;
        };
        let container = containers.iter().find(|c| {
            c.id.is_some()
                && c.id == service.active_container_id
                && c.labels.as_ref().is_some_and(|l| {
                    l.get("io.ferrix.application") == Some(&service.id.to_string())
                        && l.get("io.ferrix.project") == Some(&service.project_id.to_string())
                        && l.get("io.ferrix.environment")
                            == Some(&service.environment_id.to_string())
                })
        });
        let state = container
            .and_then(|c| c.state.as_ref())
            .map(ToString::to_string);
        let key = bucket(
            state.as_deref(),
            container.and_then(|c| c.status.as_deref()),
            service.deployed,
            service.last_failed,
        );
        summary[key] = json!(summary[key].as_u64().unwrap_or(0) + 1);
    }
    Ok(Json(
        json!({"checked_at":chrono::Utc::now(),"projects":projects.iter().filter_map(|p| summaries.remove(p)).collect::<Vec<_>>()}),
    ))
}

#[cfg(test)]
mod runtime_tests {
    use super::bucket;
    #[test]
    fn distinguishes_runtime_from_deployment_failures_and_unknown_states() {
        assert_eq!(bucket(Some("running"), None, true, true), "running");
        assert_eq!(
            bucket(Some("running"), Some("Up (unhealthy)"), true, false),
            "failed"
        );
        assert_eq!(
            bucket(Some("running"), Some("Up (health: starting)"), true, false),
            "other"
        );
        assert_eq!(bucket(Some("exited"), None, true, false), "stopped");
        assert_eq!(bucket(None, None, true, false), "failed");
        assert_eq!(bucket(None, None, false, true), "failed");
        assert_eq!(bucket(None, None, false, false), "other");
        assert_eq!(bucket(Some("restarting"), None, true, false), "other");
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct Project {
    id: Uuid,
    name: String,
    description: String,
    created_at: DateTime<Utc>,
    service_count: i64,
}

#[derive(Deserialize)]
pub(crate) struct CreateProject {
    name: String,
    #[serde(default)]
    description: String,
}

impl CreateProject {
    fn validate(mut self) -> Result<Self, ApiError> {
        self.name = self.name.trim().to_owned();
        self.description = self.description.trim().to_owned();
        if self.name.is_empty() || self.name.chars().count() > 80 {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Project name must contain 1–80 characters.",
            ));
        }
        if self.description.chars().count() > 500 {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Description must contain at most 500 characters.",
            ));
        }
        Ok(self)
    }
}

pub(crate) async fn list(
    State(state): State<AppState>,
    id: Identity,
    key: Option<axum::Extension<crate::auth::api_keys::KeyAccess>>,
) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(sqlx::query_as::<_, Project>("SELECT p.id, p.name, p.description, p.created_at, (SELECT count(*) FROM applications a JOIN environments e ON e.id=a.environment_id WHERE e.project_id=p.id AND a.service_kind IN ('application','database') AND ferrix_service_permission($3,$1,a.id) IS NOT NULL) AS service_count FROM projects p WHERE p.organization_id=$1 AND ($2::UUID IS NULL OR p.id=$2) AND ferrix_project_visible($3,$1,p.id) ORDER BY p.created_at DESC, p.id DESC LIMIT 100")
        .bind(id.organization_id).bind(key.and_then(|key| key.project_id)).bind(id.user_id).fetch_all(&state.pool).await?))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    id.manager()?;
    let input = input.validate()?;
    let mut tx = state.pool.begin().await?;
    let project = sqlx::query_as::<_, Project>("INSERT INTO projects (id, name, description, organization_id) VALUES ($1, $2, $3, $4) RETURNING id, name, description, created_at, 0::bigint AS service_count")
        .bind(Uuid::new_v4()).bind(input.name).bind(input.description).bind(id.organization_id)
        .fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'production')")
        .bind(Uuid::new_v4())
        .bind(project.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[derive(Deserialize)]
pub(crate) struct DeleteProject {
    name: String,
}

pub(crate) async fn remove(
    State(state): State<AppState>,
    id: Identity,
    Path(project): Path<Uuid>,
    Json(input): Json<DeleteProject>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    let mut tx = state.pool.begin().await?;
    let name: Option<String> = sqlx::query_scalar(
        "SELECT name FROM projects WHERE id=$1 AND organization_id=$2 FOR UPDATE",
    )
    .bind(project)
    .bind(id.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    let name = name.ok_or(ApiError(
        StatusCode::NOT_FOUND,
        "Project not found in this organization.",
    ))?;
    if input.name != name {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Enter the project name exactly to confirm deletion.",
        ));
    }
    // Block new environments and service inserts through their foreign-key locks.
    sqlx::query("SELECT id FROM environments WHERE project_id=$1 ORDER BY id FOR UPDATE")
        .bind(project)
        .fetch_all(&mut *tx)
        .await?;
    let nonempty: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id WHERE e.project_id=$1)").bind(project).fetch_one(&mut *tx).await?;
    if nonempty {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "This project still has services. Delete all services before deleting the project.",
        ));
    }
    sqlx::query("DELETE FROM projects WHERE id=$1 AND organization_id=$2")
        .bind(project)
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
