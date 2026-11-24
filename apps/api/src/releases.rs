//! Read-only stable-release discovery. Never downloads or executes update code.
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::watch;

use crate::{ApiError, auth::Identity};

const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const ENDPOINT: &str = "https://api.github.com/repos/eci4ever/ferrix/releases/latest";
const REPOSITORY: &str = "https://github.com/eci4ever/ferrix";
const BODY_LIMIT: usize = 256 * 1024;

#[derive(Clone, Default, Serialize)]
pub(crate) struct ReleaseStatus {
    current_version: &'static str,
    latest_version: Option<String>,
    update_available: bool,
    release_notes_url: Option<String>,
    upgrade_guide_url: String,
    checked_at: Option<DateTime<Utc>>,
    check_failed: bool,
}

impl ReleaseStatus {
    fn initial() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION"),
            upgrade_guide_url: format!(
                "{REPOSITORY}/blob/main/docs/production.md#operate-and-update"
            ),
            ..Self::default()
        }
    }

    fn apply(&mut self, release: GithubRelease) -> Result<(), &'static str> {
        if release.draft || release.prerelease {
            return Err("Expected a published stable release");
        }
        let latest = stable_version(&release.tag_name).ok_or("Invalid stable release version")?;
        let current = stable_version(self.current_version).ok_or("Invalid running version")?;
        // Construct links from our fixed repository, never from untrusted response URLs.
        self.release_notes_url = Some(format!("{REPOSITORY}/releases/tag/{}", release.tag_name));
        self.upgrade_guide_url = format!(
            "{REPOSITORY}/blob/{}/docs/production.md#operate-and-update",
            release.tag_name
        );
        self.latest_version = Some(release.tag_name.trim_start_matches('v').to_owned());
        self.update_available = latest > current;
        self.checked_at = Some(Utc::now());
        self.check_failed = false;
        Ok(())
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

// Only canonical stable MAJOR.MINOR.PATCH versions are eligible for notification.
fn stable_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let mut number = || {
        let value = parts.next()?;
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return None;
        }
        value.parse::<u64>().ok()
    };
    let version = (number()?, number()?, number()?);
    parts.next().is_none().then_some(version)
}

pub(crate) fn start() -> watch::Receiver<ReleaseStatus> {
    let (sender, receiver) = watch::channel(ReleaseStatus::initial());
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("Ferrix/", env!("CARGO_PKG_VERSION")))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "Release checker could not start");
                sender.send_modify(|status| status.check_failed = true);
                return;
            }
        };
        let mut ticks = tokio::time::interval(INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = sender.closed() => break,
                _ = ticks.tick() => {
                    let result = fetch(&client, ENDPOINT).await.and_then(|release| {
                        let mut next = sender.borrow().clone();
                        next.apply(release).map_err(str::to_owned)?;
                        Ok(next)
                    });
                    match result {
                        Ok(next) => { sender.send_replace(next); }
                        Err(error) => {
                            tracing::warn!(%error, "Stable release check failed; retaining cached release");
                            sender.send_modify(|status| status.check_failed = true);
                        }
                    }
                }
            }
        }
    });
    receiver
}

async fn fetch(client: &reqwest::Client, endpoint: &str) -> Result<GithubRelease, String> {
    let mut response = client
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if chunk.len() > BODY_LIMIT.saturating_sub(body.len()) {
            return Err("Release response exceeded size limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

pub(crate) async fn get(
    Extension(cache): Extension<watch::Receiver<ReleaseStatus>>,
    identity: Identity,
) -> Result<Json<ReleaseStatus>, ApiError> {
    identity.admin()?;
    Ok(Json(cache.borrow().clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> GithubRelease {
        GithubRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
        }
    }

    #[test]
    fn compares_stable_versions_numerically_and_ignores_non_stable_tags() {
        assert!(stable_version("v0.10.0") > stable_version("0.9.9"));
        for tag in [
            "v0.2.3-rc.1",
            "v0.2.3+build",
            "01.2.3",
            "1.2",
            "1.2.3.4",
            "1.2./evil",
            "1.2.+3",
        ] {
            assert!(stable_version(tag).is_none(), "{tag}");
        }
        let mut status = ReleaseStatus::initial();
        status.current_version = "0.2.2";
        status.apply(release("v0.2.3")).unwrap();
        assert!(status.update_available);
        assert!(status.checked_at.is_some());
        assert_eq!(
            status.release_notes_url.as_deref(),
            Some("https://github.com/eci4ever/ferrix/releases/tag/v0.2.3")
        );
        status.apply(release("v0.2.2")).unwrap();
        assert!(!status.update_available);
        status.apply(release("v0.2.1")).unwrap();
        assert!(!status.update_available);
    }

    #[test]
    fn invalid_or_unpublished_release_preserves_previous_success() {
        let mut status = ReleaseStatus::initial();
        status.apply(release("v99.0.0")).unwrap();
        let checked_at = status.checked_at;
        for invalid in [
            release("v99.0.1-beta"),
            GithubRelease {
                draft: true,
                ..release("v99.0.1")
            },
            GithubRelease {
                prerelease: true,
                ..release("v99.0.1")
            },
        ] {
            assert!(status.apply(invalid).is_err());
            assert_eq!(status.latest_version.as_deref(), Some("99.0.0"));
            assert_eq!(status.checked_at, checked_at);
        }
        assert_eq!(INTERVAL, Duration::from_secs(21_600));
    }

    #[tokio::test]
    async fn handles_github_success_rate_limits_and_oversized_responses() {
        use axum::{Router, routing::get};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/latest", get(|| async { Json(serde_json::json!({"tag_name":"v99.0.0", "draft":false, "prerelease":false})) }))
            .route("/limited", get(|| async { axum::http::StatusCode::TOO_MANY_REQUESTS }))
            .route("/large", get(|| async { "x".repeat(BODY_LIMIT + 1) }))
            .route("/invalid", get(|| async { "invalid json" }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let mut status = ReleaseStatus::initial();
        status
            .apply(fetch(&client, &format!("{base}/latest")).await.unwrap())
            .unwrap();
        let checked_at = status.checked_at;
        for path in ["limited", "large", "invalid"] {
            assert!(fetch(&client, &format!("{base}/{path}")).await.is_err());
            // A failed fetch never replaces the last confirmed cache entry.
            assert_eq!(status.latest_version.as_deref(), Some("99.0.0"));
            assert_eq!(status.checked_at, checked_at);
        }
        server.abort();
    }

    #[tokio::test]
    async fn only_platform_admin_can_read_release_status() {
        let (_, cache) = watch::channel(ReleaseStatus::initial());
        let mut identity = Identity {
            session_id: uuid::Uuid::nil(),
            user_id: uuid::Uuid::nil(),
            name: "Test".into(),
            email: "test@example.com".into(),
            system_role: "user".into(),
            organization_id: uuid::Uuid::nil(),
            organization_name: "Test".into(),
            role: "owner".into(),
            authenticated_at: Utc::now(),
            expires_at: Utc::now(),
        };
        assert_eq!(
            get(Extension(cache.clone()), identity.clone())
                .await
                .err()
                .unwrap()
                .0,
            axum::http::StatusCode::FORBIDDEN
        );
        identity.system_role = "admin".into();
        assert!(get(Extension(cache), identity).await.is_ok());
    }
}
