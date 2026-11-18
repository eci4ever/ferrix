use super::*;
use crate::deployments::worker::{Job, phase};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

pub(in crate::deployments) fn project(id: Uuid) -> String {
    format!("ferrix-{}", id.simple())
}
fn mapping(value: &Value) -> Result<&serde_json::Map<String, Value>, ApiError> {
    value
        .as_object()
        .ok_or_else(|| invalid("Compose sections must be YAML mappings."))
}
fn resource_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
// This supported subset cannot read server files, attach external resources, or grant host access.
pub(in crate::deployments) fn validate(yaml: &str) -> Result<Value, ApiError> {
    if yaml.len() > 65536 {
        return Err(invalid("Compose YAML must be at most 64 KiB."));
    }
    let document: Value = serde_yaml_ng::from_str(yaml)
        .map_err(|_| invalid("Enter a valid single Compose YAML document."))?;
    let root = mapping(&document)?;
    if root.keys().any(|k| {
        !matches!(
            k.as_str(),
            "services" | "volumes" | "networks" | "name" | "version"
        )
    }) {
        return Err(invalid(
            "Compose supports services, named volumes and bridge networks; includes and server files are not supported.",
        ));
    }
    let services = root
        .get("services")
        .ok_or_else(|| invalid("Compose requires a services mapping."))?;
    let services = mapping(services)?;
    if services.is_empty() || services.len() > 20 {
        return Err(invalid("Compose must define between 1 and 20 services."));
    }
    let volumes = root.get("volumes").and_then(Value::as_object);
    for (key, entries) in [
        ("volumes", volumes),
        ("networks", root.get("networks").and_then(Value::as_object)),
    ] {
        if root.contains_key(key) && !root[key].is_object() {
            return Err(invalid("Volumes and networks must be mappings."));
        }
        if let Some(entries) = entries {
            for (name, config) in entries {
                if !resource_name(name) {
                    return Err(invalid(
                        "Use simple names for Compose volumes and networks.",
                    ));
                }
                if !config.is_null() {
                    let fields = mapping(config)?;
                    if key == "volumes" && !fields.is_empty() {
                        return Err(invalid(
                            "Compose volumes must use the project-scoped local driver without external names or driver options.",
                        ));
                    }
                    if key == "networks"
                        && fields.iter().any(|(k, v)| {
                            !(k == "driver" && v == "bridge" || k == "internal" && v.is_boolean())
                        })
                    {
                        return Err(invalid(
                            "Compose networks must be project-scoped bridge networks; external networks and driver options are not supported.",
                        ));
                    }
                }
            }
        }
    }
    for (name, service) in services {
        if !resource_name(name) {
            return Err(invalid("Use simple names for Compose services."));
        }
        let fields = mapping(service)?;
        if fields.keys().any(|k| {
            !matches!(
                k.as_str(),
                "image"
                    | "command"
                    | "entrypoint"
                    | "environment"
                    | "ports"
                    | "expose"
                    | "volumes"
                    | "depends_on"
                    | "healthcheck"
                    | "restart"
                    | "working_dir"
                    | "user"
                    | "hostname"
                    | "labels"
                    | "networks"
                    | "init"
                    | "stop_grace_period"
                    | "read_only"
                    | "tmpfs"
                    | "mem_limit"
                    | "cpus"
                    | "pids_limit"
                    | "logging"
                    | "pull_policy"
            )
        }) {
            return Err(invalid(
                "Compose contains an unsupported service option. Builds, server file mounts, privileged mode and host namespaces are not supported.",
            ));
        }
        if fields
            .get("image")
            .and_then(Value::as_str)
            .is_none_or(|s| s.is_empty() || s.len() > 255)
        {
            return Err(invalid(
                "Every Compose service requires a public Docker image.",
            ));
        }
        if let Some(mounts) = fields.get("volumes") {
            let mounts = mounts
                .as_array()
                .ok_or_else(|| invalid("Service volumes must be a list."))?;
            for mount in mounts {
                let source = if let Some(text) = mount.as_str() {
                    let (source,_) = text.split_once(':').ok_or_else(||invalid("Use a declared named volume, for example data:/var/lib/postgresql/data."))?;
                    source
                } else {
                    let fields = mapping(mount)?;
                    if fields.get("type").and_then(Value::as_str) != Some("volume")
                        || fields.keys().any(|k| {
                            !matches!(k.as_str(), "type" | "source" | "target" | "read_only")
                        })
                    {
                        return Err(invalid("Only declared named volume mounts are supported."));
                    }
                    fields
                        .get("source")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("Volume mount source is required."))?
                };
                if !resource_name(source) || !volumes.is_some_and(|v| v.contains_key(source)) {
                    return Err(invalid(
                        "Mounts must reference a volume declared in this Compose file. Host paths and anonymous volumes are not supported.",
                    ));
                }
            }
        }
        if let Some(labels) = fields.get("labels") {
            let reserved = |name: &str| {
                name.starts_with("io.ferrix.") || name.starts_with("com.docker.compose.")
            };
            if labels
                .as_object()
                .is_some_and(|m| m.keys().any(|k| reserved(k)))
                || labels
                    .as_array()
                    .is_some_and(|a| a.iter().filter_map(Value::as_str).any(reserved))
            {
                return Err(invalid(
                    "Ferrix and Docker Compose ownership labels are reserved.",
                ));
            }
        }
    }
    Ok(document)
}
#[cfg(unix)]
struct ProcessGroup(Option<std::num::NonZeroI32>);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Only the new process group created for this owned Compose command is terminated.
        if let Some(group) = self.0.take() {
            unsafe {
                libc::kill(-group.get(), libc::SIGKILL);
            }
        }
    }
}
async fn command(directory: &Path, file: &str, args: &[&str], id: Uuid) -> Result<Vec<u8>, String> {
    let mut command = Command::new("docker");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", directory)
        .env("DOCKER_CONFIG", directory.join("docker-config"));
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        command.env("DOCKER_HOST", host);
    }
    command
        .current_dir(directory)
        .args([
            "compose",
            "--ansi",
            "never",
            "--project-name",
            &project(id),
            "--env-file",
            "vars.env",
            "-f",
            file,
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|_| {
        "Docker Compose CLI is unavailable. Install the Docker Compose plugin on this server."
            .to_owned()
    })?;
    #[cfg(unix)]
    let mut _group = ProcessGroup(std::num::NonZeroI32::new(
        i32::try_from(child.id().ok_or("Compose process did not start.")?)
            .map_err(|_| "Invalid Compose process ID.")?,
    ));
    let stdout = child.stdout.take().ok_or("Compose stdout unavailable.")?;
    let stderr = child.stderr.take().ok_or("Compose stderr unavailable.")?;
    let output = async {
        let read = |stream: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>| async move {
            let mut bytes = Vec::new();
            stream
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "Could not read Compose output.")?;
            if bytes.len() > 1024 * 1024 {
                return Err("Compose output exceeded its size limit.");
            }
            Ok(bytes)
        };
        let (out, _error) = tokio::try_join!(read(Box::pin(stdout)), read(Box::pin(stderr)))?;
        let status = child
            .wait()
            .await
            .map_err(|_| "Could not wait for Compose.")?;
        #[cfg(unix)]
        {
            _group.0 = None;
        }
        if !status.success() {
            return Err(
                "Compose command failed. Check YAML, image availability and container healthchecks.",
            );
        }
        Ok(out)
    };
    tokio::time::timeout(Duration::from_secs(180), output)
        .await
        .map_err(|_| "Compose command timed out.".to_owned())?
        .map_err(str::to_owned)
}
fn write_private(path: &Path, contents: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut file| file.write_all(contents))
        .map_err(|_| "Could not prepare private Compose configuration.".into())
}
pub(in crate::deployments) async fn execute(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    job: &Job,
) -> Result<(), String> {
    let config: ComposeConfig = open(
        state,
        job.application_id,
        job.service_cipher
            .as_deref()
            .ok_or("Compose configuration is missing.")?,
    )
    .map_err(|_| "Could not decrypt Compose configuration.")?;
    validate(&config.yaml)
        .map_err(|_| "Compose configuration contains unsupported or unsafe options.")?;
    let vars = decrypt(state, job.application_id, job.env_cipher.as_deref())
        .map_err(|_| "Could not decrypt Compose variables.")?;
    let directory = tempfile::Builder::new()
        .prefix("ferrix-compose-")
        .tempdir()
        .map_err(|_| "Could not create private Compose workspace.")?;
    std::fs::create_dir(directory.path().join("docker-config"))
        .map_err(|_| "Could not create private Docker configuration.")?;
    let plugin_dirs = std::env::var_os("HOME")
        .map(|home| vec![std::path::PathBuf::from(home).join(".docker/cli-plugins")])
        .unwrap_or_default();
    write_private(
        &directory.path().join("docker-config/config.json"),
        &serde_json::to_vec(&json!({"cliPluginsExtraDirs":plugin_dirs}))
            .map_err(|_| "Could not prepare Docker plugin settings.")?,
    )?;
    let mut env_file = String::new();
    for value in vars {
        let (key, value) = value.split_once('=').ok_or("Invalid Compose variable.")?;
        let quoted = serde_json::to_string(value)
            .map_err(|_| "Could not encode Compose variables.")?
            .replace('$', "$$");
        env_file.push_str(&format!("{key}={quoted}\n"));
    }
    write_private(&directory.path().join("vars.env"), env_file.as_bytes())?;
    write_private(
        &directory.path().join("source.yaml"),
        config.yaml.as_bytes(),
    )?;
    phase(conn, job.id, "Validating Compose configuration…").await?;
    let output = command(
        directory.path(),
        "source.yaml",
        &["config", "--format", "json"],
        job.application_id,
    )
    .await?;
    let mut canonical: Value = serde_json::from_slice(&output)
        .map_err(|_| "Compose did not produce a valid configuration.")?;
    let labels = super::labels(
        job.application_id,
        job.id,
        job.organization_id,
        job.project_id,
        job.environment_id,
    );
    let services = canonical
        .get_mut("services")
        .and_then(Value::as_object_mut)
        .ok_or("Compose services are missing.")?;
    for service in services.values_mut() {
        let image = service
            .get("image")
            .and_then(Value::as_str)
            .ok_or("A Compose image is missing.")?;
        let mut check = super::super::Configuration {
            name: "compose".into(),
            image: image.into(),
            container_port: 80,
            environment: String::new(),
        };
        check
            .validate()
            .map_err(|_| "A resolved Compose image reference is invalid.")?;
        let map = service
            .as_object_mut()
            .ok_or("Invalid resolved Compose service.")?;
        let existing = map
            .entry("labels")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("Invalid Compose labels.")?;
        for (key, value) in &labels {
            existing.insert(key.clone(), json!(value));
        }
        map.insert("mem_limit".into(), json!(512 * 1024 * 1024));
        map.insert("cpus".into(), json!(1.0));
        map.insert("pids_limit".into(), json!(256));
        map.insert("security_opt".into(), json!(["no-new-privileges:true"]));
        map.insert("restart".into(), json!("unless-stopped"));
        map.insert(
            "logging".into(),
            json!({"driver":"json-file","options":{"max-size":"10m","max-file":"3"}}),
        );
        if let Some(ports) = map.get_mut("ports").and_then(Value::as_array_mut) {
            for port in ports {
                let port = port.as_object_mut().ok_or("Invalid Compose port.")?;
                port.insert("host_ip".into(), json!("127.0.0.1"));
                port.insert("published".into(), json!("0"));
            }
        }
    }
    let prefix = project(job.application_id);
    for section in ["networks", "volumes"] {
        if let Some(resources) = canonical.get_mut(section).and_then(Value::as_object_mut) {
            for (name, value) in resources {
                let value = value.as_object_mut().ok_or("Invalid Compose resource.")?;
                value.insert("name".into(), json!(format!("{prefix}_{name}")));
                value.insert("labels".into(),json!({"io.ferrix.managed":"true","io.ferrix.application":job.application_id.to_string()}));
            }
        }
    }
    canonical["name"] = json!(prefix);
    verify_ownership(state, job.application_id, &canonical).await?;
    write_private(
        &directory.path().join("compose.json"),
        &serde_json::to_vec(&canonical).map_err(|_| "Could not encode Compose configuration.")?,
    )?;
    phase(
        conn,
        job.id,
        "Pulling images and starting Compose services…",
    )
    .await?;
    command(
        directory.path(),
        "compose.json",
        &[
            "up",
            "--detach",
            "--wait",
            "--wait-timeout",
            "120",
            "--pull",
            "always",
        ],
        job.application_id,
    )
    .await?;
    let mut tx = sqlx::Connection::begin(conn)
        .await
        .map_err(|_| "Could not begin Compose activation.")?;
    let count=sqlx::query("UPDATE deployments SET status='succeeded',finished_at=now(),logs=logs || E'Compose services started successfully.\\n' WHERE id=$1 AND status='running'").bind(job.id).execute(&mut *tx).await.map_err(|_|"Could not save Compose success.")?.rows_affected();
    if count != 1 {
        return Err("Compose deployment was interrupted before activation.".into());
    }
    sqlx::query("UPDATE applications SET active_deployment_id=$1 WHERE id=$2")
        .bind(job.id)
        .bind(job.application_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| "Could not activate Compose deployment.")?;
    tx.commit()
        .await
        .map_err(|_| "Could not confirm Compose activation.")?;
    Ok(())
}
pub(in crate::deployments) async fn containers(
    state: &AppState,
    id: Uuid,
) -> Result<Vec<bollard::models::ContainerSummary>, ApiError> {
    let options = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&HashMap::from([(
            "label".to_owned(),
            vec![
                format!("com.docker.compose.project={}", project(id)),
                format!("io.ferrix.application={id}"),
                "io.ferrix.managed=true".into(),
            ],
        )]))
        .build();
    tokio::time::timeout(
        Duration::from_secs(5),
        state.docker.list_containers(Some(options)),
    )
    .await
    .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Docker is unavailable."))?
    .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Docker is unavailable."))
}
pub(in crate::deployments) async fn runtime(state: &AppState, id: Uuid) -> Result<Value, ApiError> {
    let containers = containers(state, id).await?;
    let count = containers
        .iter()
        .filter(|c| c.state.as_ref().is_some_and(|s| s.to_string() == "running"))
        .count();
    let status = if containers.is_empty() {
        "not_deployed"
    } else if count == containers.len() {
        "running"
    } else if count == 0 {
        "stopped"
    } else {
        "partially_running"
    };
    let health = if containers.iter().any(|c| {
        c.status
            .as_deref()
            .is_some_and(|s| s.contains("(unhealthy)"))
    }) {
        Some("unhealthy")
    } else if containers.iter().any(|c| {
        c.status
            .as_deref()
            .is_some_and(|s| s.contains("(health: starting)"))
    }) {
        Some("starting")
    } else {
        None
    };
    Ok(
        json!({"state":status,"health":health,"project":project(id),"containers":containers.iter().map(|c|json!({"id":c.id,"name":c.labels.as_ref().and_then(|l|l.get("com.docker.compose.service")),"state":c.state,"ports":c.ports.as_ref().map(|p|p.iter().filter_map(|p|p.public_port.map(|port|json!({"port":port,"ip":p.ip,"protocol":p.typ}))).collect::<Vec<_>>())})).collect::<Vec<_>>()}),
    )
}
async fn verify_ownership(state: &AppState, app: Uuid, canonical: &Value) -> Result<(), String> {
    let existing_options = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&HashMap::from([(
            "label".to_owned(),
            vec![format!("com.docker.compose.project={}", project(app))],
        )]))
        .build();
    for container in state
        .docker
        .list_containers(Some(existing_options))
        .await
        .map_err(|_| "Could not verify Compose project ownership.")?
    {
        if !container.labels.as_ref().is_some_and(|labels| {
            labels.get("io.ferrix.application") == Some(&app.to_string())
                && labels.get("io.ferrix.managed").map(String::as_str) == Some("true")
        }) {
            return Err("Compose project name is already used by an unmanaged container.".into());
        }
    }
    for section in ["networks", "volumes"] {
        if let Some(resources) = canonical.get(section).and_then(Value::as_object) {
            for value in resources.values() {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("Missing Compose resource name.")?;
                let labels = if section == "volumes" {
                    match state.docker.inspect_volume(name).await {
                        Ok(info) => Some(info.labels),
                        Err(bollard::errors::Error::DockerResponseServerError {
                            status_code: 404,
                            ..
                        }) => None,
                        Err(_) => return Err("Could not verify Compose volume ownership.".into()),
                    }
                } else {
                    match state.docker.inspect_network(name, None).await {
                        Ok(info) => Some(info.labels.unwrap_or_default()),
                        Err(bollard::errors::Error::DockerResponseServerError {
                            status_code: 404,
                            ..
                        }) => None,
                        Err(_) => return Err("Could not verify Compose network ownership.".into()),
                    }
                };
                if labels.is_some_and(|labels| {
                    labels.get("io.ferrix.application") != Some(&app.to_string())
                        || labels.get("io.ferrix.managed").map(String::as_str) != Some("true")
                }) {
                    return Err("Compose resource ownership could not be verified.".into());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_host_access_and_external_resources() {
        for yaml in [
            "services: {x: {image: nginx, privileged: true}}",
            "services: {x: {image: nginx, volumes: ['/:/host']}}",
            "services: {x: {image: nginx, env_file: /etc/passwd}}",
            "include: /tmp/other.yaml\nservices: {x: {image: nginx}}",
            "services: {x: {image: nginx}}\nvolumes: {data: {external: true}}",
        ] {
            assert!(validate(yaml).is_err());
        }
        assert!(validate("services: {db: {image: postgres:17-alpine, volumes: ['data:/var/lib/postgresql/data']}}\nvolumes: {data: {}}").is_ok());
    }
}
