use super::*;
use bollard::{
    exec::{CreateExecOptions, StartExecResults},
    models::{
        ContainerCreateBody, EndpointSettings, HostConfig, Mount, MountTypeEnum,
        NetworkCreateRequest, NetworkingConfig, RestartPolicy, RestartPolicyNameEnum,
        VolumeCreateRequest,
    },
    query_parameters::{CreateContainerOptionsBuilder, CreateImageOptionsBuilder},
};
use futures_util::StreamExt;
use std::{collections::HashMap, time::Duration};
use tokio::io::AsyncWriteExt;

type Result<T> = std::result::Result<T, ApiError>;
async fn scope(state: &AppState) -> Result<String> {
    let id: Uuid =
        sqlx::query_scalar("SELECT instance_id FROM student_hosting_settings WHERE singleton")
            .fetch_one(&state.pool)
            .await?;
    Ok(id.simple().to_string())
}
fn compose_project() -> Option<String> {
    std::env::var("FERRIX_HOSTING_COMPOSE_PROJECT")
        .or_else(|_| std::env::var("COMPOSE_PROJECT_NAME"))
        .ok()
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 63
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
        })
}
pub(super) async fn server_name(state: &AppState) -> Result<String> {
    Ok(match compose_project() {
        Some(project) => format!("{project}-mariadb-1"),
        None => format!("ferrix-students-db-{}", scope(state).await?),
    })
}
async fn network(state: &AppState) -> Result<String> {
    Ok(format!("ferrix-students-{}", scope(state).await?))
}
async fn labels(state: &AppState, user: Option<Uuid>) -> Result<HashMap<String, String>> {
    let mut l = HashMap::from([
        ("io.ferrix.managed".into(), "true".into()),
        ("io.ferrix.hosting".into(), scope(state).await?),
    ]);
    if let Some(u) = user {
        l.insert("io.ferrix.student".into(), u.to_string());
    }
    Ok(l)
}
async fn owned(
    state: &AppState,
    name: &str,
    user: Option<Uuid>,
) -> Result<bollard::models::ContainerInspectResponse> {
    let info = state
        .docker
        .inspect_container(name, None)
        .await
        .map_err(|_| unavailable())?;
    let expected = labels(state, user).await?;
    if !info
        .config
        .as_ref()
        .and_then(|c| c.labels.as_ref())
        .is_some_and(|l| expected.iter().all(|(k, v)| l.get(k) == Some(v)))
    {
        return Err(unavailable());
    }
    Ok(info)
}
async fn image(state: &AppState, name: &str) -> Result<()> {
    if state.docker.inspect_image(name).await.is_ok() {
        return Ok(());
    }
    let mut stream = state.docker.create_image(
        Some(
            CreateImageOptionsBuilder::default()
                .from_image(name)
                .build(),
        ),
        None,
        None,
    );
    while let Some(event) = stream.next().await {
        if event.map_err(|_| unavailable())?.error_detail.is_some() {
            return Err(unavailable());
        }
    }
    Ok(())
}
async fn volume(state: &AppState, name: &str, user: Option<Uuid>) -> Result<()> {
    let expected = labels(state, user).await?;
    let info = match state.docker.inspect_volume(name).await {
        Ok(v) => v,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => state
            .docker
            .create_volume(VolumeCreateRequest {
                name: Some(name.into()),
                labels: Some(expected.clone()),
                ..Default::default()
            })
            .await
            .map_err(|_| unavailable())?,
        Err(_) => return Err(unavailable()),
    };
    if !expected.iter().all(|(k, v)| info.labels.get(k) == Some(v)) {
        return Err(unavailable());
    }
    Ok(())
}
async fn exec(state: &AppState, container: &str, command: Vec<String>, input: &str) -> Result<()> {
    let diagnostic = command
        .first()
        .is_some_and(|s| s == "/usr/local/bin/hosting-manager");
    let mut diagnostic_output = String::new();
    let job = state
        .docker
        .create_exec(
            container,
            CreateExecOptions {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(command),
                ..Default::default()
            },
        )
        .await
        .map_err(|_| unavailable())?;
    let StartExecResults::Attached {
        input: mut stdin,
        mut output,
    } = state
        .docker
        .start_exec(&job.id, None)
        .await
        .map_err(|_| unavailable())?
    else {
        return Err(unavailable());
    };
    stdin
        .write_all(input.as_bytes())
        .await
        .map_err(|_| unavailable())?;
    stdin.shutdown().await.map_err(|_| unavailable())?;
    // Never expose SQL or credentials returned by the database client.
    while let Some(chunk) = output.next().await {
        let chunk = chunk.map_err(|_| unavailable())?;
        if diagnostic && diagnostic_output.len() < 4096 {
            diagnostic_output.push_str(&chunk.to_string());
        }
    }
    if state
        .docker
        .inspect_exec(&job.id)
        .await
        .map_err(|_| unavailable())?
        .exit_code
        != Some(0)
    {
        if diagnostic {
            tracing::warn!(detail=%diagnostic_output,"Hosting manager failed");
        }
        return Err(unavailable());
    }
    Ok(())
}
async fn sql(state: &AppState, statement: &str) -> Result<()> {
    let name = server_name(state).await?;
    owned(state, &name, None).await?;
    exec(
        state,
        &name,
        vec![
            "sh".into(),
            "-c".into(),
            "exec mariadb --protocol=socket -uroot -p\"$MARIADB_ROOT_PASSWORD\"".into(),
        ],
        statement,
    )
    .await
}
async fn ensure_server(state: &AppState) -> Result<()> {
    let name = server_name(state).await?;
    let net = network(state).await?;
    let expected = labels(state, None).await?;
    // Keep the original volume identity even when the display/container name changes.
    let storage = format!("ferrix-students-db-{}", scope(state).await?);
    let mut container_labels = expected.clone();
    if let Some(project) = compose_project() {
        container_labels.insert("com.docker.compose.project".into(), project);
        container_labels.insert("com.docker.compose.service".into(), "mariadb".into());
        container_labels.insert("com.docker.compose.container-number".into(), "1".into());
        // No Compose config-hash: Ferrix retains lifecycle ownership. Compose
        // must not adopt this dynamically configured database as a YAML service.
    }
    let info = match state.docker.inspect_network(&net, None).await {
        Ok(n) => n,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            state
                .docker
                .create_network(NetworkCreateRequest {
                    name: net.clone(),
                    driver: Some("bridge".into()),
                    labels: Some(expected.clone()),
                    ..Default::default()
                })
                .await
                .map_err(|_| unavailable())?;
            state
                .docker
                .inspect_network(&net, None)
                .await
                .map_err(|_| unavailable())?
        }
        Err(_) => return Err(unavailable()),
    };
    if !info
        .labels
        .as_ref()
        .is_some_and(|l| expected.iter().all(|(k, v)| l.get(k) == Some(v)))
    {
        return Err(unavailable());
    }
    let cipher: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT root_cipher FROM student_hosting_settings WHERE singleton")
            .fetch_one(&state.pool)
            .await?;
    let pass = if let Some(cipher) = cipher {
        open(state, Uuid::nil(), &cipher)?
    } else {
        let p = secret();
        sqlx::query("UPDATE student_hosting_settings SET root_cipher=$1 WHERE singleton")
            .bind(seal(state, Uuid::nil(), &p)?)
            .execute(&state.pool)
            .await?;
        p
    };
    match state.docker.inspect_container(&name, None).await {
        Ok(_) => {
            owned(state, &name, None).await?;
        }
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            // A legacy container must be retired before changing its name.
            // Never start a second MariaDB process against the same data volume.
            if name != storage && state.docker.inspect_container(&storage, None).await.is_ok() {
                return Err(unavailable());
            }
            image(state, "mariadb:11.4").await?;
            volume(state, &storage, None).await?;
            state
                .docker
                .create_container(
                    Some(CreateContainerOptionsBuilder::default().name(&name).build()),
                    ContainerCreateBody {
                        image: Some("mariadb:11.4".into()),
                        env: Some(vec![format!("MARIADB_ROOT_PASSWORD={pass}")]),
                        labels: Some(container_labels),
                        host_config: Some(HostConfig {
                            network_mode: Some(net.clone()),
                            memory: Some(1024 * 1024 * 1024),
                            nano_cpus: Some(1_000_000_000),
                            pids_limit: Some(256),
                            security_opt: Some(vec!["no-new-privileges:true".into()]),
                            mounts: Some(vec![Mount {
                                target: Some("/var/lib/mysql".into()),
                                source: Some(storage.clone()),
                                typ: Some(MountTypeEnum::VOLUME),
                                ..Default::default()
                            }]),
                            restart_policy: Some(RestartPolicy {
                                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        networking_config: Some(NetworkingConfig {
                            endpoints_config: Some(HashMap::from([(
                                net.clone(),
                                EndpointSettings {
                                    aliases: Some(vec![
                                        name.clone(),
                                        storage.clone(),
                                        "mariadb".into(),
                                    ]),
                                    ..Default::default()
                                },
                            )])),
                        }),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| unavailable())?;
        }
        Err(_) => return Err(unavailable()),
    }
    state
        .docker
        .start_container(
            &name,
            None::<bollard::query_parameters::StartContainerOptions>,
        )
        .await
        .or_else(|e| match e {
            bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            } => Ok(()),
            _ => Err(e),
        })
        .map_err(|_| unavailable())?;
    for _ in 0..90 {
        if sql(state, "SELECT 1;").await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(unavailable())
}
fn valid_database(h: &Hosting) -> bool {
    let short = &h.user_id.simple().to_string()[..24];
    h.database_name == format!("student_{short}") && h.database_user == format!("s_{short}")
}
async fn provision(state: &AppState, user: Uuid) -> Result<()> {
    let h: Hosting = sqlx::query_as(&format!("{SELECT} WHERE h.user_id=$1 AND NOT u.banned"))
        .bind(user)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(missing)?;
    if !valid_database(&h) {
        return Err(unavailable());
    }
    let pass = open(state, user, &h.password_cipher)?;
    // Escape database-level grant wildcards as well as quoting identifiers.
    let grant = h.database_name.replace('_', "\\_");
    sql(state,&format!("CREATE DATABASE IF NOT EXISTS `{db}` CHARACTER SET utf8mb4; CREATE USER IF NOT EXISTS '{usr}'@'%' IDENTIFIED BY '{pass}'; ALTER USER '{usr}'@'%' IDENTIFIED BY '{pass}' ACCOUNT UNLOCK; GRANT ALL PRIVILEGES ON `{grant}`.* TO '{usr}'@'%'; ALTER USER '{usr}'@'%' WITH MAX_USER_CONNECTIONS 10;",db=h.database_name,usr=h.database_user)).await?;
    if h.container_id.is_some() {
        // Never discard an older installation's per-student files implicitly.
        return Err(unavailable());
    }
    ensure_runtime(state).await?;
    sync_pools(state, Some(user)).await?;
    sqlx::query("UPDATE student_hostings SET status='active',error=NULL WHERE user_id=$1")
        .bind(user)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub(crate) async fn shared_volumes(state: &AppState) -> Result<(String, String, String)> {
    let project = compose_project().unwrap_or(format!("ferrix-hosting-{}", scope(state).await?));
    Ok((
        format!("{project}_hosting_files"),
        format!("{project}_hosting_sockets"),
        format!("{project}_hosting_runtime"),
    ))
}
async fn runtime_name(state: &AppState) -> Result<String> {
    Ok(format!(
        "{}-php-fpm-1",
        compose_project().unwrap_or(format!("ferrix-hosting-{}", scope(state).await?))
    ))
}
async fn shared_volume(state: &AppState, name: &str, purpose: &str) -> Result<()> {
    let project = compose_project().unwrap_or(format!("ferrix-hosting-{}", scope(state).await?));
    let expected = HashMap::from([
        ("io.ferrix.hosting.project".to_string(), project),
        ("io.ferrix.hosting.purpose".to_string(), purpose.to_string()),
    ]);
    let info = match state.docker.inspect_volume(name).await {
        Ok(info) => info,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            let mut all = labels(state, None).await?;
            all.extend(expected.clone());
            state
                .docker
                .create_volume(VolumeCreateRequest {
                    name: Some(name.into()),
                    labels: Some(all),
                    ..Default::default()
                })
                .await
                .map_err(|_| unavailable())?
        }
        Err(_) => return Err(unavailable()),
    };
    if !expected.iter().all(|(k, v)| info.labels.get(k) == Some(v)) {
        return Err(unavailable());
    }
    Ok(())
}
async fn ensure_runtime(state: &AppState) -> Result<()> {
    let name = runtime_name(state).await?;
    let net = network(state).await?;
    let (files, sockets, config) = shared_volumes(state).await?;
    for (name, purpose) in [
        (&files, "files"),
        (&sockets, "sockets"),
        (&config, "runtime"),
    ] {
        shared_volume(state, name, purpose).await?;
    }
    match state.docker.inspect_container(&name, None).await {
        Ok(_) => {
            owned(state, &name, None).await?;
        }
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            let php = std::env::var("FERRIX_HOSTING_PHP_IMAGE")
                .unwrap_or_else(|_| "ghcr.io/eci4ever/ferrix-php-fpm:latest".into());
            image(state, &php).await?;
            let mut l = labels(state, None).await?;
            if let Some(project) = compose_project() {
                l.insert("com.docker.compose.project".into(), project);
                l.insert("com.docker.compose.service".into(), "php-fpm".into());
                l.insert("com.docker.compose.container-number".into(), "1".into());
            }
            let mount = |(name, path): (&str, &str)| Mount {
                source: Some(name.into()),
                target: Some(path.into()),
                typ: Some(MountTypeEnum::VOLUME),
                ..Default::default()
            };
            state
                .docker
                .create_container(
                    Some(CreateContainerOptionsBuilder::default().name(&name).build()),
                    ContainerCreateBody {
                        image: Some(php),
                        labels: Some(l),
                        host_config: Some(HostConfig {
                            network_mode: Some(net.clone()),
                            memory: Some(1024 * 1024 * 1024),
                            nano_cpus: Some(1_000_000_000),
                            pids_limit: Some(128),
                            cap_drop: Some(vec!["ALL".into()]),
                            cap_add: Some(
                                [
                                    "CHOWN",
                                    "DAC_OVERRIDE",
                                    "SETUID",
                                    "SETGID",
                                    "SYS_CHROOT",
                                    "KILL",
                                ]
                                .iter()
                                .map(|s| (*s).into())
                                .collect(),
                            ),
                            security_opt: Some(vec!["no-new-privileges:true".into()]),
                            mounts: Some(vec![
                                mount((&files, "/srv/hosting")),
                                mount((&sockets, "/run/hosting")),
                                mount((&config, "/runtime")),
                            ]),
                            tmpfs: Some(HashMap::from([(
                                "/tmp".into(),
                                "rw,nosuid,noexec,size=64m".into(),
                            )])),
                            restart_policy: Some(RestartPolicy {
                                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        networking_config: Some(NetworkingConfig {
                            endpoints_config: Some(HashMap::from([(
                                net,
                                EndpointSettings::default(),
                            )])),
                        }),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| unavailable())?;
        }
        Err(_) => return Err(unavailable()),
    }
    state
        .docker
        .start_container(
            &name,
            None::<bollard::query_parameters::StartContainerOptions>,
        )
        .await
        .or_else(|e| match e {
            bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            } => Ok(()),
            _ => Err(e),
        })
        .map_err(|_| unavailable())?;
    for _ in 0..30 {
        if exec(
            state,
            &name,
            vec!["/usr/local/bin/hosting-manager".into(), "ready".into()],
            "",
        )
        .await
        .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(unavailable())
}
async fn pool_row(state: &AppState, h: &Hosting) -> Result<Value> {
    Ok(
        json!({"id":h.user_id.simple().to_string(),"uid":h.unix_uid,"database":h.database_name,"username":h.database_user,"password":open(state,h.user_id,&h.password_cipher)?}),
    )
}
async fn manager(state: &AppState, operation: &str, payload: Value) -> Result<()> {
    let name = runtime_name(state).await?;
    owned(state, &name, None).await?;
    exec(
        state,
        &name,
        vec!["/usr/local/bin/hosting-manager".into(), operation.into()],
        &payload.to_string(),
    )
    .await
}
async fn sync_pools(state: &AppState, include: Option<Uuid>) -> Result<()> {
    let rows:Vec<Hosting>=sqlx::query_as(&format!("{SELECT} WHERE (h.status='active' OR h.user_id=$1) AND h.container_id IS NULL AND NOT u.banned ORDER BY h.user_id")).bind(include).fetch_all(&state.pool).await?;
    let mut students = Vec::new();
    for row in &rows {
        students.push(pool_row(state, row).await?);
    }
    manager(state, "sync", json!({"students":students})).await?;
    // A graceful reload must recreate the approved pool socket before activation.
    if let Some(user) = include {
        let path = format!("/run/hosting/{}.sock", user.simple());
        let name = runtime_name(state).await?;
        for _ in 0..30 {
            if exec(
                state,
                &name,
                vec!["test".into(), "-S".into(), path.clone()],
                "",
            )
            .await
            .is_ok()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        return Err(unavailable());
    }
    Ok(())
}
pub(super) async fn suspend(state: &AppState, user: Uuid) -> Result<()> {
    let h: Hosting = sqlx::query_as(&format!("{SELECT} WHERE h.user_id=$1"))
        .bind(user)
        .fetch_one(&state.pool)
        .await?;
    if !valid_database(&h) {
        return Err(unavailable());
    }
    if let Some(container) = h.container_id {
        owned(state, &container, Some(user)).await?;
        state
            .docker
            .stop_container(
                &container,
                None::<bollard::query_parameters::StopContainerOptions>,
            )
            .await
            .map_err(|_| unavailable())?;
    } else {
        manager(state, "suspend", json!({"id":user.simple().to_string()})).await?;
    }
    sql(
        state,
        &format!("ALTER USER '{}'@'%' ACCOUNT LOCK;", h.database_user),
    )
    .await
}
pub(crate) async fn upstream(
    state: &AppState,
    user: Uuid,
) -> Result<(String, String, Option<String>)> {
    let row:Option<Option<String>>=sqlx::query_scalar("SELECT h.container_id FROM student_hostings h JOIN auth_users u ON u.id=h.user_id WHERE h.user_id=$1 AND h.status='active' AND NOT u.banned").bind(user).fetch_optional(&state.pool).await?;
    let container = row.ok_or_else(missing)?;
    // Keep older installations readable until their files are explicitly migrated.
    if let Some(container) = container {
        let info = owned(state, &container, Some(user)).await?;
        if info.state.as_ref().and_then(|s| s.running) != Some(true) {
            return Err(unavailable());
        }
        let network = state.gateway.ingress_network().ok_or_else(unavailable)?;
        let ip = info
            .network_settings
            .and_then(|s| s.networks)
            .and_then(|mut n| n.remove(network))
            .and_then(|n| n.ip_address)
            .and_then(|ip| ip.parse::<std::net::IpAddr>().ok())
            .ok_or_else(unavailable)?;
        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
            return Err(unavailable());
        }
        return Ok((
            "application".into(),
            std::net::SocketAddr::new(ip, 80).to_string(),
            None,
        ));
    }
    let info = owned(state, &runtime_name(state).await?, None).await?;
    if info.state.as_ref().and_then(|s| s.running) != Some(true) {
        return Err(unavailable());
    }
    Ok((
        "php-fpm".into(),
        format!("unix//run/hosting/{}.sock", user.simple()),
        Some(format!("/srv/hosting/{}/site", user.simple())),
    ))
}
pub(super) async fn upload(
    state: &AppState,
    h: &Hosting,
    files: Vec<(String, Vec<u8>)>,
) -> Result<()> {
    if h.container_id.is_some() {
        return Err(invalid(
            "This legacy hosting must be migrated to shared PHP-FPM before uploading.",
        ));
    }
    let payload = files
        .into_iter()
        .map(|(path, data)| json!({"path":path,"hex":hex::encode(data)}))
        .collect::<Vec<_>>();
    manager(
        state,
        "upload",
        json!({"id":h.user_id.simple().to_string(),"files":payload}),
    )
    .await
}
pub async fn run_worker(state: AppState, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        if let Err(error) = tick(&state).await {
            tracing::warn!(?error, "Student hosting reconciliation failed");
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_secs(2))=>{},_=shutdown.changed()=>{}}
    }
}
async fn tick(state: &AppState) -> Result<()> {
    let mut conn = state.pool.acquire().await?;
    let lock:bool=sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtext(current_database() || ':' || current_schema() || ':student-hosting'))").fetch_one(&mut *conn).await?;
    if !lock {
        return Ok(());
    }
    let result = tokio::time::timeout(Duration::from_secs(300), reconcile(state))
        .await
        .map_err(|_| unavailable())
        .and_then(|r| r);
    let unlock=sqlx::query("SELECT pg_advisory_unlock(hashtext(current_database() || ':' || current_schema() || ':student-hosting'))").execute(&mut *conn).await;
    if unlock.is_err() {
        let _ = conn.close().await;
    }
    result
}
async fn reconcile(state: &AppState) -> Result<()> {
    let enabled: bool =
        sqlx::query_scalar("SELECT enabled FROM student_hosting_settings WHERE singleton")
            .fetch_one(&state.pool)
            .await?;
    if !enabled {
        return Ok(());
    }
    // A process crash leaves the record retryable and resource names deterministic.
    sqlx::query("UPDATE student_hostings SET status='queued' WHERE status='provisioning'")
        .execute(&state.pool)
        .await?;
    if ensure_server(state).await.is_err() {
        sqlx::query("UPDATE student_hosting_settings SET server_state='failed',server_error='Shared MariaDB is unavailable. Check Docker and retry.' WHERE singleton").execute(&state.pool).await?;
        return Err(unavailable());
    }
    if ensure_runtime(state).await.is_err() || sync_pools(state, None).await.is_err() {
        sqlx::query("UPDATE student_hosting_settings SET server_state='failed',server_error='Shared PHP-FPM is unavailable. Check its image, pool configuration and shared volumes.' WHERE singleton").execute(&state.pool).await?;
        return Err(unavailable());
    }
    sqlx::query("UPDATE student_hosting_settings SET server_state='running',server_error=NULL WHERE singleton").execute(&state.pool).await?;
    let next:Option<Uuid>=sqlx::query_scalar("UPDATE student_hostings SET status='provisioning' WHERE user_id=(SELECT user_id FROM student_hostings WHERE status='queued' ORDER BY approved_at LIMIT 1) RETURNING user_id").fetch_optional(&state.pool).await?;
    if let Some(user) = next
        && provision(state, user).await.is_err()
    {
        sqlx::query("UPDATE student_hostings SET status='failed',error='Could not provision hosting. Check the PHP image, ingress network and shared MariaDB, then retry.' WHERE user_id=$1").bind(user).execute(&state.pool).await?;
    }
    Ok(())
}

// Output is private user data. Never merge stderr or log the payload/result.
pub(super) async fn operation(
    state: &AppState,
    h: &Hosting,
    mode: &str,
    mut payload: Value,
) -> Result<Value> {
    if h.container_id.is_some() {
        return Err(invalid("Migrate this hosting to shared PHP-FPM first."));
    }
    payload["id"] = json!(h.user_id.simple().to_string());
    let name = runtime_name(state).await?;
    owned(state, &name, None).await?;
    let job = state
        .docker
        .create_exec(
            &name,
            CreateExecOptions {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(vec!["/usr/local/bin/hosting-manager", mode]),
                ..Default::default()
            },
        )
        .await
        .map_err(|_| unavailable())?;
    let StartExecResults::Attached {
        input: mut stdin,
        mut output,
    } = state
        .docker
        .start_exec(&job.id, None)
        .await
        .map_err(|_| unavailable())?
    else {
        return Err(unavailable());
    };
    // Write concurrently: Docker's stdin and stdout can both exceed pipe capacity.
    let input = payload.to_string();
    let writer = async {
        stdin.write_all(input.as_bytes()).await?;
        stdin.shutdown().await
    };
    let reader = async {
        let mut result = Vec::new();
        while let Some(chunk) = output.next().await {
            let chunk = chunk.map_err(|_| unavailable())?;
            if let bollard::container::LogOutput::StdOut { message } = chunk {
                if result.len() + message.len() > 34 * 1024 * 1024 {
                    return Err(unavailable());
                }
                result.extend_from_slice(&message);
            }
        }
        Ok::<_, ApiError>(result)
    };
    let (written, result) = tokio::join!(writer, reader);
    written.map_err(|_| unavailable())?;
    let result = result?;
    if state
        .docker
        .inspect_exec(&job.id)
        .await
        .map_err(|_| unavailable())?
        .exit_code
        != Some(0)
    {
        return Err(invalid(
            "Operation failed. Check the path, file size or SQL permissions. Folders must be empty before deletion; imports may have applied earlier statements.",
        ));
    }
    serde_json::from_slice(&result).map_err(|_| unavailable())
}
