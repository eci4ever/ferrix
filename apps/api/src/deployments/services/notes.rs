use super::*;

#[derive(Serialize, sqlx::FromRow)]
pub(super) struct Notes {
    notes: String,
    revision: i64,
}
#[derive(Deserialize)]
pub(super) struct SaveNotes {
    notes: String,
    revision: i64,
}
pub(super) async fn read(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Notes>, ApiError> {
    let row = sqlx::query_as::<_, Notes>("SELECT a.notes,a.notes_revision AS revision FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1 AND p.organization_id=$2 AND a.service_kind IN ('application','database')")
        .bind(service).bind(id.organization_id).fetch_optional(&state.pool).await?.ok_or_else(missing)?;
    Ok(Json(row))
}
pub(super) async fn save(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(input): Json<SaveNotes>,
) -> Result<Json<Notes>, ApiError> {
    crate::auth::permissions::developer(&state, &id, service).await?;
    if input.notes.len() > 32768 || input.notes.contains('\0') || input.revision < 0 {
        return Err(invalid(
            "Notes must be at most 32 KiB and cannot contain null characters.",
        ));
    }
    let mut tx = state.pool.begin().await?;
    let revision: i64 = sqlx::query_scalar("SELECT a.notes_revision FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=$1 AND p.organization_id=$2 AND a.service_kind IN ('application','database') FOR UPDATE OF a")
        .bind(service).bind(id.organization_id).fetch_optional(&mut *tx).await?.ok_or_else(missing)?;
    if revision != input.revision {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Notes were changed by someone else. Copy your draft before loading the latest saved notes.",
        ));
    }
    let row = sqlx::query_as::<_, Notes>("UPDATE applications SET notes=$2,notes_revision=notes_revision+1 WHERE id=$1 RETURNING notes,notes_revision AS revision")
        .bind(service).bind(input.notes).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(row))
}
