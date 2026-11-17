use crate::{ApiError, AppState};
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

pub(crate) async fn list(State(state): State<AppState>) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(sqlx::query_as::<_, Project>("SELECT id, name, description, created_at FROM projects ORDER BY created_at DESC, id DESC LIMIT 100")
        .fetch_all(&state.pool).await?))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let input = input.validate()?;
    let project = sqlx::query_as::<_, Project>("INSERT INTO projects (id, name, description) VALUES ($1, $2, $3) RETURNING id, name, description, created_at")
        .bind(Uuid::new_v4()).bind(input.name).bind(input.description)
        .fetch_one(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(project)))
}
