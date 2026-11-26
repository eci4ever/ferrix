//! Resource authorization: grants are capabilities, never organization roles.
use super::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

fn hidden() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "Resource not found in this organization.",
    )
}
pub(crate) fn manager(id: &Identity) -> bool {
    matches!(id.role.as_str(), "owner" | "admin")
}
pub(crate) async fn has_developer(
    state: &AppState,
    id: &Identity,
    project: Option<Uuid>,
) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id WHERE a.service_kind='application' AND ($3::uuid IS NULL OR e.project_id=$3) AND ferrix_service_permission($1,$2,a.id)='developer')")
        .bind(id.user_id).bind(id.organization_id).bind(project).fetch_one(&state.pool).await?)
}
pub(crate) async fn service_role(
    state: &AppState,
    id: &Identity,
    service: Uuid,
) -> Result<Option<String>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT ferrix_service_permission($1,$2,$3)")
            .bind(id.user_id)
            .bind(id.organization_id)
            .bind(service)
            .fetch_one(&state.pool)
            .await?,
    )
}
pub(crate) async fn read_service(
    state: &AppState,
    id: &Identity,
    service: Uuid,
) -> Result<(), ApiError> {
    service_role(state, id, service).await?.ok_or_else(hidden)?;
    Ok(())
}
pub(crate) async fn developer(
    state: &AppState,
    id: &Identity,
    service: Uuid,
) -> Result<(), ApiError> {
    let role = service_role(state, id, service).await?.ok_or_else(hidden)?;
    if role == "manager" {
        return Ok(());
    }
    let application: bool =
        sqlx::query_scalar("SELECT service_kind='application' FROM applications WHERE id=$1")
            .bind(service)
            .fetch_one(&state.pool)
            .await?;
    if role == "developer" && application {
        Ok(())
    } else {
        Err(forbidden())
    }
}
pub(crate) async fn project(
    state: &AppState,
    id: &Identity,
    project: Uuid,
) -> Result<(), ApiError> {
    let allowed: bool = sqlx::query_scalar("SELECT ferrix_project_visible($1,$2,$3)")
        .bind(id.user_id)
        .bind(id.organization_id)
        .bind(project)
        .fetch_one(&state.pool)
        .await?;
    if allowed { Ok(()) } else { Err(hidden()) }
}
pub(crate) async fn environment(
    state: &AppState,
    id: &Identity,
    environment: Uuid,
) -> Result<(), ApiError> {
    let allowed: bool = sqlx::query_scalar("SELECT ferrix_environment_visible($1,$2,$3)")
        .bind(id.user_id)
        .bind(id.organization_id)
        .bind(environment)
        .fetch_one(&state.pool)
        .await?;
    if allowed { Ok(()) } else { Err(hidden()) }
}

/// Called after cookie/key authentication, before body extraction or side effects.
/// Member resource routes are allowlisted; new operations require a policy choice.
pub(crate) async fn request_access(
    state: &AppState,
    id: &Identity,
    method: &str,
    path: &str,
) -> Result<(), ApiError> {
    if manager(id) {
        return Ok(());
    }
    if method == "GET" {
        let restricted: bool = sqlx::query_scalar(
            "SELECT restrict_member_resources FROM auth_organizations WHERE id=$1",
        )
        .bind(id.organization_id)
        .fetch_one(&state.pool)
        .await?;
        // Preserve pre-existing read-only endpoints until classroom restrictions
        // are explicitly enabled. Secret/terminal handlers still require developer.
        if !restricted {
            return Ok(());
        }
    }
    let path = path
        .strip_prefix("/api/v1/")
        .unwrap_or(path)
        .trim_start_matches('/');
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() < 2 {
        return Ok(());
    }
    match parts[0] {
        "organizations" if parts[1] == "quota" => Err(forbidden()),
        "services" | "applications" => {
            let service = parts[1].parse().map_err(|_| hidden())?;
            read_service(state, id, service).await?;
            let suffix = parts[2..].join("/");
            if method == "GET"
                && matches!(
                    suffix.as_str(),
                    "" | "runtime"
                        | "logs"
                        | "deployments"
                        | "notes"
                        | "metrics"
                        | "history"
                        | "logs/history"
                        | "access"
                )
            {
                return Ok(());
            }
            if (method == "GET"
                && matches!(
                    suffix.as_str(),
                    "configuration" | "environment" | "source" | "terminal" | "database-connection"
                ))
                || (method == "POST"
                    && matches!(
                        suffix.as_str(),
                        "configuration"
                            | "environment"
                            | "deploy"
                            | "control"
                            | "notes"
                            | "source"
                            | "source/test"
                            | "source/upload"
                    ))
            {
                return developer(state, id, service).await;
            }
            Err(forbidden())
        }
        "projects" if parts[1] != "service-status" => {
            let resource = parts[1].parse().map_err(|_| hidden())?;
            project(state, id, resource).await?;
            if method == "GET" && matches!(parts.get(2).copied(), Some("environments" | "access")) {
                Ok(())
            } else {
                Err(forbidden())
            }
        }
        "environments" => {
            environment(state, id, parts[1].parse().map_err(|_| hidden())?).await?;
            if method == "GET"
                && matches!(
                    parts.get(2).copied(),
                    Some("services" | "applications" | "service-status")
                )
            {
                Ok(())
            } else {
                Err(forbidden())
            }
        }
        _ => Ok(()),
    }
}

pub(crate) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/projects/{id}/assignments",
            get(project_grants).post(project_grant),
        )
        .route(
            "/services/{id}/assignments",
            get(service_grants).post(service_grant),
        )
        .route("/services/{id}/access", get(effective))
        .route(
            "/auth/organizations/access",
            get(settings).post(save_settings),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            sessions::protect,
        ))
}
async fn effective(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let role = service_role(&state, &id, service)
        .await?
        .ok_or_else(hidden)?;
    let can_develop = developer(&state, &id, service).await.is_ok();
    Ok(Json(
        json!({"permission":role,"can_develop":can_develop,"can_admin":manager(&id)}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Restriction {
    restricted: bool,
}
async fn settings(State(state): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    let restricted: bool =
        sqlx::query_scalar("SELECT restrict_member_resources FROM auth_organizations WHERE id=$1")
            .bind(id.organization_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(json!({"restricted":restricted})))
}
async fn save_settings(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Restriction>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "manager").await?;
    sqlx::query("UPDATE auth_organizations SET restrict_member_resources=$2 WHERE id=$1")
        .bind(id.organization_id)
        .bind(input.restricted)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        if input.restricted {
            "resource_access.restricted"
        } else {
            "resource_access.legacy"
        },
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    user_id: Uuid,
    permission: Option<String>,
}
async fn grants(
    state: AppState,
    id: Identity,
    resource: Uuid,
    service: bool,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    if service {
        read_service(&state, &id, resource).await?
    } else {
        project(&state, &id, resource).await?
    }
    let (table, column) = if service {
        ("service_assignments", "service_id")
    } else {
        ("project_assignments", "project_id")
    };
    let rows:Vec<(Uuid,String)>=sqlx::query_as(&format!("SELECT user_id,permission FROM {table} WHERE {column}=$1 AND organization_id=$2 ORDER BY user_id")).bind(resource).bind(id.organization_id).fetch_all(&state.pool).await?;
    Ok(Json(
        json!({"assignments":rows.into_iter().map(|(user_id,permission)|json!({"user_id":user_id,"permission":permission})).collect::<Vec<_>>()}),
    ))
}
async fn grant(
    state: AppState,
    id: Identity,
    resource: Uuid,
    service: bool,
    input: Grant,
) -> Result<StatusCode, ApiError> {
    if input
        .permission
        .as_deref()
        .is_some_and(|v| !matches!(v, "viewer" | "developer"))
    {
        return Err(bad(
            "Choose viewer or developer; use null to revoke an assignment.",
        ));
    }
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "manager").await?;
    let exists: bool = if service {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1 AND p.organization_id=$2)").bind(resource).bind(id.organization_id).fetch_one(&mut *tx).await?
    } else {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1 AND organization_id=$2)",
        )
        .bind(resource)
        .bind(id.organization_id)
        .fetch_one(&mut *tx)
        .await?
    };
    if !exists {
        return Err(hidden());
    }
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM auth_members WHERE organization_id=$1 AND user_id=$2)",
    )
    .bind(id.organization_id)
    .bind(input.user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !member {
        return Err(bad("Assign only members of this organization."));
    }
    let (table, column) = if service {
        ("service_assignments", "service_id")
    } else {
        ("project_assignments", "project_id")
    };
    if let Some(permission) = input.permission.as_deref() {
        sqlx::query(&format!("INSERT INTO {table}(organization_id,{column},user_id,permission) VALUES($1,$2,$3,$4) ON CONFLICT({column},user_id) DO UPDATE SET permission=EXCLUDED.permission")).bind(id.organization_id).bind(resource).bind(input.user_id).bind(permission).execute(&mut *tx).await?;
    } else {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE {column}=$1 AND user_id=$2 AND organization_id=$3"
        ))
        .bind(resource)
        .bind(input.user_id)
        .bind(id.organization_id)
        .execute(&mut *tx)
        .await?;
    }
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        &format!(
            "assignment.{}:{}:{}:{}",
            if service { "service" } else { "project" },
            resource,
            input.user_id,
            input.permission.as_deref().unwrap_or("revoked")
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn project_grants(
    State(s): State<AppState>,
    id: Identity,
    Path(r): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    grants(s, id, r, false).await
}
async fn service_grants(
    State(s): State<AppState>,
    id: Identity,
    Path(r): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    grants(s, id, r, true).await
}
async fn project_grant(
    State(s): State<AppState>,
    id: Identity,
    Path(r): Path<Uuid>,
    Json(g): Json<Grant>,
) -> Result<StatusCode, ApiError> {
    grant(s, id, r, false, g).await
}
async fn service_grant(
    State(s): State<AppState>,
    id: Identity,
    Path(r): Path<Uuid>,
    Json(g): Json<Grant>,
) -> Result<StatusCode, ApiError> {
    grant(s, id, r, true, g).await
}
