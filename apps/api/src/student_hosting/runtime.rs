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
pub(super) async fn server_name(state: &AppState) -> Result<String> {
    Ok(format!("ferrix-students-db-{}", scope(state).await?))
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
        chunk.map_err(|_| unavailable())?;
    }
    if state
        .docker
        .inspect_exec(&job.id)
        .await
        .map_err(|_| unavailable())?
        .exit_code
        != Some(0)
    {
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
            image(state, "mariadb:11.4").await?;
            volume(state, &name, None).await?;
            state
                .docker
                .create_container(
                    Some(CreateContainerOptionsBuilder::default().name(&name).build()),
                    ContainerCreateBody {
                        image: Some("mariadb:11.4".into()),
                        env: Some(vec![format!("MARIADB_ROOT_PASSWORD={pass}")]),
                        labels: Some(expected),
                        host_config: Some(HostConfig {
                            network_mode: Some(net.clone()),
                            memory: Some(1024 * 1024 * 1024),
                            nano_cpus: Some(1_000_000_000),
                            pids_limit: Some(256),
                            security_opt: Some(vec!["no-new-privileges:true".into()]),
                            mounts: Some(vec![Mount {
                                target: Some("/var/lib/mysql".into()),
                                source: Some(name.clone()),
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
                                    aliases: Some(vec![name.clone()]),
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
    let name = format!("ferrix-student-{}-{}", scope(state).await?, user.simple());
    let vol = format!("{name}-files");
    let net = network(state).await?;
    let php = std::env::var("FERRIX_STUDENT_PHP_IMAGE")
        .unwrap_or_else(|_| "ghcr.io/eci4ever/ferrix-student-php:latest".into());
    image(state, &php).await?;
    volume(state, &vol, Some(user)).await?;
    match state.docker.inspect_container(&name, None).await {
        Ok(_) => {
            let existing = owned(state, &name, Some(user)).await?;
            sqlx::query("UPDATE student_hostings SET container_id=$1 WHERE user_id=$2")
                .bind(existing.id)
                .bind(user)
                .execute(&state.pool)
                .await?;
        }
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            let mut config = ContainerCreateBody {
                image: Some(php),
                user: Some("33:33".into()),
                labels: Some(labels(state, Some(user)).await?),
                env: Some(vec![
                    format!("DB_HOST={}", server_name(state).await?),
                    format!("DB_NAME={}", h.database_name),
                    format!("DB_USER={}", h.database_user),
                    format!("DB_PASSWORD={pass}"),
                ]),
                host_config: Some(HostConfig {
                    network_mode: Some(net.clone()),
                    memory: Some(256 * 1024 * 1024),
                    nano_cpus: Some(500_000_000),
                    pids_limit: Some(64),
                    readonly_rootfs: Some(true),
                    cap_drop: Some(vec!["ALL".into()]),
                    security_opt: Some(vec!["no-new-privileges:true".into()]),
                    tmpfs: Some(HashMap::from([
                        (
                            "/tmp".into(),
                            "rw,noexec,nosuid,size=64m,uid=33,gid=33".into(),
                        ),
                        (
                            "/var/run/apache2".into(),
                            "rw,noexec,nosuid,size=8m,uid=33,gid=33".into(),
                        ),
                        (
                            "/var/lock/apache2".into(),
                            "rw,noexec,nosuid,size=8m,uid=33,gid=33".into(),
                        ),
                    ])),
                    mounts: Some(vec![Mount {
                        target: Some("/var/www/html".into()),
                        source: Some(vol),
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
                        EndpointSettings::default(),
                    )])),
                }),
                ..Default::default()
            };
            crate::gateway::attach(state, &mut config)
                .await
                .map_err(|_| unavailable())?;
            let created = state
                .docker
                .create_container(
                    Some(CreateContainerOptionsBuilder::default().name(&name).build()),
                    config,
                )
                .await
                .map_err(|_| unavailable())?;
            sqlx::query("UPDATE student_hostings SET container_id=$1 WHERE user_id=$2")
                .bind(created.id)
                .bind(user)
                .execute(&state.pool)
                .await?;
        }
        Err(_) => return Err(unavailable()),
    }
    // The image seeds an owned /var/www/html directory into newly created volumes.
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
        let info = owned(state, &name, Some(user)).await?;
        if info
            .state
            .as_ref()
            .and_then(|s| s.health.as_ref())
            .and_then(|h| h.status.as_ref())
            .is_some_and(|s| s.to_string() == "healthy")
        {
            sqlx::query("UPDATE student_hostings SET status='active',error=NULL WHERE user_id=$1")
                .bind(user)
                .execute(&state.pool)
                .await?;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(unavailable())
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
    }
    sql(
        state,
        &format!("ALTER USER '{}'@'%' ACCOUNT LOCK;", h.database_user),
    )
    .await
}
pub(crate) async fn upstream(state: &AppState, user: Uuid) -> Result<String> {
    let container:Option<String>=sqlx::query_scalar("SELECT h.container_id FROM student_hostings h JOIN auth_users u ON u.id=h.user_id WHERE h.user_id=$1 AND h.status='active' AND NOT u.banned").bind(user).fetch_optional(&state.pool).await?.flatten();
    let info = owned(state, &container.ok_or_else(missing)?, Some(user)).await?;
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
    Ok(std::net::SocketAddr::new(ip, 80).to_string())
}
pub(super) async fn upload(
    state: &AppState,
    h: &Hosting,
    files: Vec<(String, Vec<u8>)>,
) -> Result<()> {
    let container = h.container_id.as_ref().ok_or_else(unavailable)?;
    owned(state, container, Some(h.user_id)).await?;
    // No archive extraction or shell interpolation. The unprivileged PHP process
    // writes only validated relative paths inside its own persistent web volume.
    let script = r#"$root='/var/www/html';$files=json_decode(stream_get_contents(STDIN),true,512,JSON_THROW_ON_ERROR);foreach($files as $file){$path=$file['path'];if(!preg_match('~^[A-Za-z0-9_/-]+(?:[.][A-Za-z0-9_/-]+)*$~',$path))exit(2);$current=$root;foreach(explode('/',$path) as $part){if($part===''||$part==='.'||$part==='..'||str_starts_with($part,'.'))exit(2);$current.='/'.$part;if(is_link($current))exit(2);} $dest=$root.'/'.$path;if(!is_dir(dirname($dest))&&!mkdir(dirname($dest),0755,true))exit(3);$data=hex2bin($file['hex']);if($data===false||file_put_contents($dest,$data,LOCK_EX)===false)exit(4);}"#;
    let payload = files
        .into_iter()
        .map(|(path, data)| json!({"path":path,"hex":hex::encode(data)}))
        .collect::<Vec<_>>();
    exec(
        state,
        container,
        vec!["php".into(), "-r".into(), script.into()],
        &serde_json::to_string(&payload).map_err(|_| unavailable())?,
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
    sqlx::query("UPDATE student_hosting_settings SET server_state='running',server_error=NULL WHERE singleton").execute(&state.pool).await?;
    let next:Option<Uuid>=sqlx::query_scalar("UPDATE student_hostings SET status='provisioning' WHERE user_id=(SELECT user_id FROM student_hostings WHERE status='queued' ORDER BY approved_at LIMIT 1) RETURNING user_id").fetch_optional(&state.pool).await?;
    if let Some(user) = next
        && provision(state, user).await.is_err()
    {
        sqlx::query("UPDATE student_hostings SET status='failed',error='Could not provision hosting. Check the PHP image, ingress network and shared MariaDB, then retry.' WHERE user_id=$1").bind(user).execute(&state.pool).await?;
    }
    Ok(())
}
