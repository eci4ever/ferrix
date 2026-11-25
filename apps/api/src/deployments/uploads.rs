//! Bounded project-private source revisions; runtime secrets never enter build contexts.
use super::*;
use axum::body::Bytes;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{Cursor, Read, Write},
    path::{Component, Path as FsPath},
};
const MAX_UPLOAD: usize = 32 * 1024 * 1024;
const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    context: String,
    dockerfile: String,
}
#[derive(Serialize, Deserialize)]
struct Preview {
    files: Vec<String>,
    excluded: Vec<String>,
    file_count: usize,
    expanded_bytes: u64,
    truncated: bool,
}
struct Prepared {
    archive: tempfile::NamedTempFile,
    preview: Preview,
}
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/services/{id}/source/upload",
            post(upload)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD))
                .layer(axum::middleware::from_fn(heavy_guard)),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}
async fn upload(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
    Query(options): Query<Options>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    crate::auth::permissions::developer(&state, &id, app).await?;
    if app_access(&state, &id, app).await?.kind != "application" {
        return Err(missing());
    }
    crate::auth::accounts::throttle(&state.pool, &format!("source-upload:{}", id.user_id), 10)
        .await?;
    if body.is_empty() || body.len() > MAX_UPLOAD {
        return Err(invalid("Upload a ZIP, tar or tar.gz archive up to 32 MiB."));
    }
    let context = github::path(&options.context, true)?;
    let dockerfile = github::path(&options.dockerfile, false)?;
    let bytes = body.to_vec();
    let c = context.clone();
    let d = dockerfile.clone();
    let prepared=tokio::task::spawn_blocking(move||prepare(&bytes,&c,&d)).await.map_err(|_|internal())?.map_err(|_|invalid("Invalid source archive. Check Dockerfile/context paths, limits and unsupported links or special files."))?;
    let revision = Uuid::new_v4();
    let sha = hex::encode(Sha256::digest(&body));
    let preview = serde_json::to_value(&prepared.preview).map_err(|_| internal())?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO application_source_uploads(id,application_id,created_by,archive,sha256,context,dockerfile,preview) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(revision).bind(app).bind(id.user_id).bind(body.as_ref()).bind(&sha).bind(&context).bind(&dockerfile).bind(&preview).execute(&mut *tx).await?;
    // Keep five recent revisions plus the configured revision and all pending build snapshots.
    // Completed deployments roll back using their immutable image, not their source archive.
    sqlx::query("DELETE FROM application_source_uploads u WHERE application_id=$1 AND id NOT IN (SELECT id FROM application_source_uploads WHERE application_id=$1 ORDER BY created_at DESC,id DESC LIMIT 5) AND NOT EXISTS(SELECT 1 FROM applications a WHERE a.source_upload_id=u.id) AND NOT EXISTS(SELECT 1 FROM deployments d WHERE d.source_upload_id=u.id AND d.status IN ('queued','running'))").bind(app).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"upload_id":revision,"sha256":sha,"context":context,"dockerfile":dockerfile,"preview":preview}),
    ))
}
pub(super) async fn current(state: &AppState, app: Uuid) -> Result<Option<Value>, ApiError> {
    let row:Option<(Uuid,String,String,String,Value)>=sqlx::query_as("SELECT u.id,u.sha256,u.context,u.dockerfile,u.preview FROM applications a JOIN application_source_uploads u ON u.id=a.source_upload_id WHERE a.id=$1").bind(app).fetch_optional(&state.pool).await?;
    Ok(row.map(|(id,sha,c,d,preview)|json!({"mode":"upload","upload_id":id,"sha256":sha,"context":c,"dockerfile":d,"preview":preview,"repository":"","branch":"main","auto_deploy":false,"token_saved":false,"webhook_configured":false,"webhook_url":"","pending":false,"last_received_at":null,"last_message":null})))
}
fn safe_path(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 1024
        || name.contains('\\')
        || name.chars().any(char::is_control)
        || name.split('/').any(|p| p == "..")
        || FsPath::new(name).is_absolute()
        || FsPath::new(name)
            .components()
            .any(|p| !matches!(p, Component::Normal(_) | Component::CurDir))
        || name.split('/').count() > 64
    {
        return Err("Unsafe archive path.".into());
    }
    Ok(())
}
#[derive(Default)]
struct Budget {
    seen: HashSet<String>,
    total: u64,
}
fn write_entry(
    root: &FsPath,
    name: &str,
    dir: bool,
    mode: u32,
    size: u64,
    reader: &mut dyn Read,
    budget: &mut Budget,
) -> Result<(), String> {
    safe_path(name)?;
    let normalized = FsPath::new(name)
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect::<std::path::PathBuf>();
    if normalized.as_os_str().is_empty() {
        if dir {
            return Ok(());
        }
        return Err("Invalid empty file path.".into());
    }
    let key = normalized.to_string_lossy().into_owned();
    if !budget.seen.insert(key) || budget.seen.len() > 20_000 {
        return Err("Duplicate path or too many files.".into());
    }
    budget.total = budget.total.checked_add(size).ok_or("Size overflow.")?;
    if budget.total > MAX_EXPANDED {
        return Err("Expanded source exceeds 128 MiB.".into());
    }
    let target = root.join(normalized);
    if dir {
        std::fs::create_dir_all(target).map_err(|_| "Invalid directory.")?;
    } else {
        std::fs::create_dir_all(target.parent().ok_or("Invalid parent.")?)
            .map_err(|_| "Invalid parent directory.")?;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)
            .map_err(|_| "Conflicting file path.")?;
        let copied = std::io::copy(&mut reader.take(size + 1), &mut file)
            .map_err(|_| "Invalid archive data.")?;
        if copied != size {
            return Err("Incorrect archive entry size.".into());
        }
        file.flush().map_err(|_| "Could not flush source.")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                target,
                std::fs::Permissions::from_mode(if mode & 0o111 != 0 { 0o755 } else { 0o644 }),
            )
            .map_err(|_| "Could not protect source.")?;
        }
    }
    Ok(())
}
fn prepare(bytes: &[u8], context: &str, dockerfile: &str) -> Result<Prepared, String> {
    let root = tempfile::tempdir().map_err(|_| "Could not allocate source workspace.")?;
    let mut budget = Budget::default();
    if bytes.starts_with(b"PK") {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Invalid ZIP archive.")?;
        if archive.len() > 20_000 {
            return Err("Too many ZIP entries.".into());
        }
        for n in 0..archive.len() {
            let mut entry = archive
                .by_index(n)
                .map_err(|_| "Invalid or encrypted ZIP entry.")?;
            let name = entry.name().to_owned();
            let dir = entry.is_dir();
            let mode = entry
                .unix_mode()
                .unwrap_or(if dir { 0o40700 } else { 0o100600 });
            if !matches!(mode & 0o170000, 0 | 0o040000 | 0o100000) {
                return Err("Links and special files are unsupported.".into());
            }
            let size = entry.size();
            write_entry(root.path(), &name, dir, mode, size, &mut entry, &mut budget)?;
        }
    } else {
        let reader: Box<dyn Read + '_> = if bytes.starts_with(&[0x1f, 0x8b]) {
            Box::new(flate2::read::GzDecoder::new(bytes).take(MAX_EXPANDED + 4 * 1024 * 1024))
        } else {
            Box::new(bytes)
        };
        let mut archive = tar::Archive::new(reader);
        let mut entries = 0;
        for entry in archive.entries().map_err(|_| "Invalid tar archive.")? {
            entries += 1;
            if entries > 20_000 {
                return Err("Too many tar entries.".into());
            }
            let mut entry = entry.map_err(|_| "Invalid tar entry.")?;
            let kind = entry.header().entry_type();
            if !kind.is_file() && !kind.is_dir() {
                return Err("Only regular files and directories are accepted.".into());
            }
            let name = entry
                .path()
                .map_err(|_| "Invalid tar path.")?
                .to_str()
                .ok_or("Source names must be UTF-8.")?
                .to_owned();
            let mode = entry.header().mode().map_err(|_| "Invalid file mode.")?;
            let size = entry.size();
            write_entry(
                root.path(),
                &name,
                kind.is_dir(),
                mode,
                size,
                &mut entry,
                &mut budget,
            )?;
        }
    }
    let context = root.path().join(context);
    if !context.is_dir() || !context.join(dockerfile).is_file() {
        return Err("Dockerfile/context not found. Paths are relative to the archive root.".into());
    }
    let output = tempfile::NamedTempFile::new().map_err(|_| "Could not allocate build context.")?;
    let mut builder = tar::Builder::new(output.reopen().map_err(|_| "Could not open context.")?);
    github::build::filtered_context(&mut builder, &context, dockerfile)?;
    builder.finish().map_err(|_| "Could not finish context.")?;
    let mut included = HashSet::new();
    for entry in tar::Archive::new(output.reopen().map_err(|_| "Could not read context.")?)
        .entries()
        .map_err(|_| "Invalid prepared context.")?
    {
        let entry = entry.map_err(|_| "Invalid prepared entry.")?;
        included.insert(
            entry
                .path()
                .map_err(|_| "Invalid prepared path.")?
                .to_string_lossy()
                .into_owned(),
        );
    }
    let prefix = if context == root.path() {
        String::new()
    } else {
        context
            .strip_prefix(root.path())
            .map_err(|_| "Invalid context.")?
            .to_string_lossy()
            .to_string()
            + "/"
    };
    let mut files: Vec<_> = included.iter().cloned().collect();
    files.sort();
    let mut excluded: Vec<_> = budget
        .seen
        .into_iter()
        .filter(|p| {
            !p.strip_prefix(&prefix)
                .is_some_and(|p| included.contains(p))
        })
        .collect();
    excluded.sort();
    let count = files.len();
    let truncated = files.len() > 200 || excluded.len() > 200;
    files.truncate(200);
    excluded.truncate(200);
    Ok(Prepared {
        archive: output,
        preview: Preview {
            files,
            excluded,
            file_count: count,
            expanded_bytes: budget.total,
            truncated,
        },
    })
}
pub(super) async fn build_image(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    job: &worker::Job,
) -> Result<bollard::models::ImageInspect, String> {
    let (bytes,sha,context,dockerfile):(Vec<u8>,String,String,String)=sqlx::query_as("SELECT archive,sha256,context,dockerfile FROM application_source_uploads WHERE id=$1 AND application_id=$2").bind(job.source_upload_id).bind(job.application_id).fetch_optional(&mut *conn).await.map_err(|_|"Could not load uploaded source.")?.ok_or("Uploaded source revision is unavailable.")?;
    if hex::encode(Sha256::digest(&bytes)) != sha {
        return Err("Source archive checksum mismatch.".into());
    }
    let c = context.clone();
    let d = dockerfile.clone();
    let prepared = tokio::task::spawn_blocking(move || prepare(&bytes, &c, &d))
        .await
        .map_err(|_| "Source preparation interrupted.")??;
    worker::phase(
        conn,
        job.id,
        &format!("Building uploaded source SHA-256 {sha}…"),
    )
    .await?;
    let source = github::Source {
        repository: String::new(),
        branch: String::new(),
        context,
        dockerfile,
        token: None,
    };
    github::build::build_prepared(state, conn, job, &source, &sha, prepared.archive).await
}
// Acquire before Bytes extraction so bounded archives cannot multiply unbounded memory use.
pub(super) async fn heavy_guard(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let Some(id) = req.extensions().get::<Identity>() else {
        return ApiError(StatusCode::UNAUTHORIZED, "Sign in to continue.").into_response();
    };
    let _ = id; // Resource permission was checked by sessions::protect before this guard.
    let Ok(_slot) = SLOTS.try_acquire() else {
        return ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Archive preparation is busy. Retry shortly.",
        )
        .into_response();
    };
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn tar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap()
    }
    #[test]
    fn preview_filters_and_builds_only_selected_context() {
        let bytes = tar_bytes(&[
            ("web/Dockerfile", b"FROM scratch"),
            ("web/index.html", b"ok"),
            ("web/.env", b"private"),
            ("web/.dockerignore", b".env"),
            ("database.sql", b"private"),
        ]);
        let p = prepare(&bytes, "web", "Dockerfile").unwrap();
        assert!(p.preview.files.contains(&"index.html".into()));
        assert!(!p.preview.files.contains(&".env".into()));
        assert!(p.preview.excluded.contains(&"database.sql".into()));
    }
    #[test]
    fn rejects_unsafe_paths_and_duplicate_entries() {
        for s in ["../outside", "/root", "a/../../b", "a\\b", "x\0y"] {
            assert!(safe_path(s).is_err());
        }
        assert!(
            prepare(
                &tar_bytes(&[("Dockerfile", b"a"), ("Dockerfile", b"b")]),
                ".",
                "Dockerfile"
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_links() {
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_mode(0o777);
        h.set_link_name("/etc/passwd").unwrap();
        h.set_cksum();
        b.append_data(&mut h, "Dockerfile", std::io::empty())
            .unwrap();
        assert!(prepare(&b.into_inner().unwrap(), ".", "Dockerfile").is_err());
    }
}

#[cfg(test)]
mod integration_tests;
