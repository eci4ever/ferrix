use super::*;
use bollard::models::{ContainerCreateBody, Mount, MountTypeEnum, VolumeCreateRequest};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct RuntimeConfig {
    #[serde(default = "default_memory_mib")]
    pub memory_mib: i64,
    #[serde(default = "default_cpus")]
    pub cpus: f64,
    #[serde(default)]
    pub worker_mode: bool,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub storage: Option<Storage>,
}
fn default_memory_mib() -> i64 {
    512
}
fn default_cpus() -> f64 {
    1.0
}
impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            memory_mib: default_memory_mib(),
            cpus: default_cpus(),
            worker_mode: false,
            command: Vec::new(),
            storage: None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct Storage {
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}
impl RuntimeConfig {
    pub(super) fn validate(&self) -> Result<(), ApiError> {
        if !(6..=1_048_576).contains(&self.memory_mib) {
            return Err(invalid(
                "Memory limit must be a whole number from 6 to 1048576 MiB. Unlimited memory is not supported.",
            ));
        }
        if !self.cpus.is_finite()
            || !(0.01..=1024.0).contains(&self.cpus)
            || (self.cpus * 100.0 - (self.cpus * 100.0).round()).abs() > 1e-8
        {
            return Err(invalid(
                "CPU limit must be from 0.01 to 1024 cores in steps of 0.01. Unlimited CPU is not supported.",
            ));
        }
        if self.command.len() > 128
            || self.command.iter().map(String::len).sum::<usize>() > 16384
            || self.command.iter().any(|arg| arg.contains('\0'))
        {
            return Err(invalid(
                "Command must contain at most 128 arguments and 16 KiB, without null bytes.",
            ));
        }
        if let Some(s) = &self.storage {
            let p = &s.target;
            if p.len() > 255
                || !p.starts_with('/')
                || p.ends_with('/')
                || p.split('/')
                    .skip(1)
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || p.chars().any(char::is_control)
                || [
                    "/proc", "/sys", "/dev", "/run", "/etc", "/bin", "/sbin", "/usr", "/lib",
                    "/lib64", "/boot", "/var/run",
                ]
                .iter()
                .any(|s| p == s || p.starts_with(&format!("{s}/")))
            {
                return Err(invalid(
                    "Use a dedicated absolute data directory such as /opt/data. Root, system paths and traversal are not allowed.",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn exclusive(&self) -> bool {
        self.worker_mode || self.storage.is_some()
    }
}
pub(super) fn volume(app: Uuid) -> String {
    format!("ferrix-app-{}", app.simple())
}
pub(super) fn decode(
    state: &AppState,
    app: Uuid,
    cipher: Option<&[u8]>,
) -> Result<RuntimeConfig, ApiError> {
    cipher
        .map(|c| services::open(state, app, c))
        .transpose()
        .map(Option::unwrap_or_default)
}
pub(super) async fn load(state: &AppState, app: Uuid) -> Result<RuntimeConfig, ApiError> {
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT service_cipher FROM applications WHERE id=$1")
            .bind(app)
            .fetch_one(&state.pool)
            .await?;
    decode(state, app, cipher.as_deref())
}
pub(super) async fn configure(
    state: &AppState,
    job: &worker::Job,
    config: &mut ContainerCreateBody,
    image: &bollard::models::ImageInspect,
) -> Result<RuntimeConfig, String> {
    let runtime = decode(state, job.application_id, job.service_cipher.as_deref())
        .map_err(|_| "Could not decrypt Application runtime settings.")?;
    runtime.validate().map_err(|e| e.1.to_owned())?;
    let host = config
        .host_config
        .as_mut()
        .ok_or("Missing host configuration.")?;
    host.memory = Some(runtime.memory_mib * 1024 * 1024);
    host.nano_cpus = Some((runtime.cpus * 1_000_000_000.0).round() as i64);
    if !runtime.command.is_empty() {
        config.cmd = Some(runtime.command.clone());
    }
    if runtime.worker_mode {
        config.exposed_ports = None;
        config
            .host_config
            .as_mut()
            .ok_or("Missing host configuration.")?
            .port_bindings = None;
    }
    // Do not allow Docker to create implicit anonymous volumes with untracked data.
    if image
        .config
        .as_ref()
        .and_then(|c| c.volumes.as_ref())
        .is_some_and(|paths| {
            paths
                .iter()
                .any(|p| !runtime.storage.as_ref().is_some_and(|s| s.target == *p))
        })
    {
        return Err("Every image-declared volume must match the configured storage target. This version supports one named volume per Application.".into());
    }
    if let Some(storage) = &runtime.storage {
        let name = volume(job.application_id);
        let info = match state.docker.inspect_volume(&name).await {
            Ok(info) => info,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => state
                .docker
                .create_volume(VolumeCreateRequest {
                    name: Some(name.clone()),
                    labels: Some(HashMap::from([
                        ("io.ferrix.managed".into(), "true".into()),
                        (
                            "io.ferrix.application".into(),
                            job.application_id.to_string(),
                        ),
                        (
                            "io.ferrix.organization".into(),
                            job.organization_id.to_string(),
                        ),
                        ("io.ferrix.project".into(), job.project_id.to_string()),
                    ])),
                    ..Default::default()
                })
                .await
                .map_err(|_| "Could not create Application storage.")?,
            Err(_) => return Err("Could not inspect Application storage.".into()),
        };
        if info.driver != "local"
            || !info.options.is_empty()
            || info.labels.get("io.ferrix.managed").map(String::as_str) != Some("true")
            || info.labels.get("io.ferrix.application") != Some(&job.application_id.to_string())
            || info.labels.get("io.ferrix.organization") != Some(&job.organization_id.to_string())
            || info.labels.get("io.ferrix.project") != Some(&job.project_id.to_string())
        {
            return Err(
                "Application volume ownership or local driver could not be verified.".into(),
            );
        }
        config
            .host_config
            .as_mut()
            .ok_or("Missing host configuration.")?
            .mounts = Some(vec![Mount {
            typ: Some(MountTypeEnum::VOLUME),
            source: Some(name),
            target: Some(storage.target.clone()),
            read_only: Some(storage.read_only),
            volume_options: Some(bollard::models::MountVolumeOptions {
                no_copy: Some(job.start_stopped),
                ..Default::default()
            }),
            ..Default::default()
        }]);
    }
    Ok(runtime)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_defaults_and_resource_validation() {
        let legacy: RuntimeConfig = serde_json::from_value(
            json!({"worker_mode":true,"command":["gateway","run"],"storage":null}),
        )
        .unwrap();
        assert_eq!(legacy.memory_mib, 512);
        assert_eq!(legacy.cpus, 1.0);
        assert_eq!(RuntimeConfig::default().memory_mib, 512);
        assert_eq!(RuntimeConfig::default().cpus, 1.0);
        for memory_mib in [-1, 0, 5, 1_048_577, i64::MAX] {
            assert!(
                RuntimeConfig {
                    memory_mib,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        for cpus in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.001, 0.755, 1024.01] {
            assert!(
                RuntimeConfig {
                    cpus,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        for cpus in [0.01, 0.29, 0.75, 1.0, 1024.0] {
            assert!(
                RuntimeConfig {
                    memory_mib: 768,
                    cpus,
                    ..Default::default()
                }
                .validate()
                .is_ok()
            );
        }
        assert!(serde_json::from_value::<RuntimeConfig>(json!({"memory_mib":768.5})).is_err());
        assert!(serde_json::from_value::<RuntimeConfig>(json!({"cpus":null})).is_err());
    }
    #[test]
    fn validate_runtime_paths_and_literal_arguments() {
        for path in [
            "/",
            "/etc",
            "/proc/a",
            "/var/run/docker.sock",
            "/data/../etc",
            "/data//x",
            "relative",
            "/data/",
        ] {
            assert!(
                RuntimeConfig {
                    storage: Some(Storage {
                        target: path.into(),
                        read_only: false
                    }),
                    ..Default::default()
                }
                .validate()
                .is_err(),
                "{path}"
            );
        }
        assert!(
            RuntimeConfig {
                command: vec!["gateway".into(), "run".into(), "$(literal)".into()],
                worker_mode: true,
                storage: Some(Storage {
                    target: "/opt/data".into(),
                    read_only: false
                }),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
        assert!(
            RuntimeConfig {
                command: vec!["x\0y".into()],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}

pub(super) async fn lock_lineage(
    conn: &mut sqlx::PgConnection,
    app: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_database() || ':' || current_schema() || ':ferrix-lineage:' || COALESCE(data_origin_id,id)::text,0)) FROM applications WHERE id=$1").bind(app).execute(conn).await?;
    Ok(())
}
pub(super) async fn lineage_guard(
    state: &AppState,
    app: Uuid,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, String> {
    let mut guard = state
        .pool
        .begin()
        .await
        .map_err(|_| "Could not begin lineage lock.")?;
    lock_lineage(&mut guard, app)
        .await
        .map_err(|_| "Could not serialize restored service operations.")?;
    Ok(guard)
}
pub(super) async fn ensure_single_writer(
    state: &AppState,
    app: Uuid,
    runtime: &RuntimeConfig,
    conn: &mut sqlx::PgConnection,
) -> Result<(), String> {
    use bollard::query_parameters::ListContainersOptionsBuilder;
    if !runtime.exclusive() {
        return Ok(());
    }
    // Restored clones share a lineage even though each owns a different volume.
    // Do not start a second poller while a sibling runs or is paused for backup.
    let peers:Vec<(Uuid,Option<String>,bool)>=sqlx::query_as("SELECT a.id,a.active_container_id,(EXISTS(SELECT 1 FROM database_jobs j WHERE j.application_id=a.id AND j.resume_needed) OR EXISTS(SELECT 1 FROM project_bundle_members bm JOIN project_bundle_jobs bj ON bj.id=bm.job_id WHERE bm.source_service_id=a.id AND bm.resume_needed AND NOT(bj.status='cutting_over' AND bj.id::text=coalesce(current_setting('ferrix.bundle_context',true),''))) OR EXISTS(SELECT 1 FROM deployments d WHERE d.application_id=a.id AND d.restore_container_id IS NOT NULL)) FROM applications a WHERE a.id<>$1 AND a.service_kind='application' AND COALESCE(a.data_origin_id,a.id)=(SELECT COALESCE(data_origin_id,id) FROM applications WHERE id=$1)").bind(app).fetch_all(&mut *conn).await.map_err(|_|"Could not verify restored service lineage.")?;
    for (peer, container, resuming) in peers {
        if resuming {
            return Err("A related service is paused for backup and may resume. Wait before starting this restored service.".into());
        }
        if let Some(container) = container {
            let info = worker::inspect_owned(state, peer, &container)
                .await
                .map_err(|_| "Could not verify a related restored service.")?;
            if info.state.and_then(|s| s.running) == Some(true) {
                return Err("Stop the source or related restored service before starting this service. Only one restored instance may run.".into());
            }
        }
    }
    let mut filters = HashMap::new();
    if runtime.storage.is_some() {
        filters.insert("volume".to_owned(), vec![volume(app)]);
    } else {
        filters.insert(
            "label".to_owned(),
            vec![format!("io.ferrix.application={app}")],
        );
    }
    let running = state
        .docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(false)
                .filters(&filters)
                .build(),
        ))
        .await
        .map_err(|_| "Could not verify exclusive storage/runtime access.")?;
    // Filtered inventory can include created/stopped entries across Docker versions
    // and transitions. Inspect live state instead of treating every match as a writer.
    for container in running {
        let id = container
            .id
            .ok_or("Docker did not identify a storage container.")?;
        match state.docker.inspect_container(&id,None).await {
            Ok(info) if info.state.as_ref().and_then(|s|s.running)==Some(true)=>return Err("Another running container is using this service or its data. Stop it before deployment; Ferrix will not start a second writer.".into()),
            Ok(_)=>{},
            Err(bollard::errors::Error::DockerResponseServerError{status_code:404,..})=>{},
            Err(_)=>return Err("Could not verify exclusive storage/runtime access.".into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod integration_tests;
