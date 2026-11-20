use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileOperation {
    operation: String,
    #[serde(default)]
    path: String,
    name: Option<String>,
    hex: Option<String>,
}
fn valid_path(path: &str, root: bool) -> bool {
    (root && path.is_empty())
        || (!path.is_empty()
            && path.len() <= 100
            && path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b))
            && path
                .split('/')
                .all(|part| !part.is_empty() && !part.starts_with('.')))
}
async fn active(state: &AppState, id: &Identity, user: Uuid) -> Result<Hosting, ApiError> {
    let h = access(state, id, user).await?;
    if h.status != "active" {
        return Err(ApiError(StatusCode::CONFLICT, "Hosting must be active."));
    }
    Ok(h)
}
pub(super) async fn files(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
    Json(input): Json<FileOperation>,
) -> Result<Json<Value>, ApiError> {
    let h = active(&state, &id, user).await?;
    if !matches!(
        input.operation.as_str(),
        "list" | "read" | "write" | "mkdir" | "rename" | "delete"
    ) || !valid_path(&input.path, input.operation == "list")
    {
        return Err(invalid(
            "Choose a relative path without hidden files or parent directories.",
        ));
    }
    if input.operation == "rename"
        && !input
            .name
            .as_deref()
            .is_some_and(|n| valid_path(n, false) && !n.contains('/'))
    {
        return Err(invalid("Choose a single file or folder name."));
    }
    if input.operation == "write" {
        let data = input
            .hex
            .as_deref()
            .ok_or_else(|| invalid("File content required."))?;
        if data.len() > 16 * 1024 * 1024 || hex::decode(data).is_err() {
            return Err(invalid("File must be valid data up to 8 MiB."));
        }
    }
    let payload =
        json!({"operation":input.operation,"path":input.path,"name":input.name,"hex":input.hex});
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        runtime::operation(&state, &h, "files", payload),
    )
    .await
    .map_err(|_| unavailable())??;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DatabaseOperation {
    operation: String,
    #[serde(default)]
    sql: String,
}
pub(super) async fn database(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
    Json(input): Json<DatabaseOperation>,
) -> Result<Json<Value>, ApiError> {
    let h = active(&state, &id, user).await?;
    if !matches!(input.operation.as_str(), "import" | "export")
        || input.sql.len() > 8 * 1024 * 1024
        || (input.operation == "import" && input.sql.trim().is_empty())
    {
        return Err(invalid(
            "Choose import or export. Import a non-empty SQL file up to 8 MiB.",
        ));
    }
    crate::auth::accounts::throttle(&state.pool, &format!("hosting-db:{}", id.user_id), 30).await?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(55),
        runtime::operation(
            &state,
            &h,
            "database",
            json!({"operation":input.operation,"sql":input.sql}),
        ),
    )
    .await
    .map_err(|_| unavailable())??;
    Ok(Json(result))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_relative_and_never_hidden() {
        for path in [
            "../x", "/x", "a/../x", "a/.env", "a//b", "a\\b", ".", "a/", "a\0b",
        ] {
            assert!(!valid_path(path, false));
        }
        assert!(valid_path("folder/index.php", false));
        assert!(valid_path("", true));
        assert!(!valid_path("", false));
    }
}
