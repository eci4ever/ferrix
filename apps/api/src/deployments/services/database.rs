use super::*;
use crate::deployments::worker::{Job, inspect_owned};
use bollard::models::{
    ContainerCreateBody, EndpointSettings, HealthConfig, Mount, MountTypeEnum,
    NetworkCreateRequest, NetworkingConfig, VolumeCreateRequest,
};

pub(in crate::deployments) fn network(environment: Uuid) -> String {
    format!("ferrix-env-{}", environment.simple())
}
pub(in crate::deployments) async fn attach_network(
    state: &AppState,
    job: &Job,
    config: &mut ContainerCreateBody,
) -> Result<(), String> {
    let name = network(job.environment_id);
    let info = match state.docker.inspect_network(&name, None).await {
        Ok(info) => info,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            state
                .docker
                .create_network(NetworkCreateRequest {
                    name: name.clone(),
                    driver: Some("bridge".into()),
                    labels: Some(HashMap::from([
                        ("io.ferrix.managed".into(), "true".into()),
                        (
                            "io.ferrix.environment".into(),
                            job.environment_id.to_string(),
                        ),
                    ])),
                    ..Default::default()
                })
                .await
                .map_err(|_| "Could not create the environment network.")?;
            state
                .docker
                .inspect_network(&name, None)
                .await
                .map_err(|_| "Could not inspect the environment network.")?
        }
        Err(_) => return Err("Could not inspect the environment network.".into()),
    };
    if !info.labels.as_ref().is_some_and(|l| {
        l.get("io.ferrix.environment") == Some(&job.environment_id.to_string())
            && l.get("io.ferrix.managed").map(String::as_str) == Some("true")
    }) {
        return Err("Environment network ownership could not be verified.".into());
    }
    config
        .host_config
        .as_mut()
        .ok_or("Missing container host configuration.")?
        .network_mode = Some(name.clone());
    config.networking_config = Some(NetworkingConfig {
        endpoints_config: Some(HashMap::from([(
            name,
            EndpointSettings {
                aliases: Some(vec![format!("ferrix-{}", job.application_id.simple())]),
                ..Default::default()
            },
        )])),
    });
    Ok(())
}
pub(in crate::deployments) async fn configure(
    state: &AppState,
    job: &Job,
    config: &mut ContainerCreateBody,
) -> Result<(), String> {
    let database: DatabaseConfig = open(
        state,
        job.application_id,
        job.service_cipher
            .as_deref()
            .ok_or("Database configuration is missing.")?,
    )
    .map_err(|_| "Could not decrypt database configuration.")?;
    let name = format!("ferrix-db-{}", job.application_id.simple());
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
                ])),
                ..Default::default()
            })
            .await
            .map_err(|_| "Could not create persistent database storage.")?,
        Err(_) => return Err("Could not inspect database storage.".into()),
    };
    if info.labels.get("io.ferrix.application") != Some(&job.application_id.to_string())
        || info.labels.get("io.ferrix.managed").map(String::as_str) != Some("true")
    {
        return Err("Database volume ownership could not be verified.".into());
    }
    let env = config.env.get_or_insert_default();
    let (target, health) = match database.engine {
        DatabaseEngine::Postgres => {
            env.extend([
                format!("POSTGRES_DB={}", database.database_name),
                format!("POSTGRES_USER={}", database.username),
                format!("POSTGRES_PASSWORD={}", database.password),
            ]);
            (
                "/var/lib/postgresql/data",
                vec![
                    "CMD-SHELL".into(),
                    "pg_isready -h 127.0.0.1 -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\"".into(),
                ],
            )
        }
        DatabaseEngine::Mongodb => {
            env.extend([
                format!("MONGO_INITDB_DATABASE={}", database.database_name),
                format!("MONGO_INITDB_ROOT_USERNAME={}", database.username),
                format!("MONGO_INITDB_ROOT_PASSWORD={}", database.password),
            ]);
            config.cmd = Some(vec![
                "mongod".into(),
                "--wiredTigerCacheSizeGB".into(),
                "0.25".into(),
            ]);
            ("/data/db",vec!["CMD-SHELL".into(),"mongosh --quiet --host 127.0.0.1 --username \"$MONGO_INITDB_ROOT_USERNAME\" --password \"$MONGO_INITDB_ROOT_PASSWORD\" --authenticationDatabase admin --eval 'quit(db.adminCommand({ping:1}).ok ? 0 : 1)'".into()])
        }
        DatabaseEngine::Mariadb => {
            env.extend([
                format!("MARIADB_DATABASE={}", database.database_name),
                format!("MARIADB_USER={}", database.username),
                format!("MARIADB_PASSWORD={}", database.password),
                format!(
                    "MARIADB_ROOT_PASSWORD={}",
                    database
                        .root_password
                        .as_deref()
                        .ok_or("Missing MariaDB root password.")?
                ),
            ]);
            (
                "/var/lib/mysql",
                vec![
                    "CMD".into(),
                    "healthcheck.sh".into(),
                    "--connect".into(),
                    "--innodb_initialized".into(),
                ],
            )
        }
        DatabaseEngine::Redis => {
            env.push(format!("REDIS_PASSWORD={}", database.password));
            config.cmd = Some(vec![
                "redis-server".into(),
                "--appendonly".into(),
                "yes".into(),
                "--requirepass".into(),
                database.password.clone(),
                "--maxmemory".into(),
                "384mb".into(),
                "--maxmemory-policy".into(),
                "noeviction".into(),
            ]);
            ("/data",vec!["CMD-SHELL".into(),"REDISCLI_AUTH=\"$REDIS_PASSWORD\" redis-cli --no-auth-warning ping | grep -q PONG".into()])
        }
    };
    config.healthcheck = Some(HealthConfig {
        test: Some(health),
        interval: Some(2_000_000_000),
        timeout: Some(5_000_000_000),
        retries: Some(60),
        start_period: Some(10_000_000_000),
        ..Default::default()
    });
    let host = config
        .host_config
        .as_mut()
        .ok_or("Missing database host configuration.")?;
    host.port_bindings = None;
    let mut mounts = vec![Mount {
        typ: Some(MountTypeEnum::VOLUME),
        source: Some(name.clone()),
        target: Some(target.into()),
        ..Default::default()
    }];
    if database.engine == DatabaseEngine::Mongodb {
        host.memory = Some(1024 * 1024 * 1024);
        let config_volume = format!("{name}-config");
        let info = match state.docker.inspect_volume(&config_volume).await {
            Ok(info) => info,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => state
                .docker
                .create_volume(VolumeCreateRequest {
                    name: Some(config_volume.clone()),
                    labels: Some(HashMap::from([
                        ("io.ferrix.managed".into(), "true".into()),
                        (
                            "io.ferrix.application".into(),
                            job.application_id.to_string(),
                        ),
                    ])),
                    ..Default::default()
                })
                .await
                .map_err(|_| "Could not create MongoDB configuration storage.")?,
            Err(_) => return Err("Could not inspect MongoDB configuration storage.".into()),
        };
        if info.labels.get("io.ferrix.application") != Some(&job.application_id.to_string())
            || info.labels.get("io.ferrix.managed").map(String::as_str) != Some("true")
        {
            return Err("MongoDB configuration volume ownership could not be verified.".into());
        }
        mounts.push(Mount {
            typ: Some(MountTypeEnum::VOLUME),
            source: Some(config_volume),
            target: Some("/data/configdb".into()),
            ..Default::default()
        });
    }
    host.mounts = Some(mounts);
    Ok(())
}
pub(in crate::deployments) async fn stop_previous(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    job: &Job,
) -> Result<(), String> {
    let previous: Option<String> =
        sqlx::query_scalar("SELECT active_container_id FROM applications WHERE id=$1")
            .bind(job.application_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|_| "Could not read the current database container.")?;
    if let Some(previous) = previous {
        inspect_owned(state, job.application_id, &previous)
            .await
            .map_err(|_| "Could not verify the previous database container.")?;
        sqlx::query(
            "UPDATE deployments SET restore_container_id=$1 WHERE id=$2 AND status='running'",
        )
        .bind(&previous)
        .bind(job.id)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not persist database recovery information.")?;
        state
            .docker
            .stop_container(&previous, None)
            .await
            .map_err(|_| "Could not stop the previous database safely.")?;
    }
    Ok(())
}
pub(in crate::deployments) async fn restore_pending(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<(), String> {
    let pending:Vec<(Uuid,Uuid,String,Option<String>)>=sqlx::query_as("SELECT d.id,d.application_id,d.restore_container_id,a.active_container_id FROM deployments d JOIN applications a ON a.id=d.application_id WHERE d.status='failed' AND d.restore_container_id IS NOT NULL")
        .fetch_all(&mut *conn).await.map_err(|_|"Could not read database recovery information.")?;
    for (deployment, app, previous, active) in pending {
        if active.as_deref() == Some(&previous) {
            let info = inspect_owned(state, app, &previous)
                .await
                .map_err(|_| "Could not verify the database container for recovery.")?;
            if info.state.and_then(|s| s.running) != Some(true) {
                state
                    .docker
                    .start_container(&previous, None)
                    .await
                    .map_err(|_| "Could not restart the previous database container.")?;
            }
        }
        sqlx::query("UPDATE deployments SET restore_container_id=NULL,logs=logs || E'Previous database container preserved/restored.\\n' WHERE id=$1").bind(deployment).execute(&mut *conn).await.map_err(|_|"Could not persist database recovery.")?;
    }
    Ok(())
}
