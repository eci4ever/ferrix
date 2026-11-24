//! Explicit, browser-admin-only updates for the standard single-host Compose installation.
use crate::{ApiError, AppState, auth::Identity, releases::ReleaseStatus};
use axum::{
    Extension, Json,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use bollard::{
    models::{ContainerCreateBody, HostConfig, HostConfigLogConfig},
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, LogsOptionsBuilder,
    },
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::{RwLock, RwLockReadGuard, watch};

// Pinned official amd64 Docker CLI image, including the Compose plugin.
const HELPER: &str = "docker.io/library/docker@sha256:a3451ad65d1aae1fa7bcba62e08037296014bc500def2bc2075c59416af3cc4c";
const SCRIPT: &str = include_str!("self_update/update.sh");
static BUSY: AtomicBool = AtomicBool::new(false);
static ACTIVITY: RwLock<()> = RwLock::const_new(());

fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "UI updates require a healthy managed production installation. Use the upgrade guide.",
    )
}
fn conflict() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "An update or service operation is active. Wait for it to finish before trying again.",
    )
}
fn enabled() -> bool {
    std::env::var("FERRIX_SELF_UPDATE").as_deref() == Ok("true")
}

/// Shared by lifecycle requests and the deployment/backup worker; updates drain them first.
pub(crate) async fn activity() -> Option<RwLockReadGuard<'static, ()>> {
    if BUSY.load(Ordering::SeqCst) {
        return None;
    }
    let guard = ACTIVITY.read().await;
    if BUSY.load(Ordering::SeqCst) {
        None
    } else {
        Some(guard)
    }
}

pub(crate) async fn guard(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let mutation = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) || path.ends_with("/terminal");
    if !mutation || path == "/api/v1/releases/update" {
        return next.run(req).await;
    }
    if BUSY.load(Ordering::SeqCst) {
        return conflict().into_response();
    }
    let Ok(_activity) = ACTIVITY.try_read() else {
        return conflict().into_response();
    };
    if BUSY.load(Ordering::SeqCst) {
        return conflict().into_response();
    }
    next.run(req).await
}

struct Installation {
    project: String,
    container: String,
    old_ref: String,
}
impl Installation {
    fn helper(&self) -> String {
        format!("{}-updater", self.project)
    }
}

async fn installation(state: &AppState) -> Result<Installation, ApiError> {
    if !enabled() {
        return Err(unavailable());
    }
    let hostname = std::fs::read_to_string("/etc/hostname").map_err(|_| unavailable())?;
    let hostname = hostname.trim();
    if !(12..=64).contains(&hostname.len()) || !hostname.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(unavailable());
    }
    let info = state
        .docker
        .inspect_container(hostname, None)
        .await
        .map_err(|_| unavailable())?;
    let labels = info
        .config
        .as_ref()
        .and_then(|c| c.labels.as_ref())
        .ok_or_else(unavailable)?;
    if labels.get("io.ferrix.self-update").map(String::as_str) != Some("true")
        || labels.get("com.docker.compose.service").map(String::as_str) != Some("ferrix")
        || labels
            .get("com.docker.compose.project.working_dir")
            .map(String::as_str)
            != Some("/opt/ferrix")
        || labels
            .get("com.docker.compose.project.config_files")
            .map(String::as_str)
            != Some("/opt/ferrix/compose.yaml")
    {
        return Err(unavailable());
    }
    let project = labels
        .get("com.docker.compose.project")
        .cloned()
        .ok_or_else(unavailable)?;
    if project.is_empty()
        || project.len() > 63
        || !project
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(unavailable());
    }
    let image = state
        .docker
        .inspect_image(info.image.as_deref().ok_or_else(unavailable)?)
        .await
        .map_err(|_| unavailable())?;
    if image.architecture.as_deref() != Some("amd64") {
        return Err(unavailable());
    }
    let old_ref = image
        .repo_digests
        .unwrap_or_default()
        .into_iter()
        .find(|s| official_digest(s))
        .ok_or_else(unavailable)?;
    Ok(Installation {
        project,
        container: info.id.ok_or_else(unavailable)?,
        old_ref,
    })
}
fn official_digest(value: &str) -> bool {
    value
        .strip_prefix("ghcr.io/eci4ever/ferrix@sha256:")
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
}

#[derive(Default, Serialize)]
pub(crate) struct UpdateStatus {
    supported: bool,
    active: bool,
    stage: String,
    target_version: Option<String>,
    backup_path: Option<String>,
}
async fn inspect(
    state: &AppState,
    install: &Installation,
) -> Result<Option<UpdateStatus>, ApiError> {
    let info = match state
        .docker
        .inspect_container(&install.helper(), None)
        .await
    {
        Ok(info) => info,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => return Ok(None),
        Err(_) => return Err(unavailable()),
    };
    let labels = info
        .config
        .as_ref()
        .and_then(|c| c.labels.as_ref())
        .ok_or_else(unavailable)?;
    if labels.get("io.ferrix.updater").map(String::as_str) != Some(install.project.as_str()) {
        return Err(unavailable());
    }
    let runtime = info.state.ok_or_else(unavailable)?;
    // A helper left unstarted by a cancelled request is safe to discard on retry.
    let active = runtime.running == Some(true);
    let mut stage = if active { "preparing" } else { "failed" }.to_owned();
    let mut logs = state.docker.logs(
        &install.helper(),
        Some(
            LogsOptionsBuilder::default()
                .stdout(true)
                .tail("32")
                .build(),
        ),
    );
    let mut size = 0;
    while let Some(line) = logs.next().await {
        let bytes = line.map_err(|_| unavailable())?.into_bytes();
        size += bytes.len();
        if size > 8192 {
            break;
        }
        for line in String::from_utf8_lossy(&bytes).lines() {
            if let Some(value) = line.strip_prefix("FERRIX_UPDATE_STAGE=")
                && matches!(
                    value,
                    "preparing"
                        | "pulling"
                        | "backing_up"
                        | "updating"
                        | "verifying"
                        | "succeeded"
                        | "rolling_back"
                        | "rolled_back"
                        | "failed"
                )
            {
                stage = value.to_owned();
            }
        }
    }
    if !active && (runtime.exit_code != Some(0) || stage != "succeeded") && stage != "rolled_back" {
        stage = "failed".into();
    }
    let job = labels
        .get("io.ferrix.update-id")
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    Ok(Some(UpdateStatus {
        supported: true,
        active,
        stage,
        target_version: labels.get("io.ferrix.target-version").cloned(),
        backup_path: job.map(|id| format!("/opt/ferrix/updates/{id}")),
    }))
}

pub(crate) async fn status(
    State(state): State<AppState>,
    identity: Identity,
) -> Result<Json<UpdateStatus>, ApiError> {
    identity.admin()?;
    if !enabled() {
        return Ok(Json(UpdateStatus::default()));
    }
    let install = tokio::time::timeout(Duration::from_secs(5), installation(&state))
        .await
        .map_err(|_| unavailable())??;
    let value = tokio::time::timeout(Duration::from_secs(5), inspect(&state, &install))
        .await
        .map_err(|_| unavailable())??;
    Ok(Json(value.unwrap_or(UpdateStatus {
        supported: true,
        active: BUSY.load(Ordering::SeqCst),
        stage: "idle".into(),
        ..Default::default()
    })))
}

#[derive(Deserialize)]
pub(crate) struct Start {
    version: String,
    confirmation: String,
    backup_acknowledged: bool,
    password: String,
    #[serde(default)]
    code: String,
}
struct Reset(bool);
impl Drop for Reset {
    fn drop(&mut self) {
        if self.0 {
            BUSY.store(false, Ordering::SeqCst);
        }
    }
}

pub(crate) async fn start(
    State(state): State<AppState>,
    Extension(cache): Extension<watch::Receiver<ReleaseStatus>>,
    key: Option<Extension<crate::auth::api_keys::KeyAccess>>,
    identity: Identity,
    Json(input): Json<Start>,
) -> Result<(StatusCode, Json<UpdateStatus>), ApiError> {
    identity.admin()?;
    if key.is_some() {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "UI updates require an administrator browser session.",
        ));
    }
    if input.confirmation != "UPDATE FERRIX" || !input.backup_acknowledged {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Confirm the backup policy and type UPDATE FERRIX.",
        ));
    }
    {
        let release = cache.borrow();
        if !release.update_available
            || release.check_failed
            || release.latest_version.as_deref() != Some(input.version.as_str())
            || release
                .checked_at
                .is_none_or(|time| time < chrono::Utc::now() - chrono::Duration::hours(7))
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "A freshly confirmed newer stable release is required. Check the release notice again later.",
            ));
        }
    }
    let install = tokio::time::timeout(Duration::from_secs(5), installation(&state))
        .await
        .map_err(|_| unavailable())??;
    let _exclusive = tokio::time::timeout(Duration::from_secs(2), ACTIVITY.write())
        .await
        .map_err(|_| conflict())?;
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err(conflict());
    }
    let mut reset = Reset(true);
    // Revalidate the browser principal after draining competing account mutations.
    let authorized: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_sessions s JOIN auth_users u ON u.id=s.user_id WHERE s.id=$1 AND u.id=$2 AND u.system_role='admin' AND NOT auth_user_banned(u.banned,u.ban_expires_at) AND s.expires_at>now() AND s.absolute_expires_at>now())")
        .bind(identity.session_id).bind(identity.user_id).fetch_one(&state.pool).await?;
    if !authorized {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "An active platform-administrator session is required.",
        ));
    }
    crate::auth::accounts::check(
        &state,
        &identity,
        crate::auth::accounts::Reauthentication {
            password: input.password,
            code: input.code,
        },
    )
    .await?;

    if let Some(previous) = inspect(&state, &install).await? {
        if previous.active {
            reset.0 = false;
            return Err(conflict());
        }
        state
            .docker
            .remove_container(&install.helper(), None)
            .await
            .map_err(|_| unavailable())?;
    }
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deployments WHERE status IN ('queued','running') OR restore_container_id IS NOT NULL) OR EXISTS(SELECT 1 FROM database_jobs WHERE status IN ('queued','running')) OR EXISTS(SELECT 1 FROM project_maintenance) OR EXISTS(SELECT 1 FROM project_bundle_jobs WHERE status IN ('queued','running','resuming','cutover_queued','cutting_over')) OR EXISTS(SELECT 1 FROM terminal_connections WHERE closed_at IS NULL AND expires_at>now())").fetch_one(&state.pool).await?;
    if pending {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Finish pending deployments, backups, recovery jobs and terminal sessions before updating Ferrix.",
        ));
    }
    tokio::time::timeout(Duration::from_secs(120), async {
        if state.docker.inspect_image(HELPER).await.is_err() {
            let mut pull = state.docker.create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(HELPER)
                        .build(),
                ),
                None,
                None,
            );
            while let Some(item) = pull.next().await {
                if item.map_err(|_| unavailable())?.error_detail.is_some() {
                    return Err(unavailable());
                }
            }
        }
        Ok::<_, ApiError>(())
    })
    .await
    .map_err(|_| unavailable())??;
    let job = uuid::Uuid::new_v4();
    let helper = ContainerCreateBody {
        image: Some(HELPER.into()),
        user: Some("0:0".into()),
        entrypoint: Some(vec![
            "timeout".into(),
            "-s".into(),
            "TERM".into(),
            "1800".into(),
            "/bin/sh".into(),
            "-c".into(),
        ]),
        cmd: Some(vec![
            SCRIPT.into(),
            "ferrix-update".into(),
            input.version.clone(),
            install.project.clone(),
            install.container.clone(),
            install.old_ref.clone(),
            job.to_string(),
        ]),
        env: Some(vec![
            "DOCKER_HOST=unix:///var/run/docker.sock".into(),
            "DOCKER_CONFIG=/tmp/docker".into(),
            "HOME=/tmp".into(),
        ]),
        labels: Some(HashMap::from([
            ("io.ferrix.updater".into(), install.project.clone()),
            ("io.ferrix.update-id".into(), job.to_string()),
            ("io.ferrix.target-version".into(), input.version.clone()),
            (
                "io.ferrix.requested-by".into(),
                identity.user_id.to_string(),
            ),
        ])),
        host_config: Some(HostConfig {
            binds: Some(vec![
                "/var/run/docker.sock:/var/run/docker.sock".into(),
                "/opt/ferrix:/opt/ferrix".into(),
            ]),
            network_mode: Some("none".into()),
            readonly_rootfs: Some(true),
            tmpfs: Some(HashMap::from([(
                "/tmp".into(),
                "rw,noexec,nosuid,size=32m".into(),
            )])),
            memory: Some(256 * 1024 * 1024),
            nano_cpus: Some(500_000_000),
            pids_limit: Some(128),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".into()),
                config: Some(HashMap::from([
                    ("max-size".into(), "64k".into()),
                    ("max-file".into(), "1".into()),
                ])),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let name = install.helper();
    state
        .docker
        .create_container(
            Some(CreateContainerOptionsBuilder::default().name(&name).build()),
            helper,
        )
        .await
        .map_err(|_| conflict())?;
    if !matches!(
        tokio::time::timeout(
            Duration::from_secs(10),
            state.docker.start_container(&name, None)
        )
        .await,
        Ok(Ok(()))
    ) {
        // A lost Docker response does not prove the helper failed to start.
        // Keep maintenance active whenever its runtime state is unknown.
        match tokio::time::timeout(
            Duration::from_secs(5),
            state.docker.inspect_container(&name, None),
        )
        .await
        {
            Ok(Ok(info))
                if info
                    .state
                    .as_ref()
                    .is_some_and(|runtime| runtime.running == Some(true)) => {}
            Ok(Ok(_)) => {
                let _ = state.docker.remove_container(&name, None).await;
                return Err(unavailable());
            }
            _ => {
                reset.0 = false;
                return Err(unavailable());
            }
        }
    }
    reset.0 = false;
    Ok((
        StatusCode::ACCEPTED,
        Json(UpdateStatus {
            supported: true,
            active: true,
            stage: "preparing".into(),
            target_version: Some(input.version),
            backup_path: Some(format!("/opt/ferrix/updates/{job}")),
        }),
    ))
}

/// Reconcile the helper after a Ferrix restart before accepting lifecycle work.
pub async fn initialize(state: &AppState) {
    if !enabled() {
        return;
    }
    BUSY.store(true, Ordering::SeqCst);
    match tokio::time::timeout(Duration::from_secs(5), installation(state)).await {
        Ok(Ok(install)) => {
            if let Ok(Ok(value)) =
                tokio::time::timeout(Duration::from_secs(5), inspect(state, &install)).await
            {
                BUSY.store(value.is_some_and(|s| s.active), Ordering::SeqCst);
            }
        }
        _ => {
            tracing::warn!("Managed UI updates are unavailable for this installation");
            BUSY.store(false, Ordering::SeqCst);
        }
    }
}
pub async fn run_worker(state: AppState, mut shutdown: watch::Receiver<bool>) {
    if !enabled() {
        return;
    }
    loop {
        tokio::select! {_=shutdown.changed()=>return,_=tokio::time::sleep(Duration::from_secs(3))=>{}}
        if !BUSY.load(Ordering::SeqCst) {
            continue;
        }
        let Ok(_activity) = ACTIVITY.try_read() else {
            continue;
        };
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let install = installation(&state).await?;
            inspect(&state, &install).await
        })
        .await;
        if let Ok(Ok(Some(value))) = result {
            BUSY.store(value.active, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(admin: bool) -> Identity {
        Identity {
            session_id: uuid::Uuid::nil(),
            user_id: uuid::Uuid::nil(),
            name: "Test".into(),
            email: "test@example.com".into(),
            system_role: if admin { "admin" } else { "user" }.into(),
            organization_id: uuid::Uuid::nil(),
            organization_name: "Test".into(),
            role: "owner".into(),
            authenticated_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now(),
        }
    }
    fn state() -> AppState {
        AppState {
            gateway: Default::default(),
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
                .unwrap(),
            docker: bollard::Docker::connect_with_local_defaults().unwrap(),
            auth: std::sync::Arc::new(crate::auth::AuthConfig {
                origin: "http://localhost".into(),
                secure: false,
                cookie_name: "test".into(),
                key: [1; 32],
            }),
        }
    }
    fn input() -> Start {
        Start {
            version: "0.2.4".into(),
            confirmation: "UPDATE FERRIX".into(),
            backup_acknowledged: true,
            password: "not-used".into(),
            code: String::new(),
        }
    }
    #[test]
    fn accepts_only_immutable_official_image_digests() {
        assert!(official_digest(&format!(
            "ghcr.io/eci4ever/ferrix@sha256:{}",
            "a".repeat(64)
        )));
        for value in [
            "ghcr.io/eci4ever/ferrix:latest",
            "ghcr.io/other/ferrix@sha256:abc",
            "ghcr.io/eci4ever/ferrix@sha256:abc",
            "ghcr.io/eci4ever/ferrix@sha256:../../tmp",
        ] {
            assert!(!official_digest(value));
        }
    }
    #[tokio::test]
    async fn rejects_non_admin_keys_missing_confirmation_and_stale_releases_before_side_effects() {
        let (_, cache) = watch::channel(ReleaseStatus::default());
        let result = start(
            State(state()),
            Extension(cache.clone()),
            None,
            identity(false),
            Json(input()),
        )
        .await;
        assert_eq!(result.err().unwrap().0, StatusCode::FORBIDDEN);
        let key = crate::auth::api_keys::KeyAccess {
            id: uuid::Uuid::nil(),
            scopes: vec!["deploy".into()],
            project_id: None,
            project_name: None,
            name: "Test".into(),
        };
        let result = start(
            State(state()),
            Extension(cache.clone()),
            Some(Extension(key)),
            identity(true),
            Json(input()),
        )
        .await;
        assert_eq!(result.err().unwrap().0, StatusCode::FORBIDDEN);
        let mut missing = input();
        missing.backup_acknowledged = false;
        let result = start(
            State(state()),
            Extension(cache.clone()),
            None,
            identity(true),
            Json(missing),
        )
        .await;
        assert_eq!(result.err().unwrap().0, StatusCode::BAD_REQUEST);
        let result = start(
            State(state()),
            Extension(cache),
            None,
            identity(true),
            Json(input()),
        )
        .await;
        assert_eq!(result.err().unwrap().0, StatusCode::CONFLICT);
        assert_eq!(
            status(State(state()), identity(false))
                .await
                .err()
                .unwrap()
                .0,
            StatusCode::FORBIDDEN
        );
    }
}
