use crate::{ApiError, AppState, auth::Identity};
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(sqlx::query_as::<_, Project>("SELECT p.id, p.name, p.description, p.created_at, (SELECT count(*) FROM applications a JOIN environments e ON e.id=a.environment_id WHERE e.project_id=p.id AND a.service_kind IN ('application','database')) AS service_count FROM projects p WHERE p.organization_id=$1 ORDER BY p.created_at DESC, p.id DESC LIMIT 100")
        .bind(id.organization_id).fetch_all(&state.pool).await?))
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
