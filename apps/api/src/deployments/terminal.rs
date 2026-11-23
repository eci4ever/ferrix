//! Browser-only PTY. No bearer keys, arbitrary container IDs or elevated exec.
use super::*;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{HeaderMap, header},
    response::Response,
};
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults};
use futures_util::{SinkExt, StreamExt};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/services/{id}/terminal", get(open))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::identify,
        ))
}
fn denied() -> ApiError {
    ApiError(
        StatusCode::FORBIDDEN,
        "Terminal access requires an owner/admin browser session from this Ferrix origin.",
    )
}
async fn open(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    id.manager()?;
    if id.session_id.is_nil()
        || headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
            != Some(state.auth.origin.as_str())
    {
        return Err(denied());
    }
    let app = app_access(&state, &id, service).await?;
    let container = app
        .active_container_id
        .ok_or_else(|| invalid("Deploy and start this service before opening a terminal."))?;
    let info = worker::inspect_owned(&state, service, &container).await?;
    if !info.state.is_some_and(|s| s.running == Some(true)) {
        return Err(invalid("Start the service before opening a terminal."));
    }
    let permit = SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(16)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "All terminal connections are in use. Try again later.",
            )
        })?;
    let connection = Uuid::new_v4();
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("terminal:{}", id.user_id))
        .execute(&mut *tx)
        .await?;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM terminal_connections WHERE user_id=$1 AND closed_at IS NULL AND expires_at>now()").bind(id.user_id).fetch_one(&mut *tx).await?;
    if count >= 2 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Close another terminal first. Each account can open two terminals.",
        ));
    }
    sqlx::query("INSERT INTO terminal_connections(id,user_id,organization_id,service_id) VALUES($1,$2,$3,$4)").bind(connection).bind(id.user_id).bind(id.organization_id).bind(service).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(ws
        .max_message_size(16384)
        .max_frame_size(16384)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            run(socket, state, id, headers, service, container, connection).await;
        }))
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}
async fn send(socket: &mut WebSocket, value: Value) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_secs(5),
            socket.send(Message::Text(value.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    )
}
async fn valid(
    state: &AppState,
    id: &Identity,
    headers: &HeaderMap,
    service: Uuid,
    container: &str,
) -> bool {
    let Ok(current) = crate::auth::sessions::resolve(state, headers).await else {
        return false;
    };
    if current.session_id != id.session_id
        || current.organization_id != id.organization_id
        || current.manager().is_err()
    {
        return false;
    }
    let Ok(app) = app_access(state, &current, service).await else {
        return false;
    };
    if app.active_container_id.as_deref() != Some(container) {
        return false;
    }
    worker::inspect_owned(state, service, container)
        .await
        .ok()
        .is_some_and(|i| i.state.is_some_and(|s| s.running == Some(true)))
}
async fn run(
    mut socket: WebSocket,
    state: AppState,
    id: Identity,
    headers: HeaderMap,
    service: Uuid,
    container: String,
    connection: Uuid,
) {
    let mut exec_id = None;
    let result = async {
        if !valid(&state, &id, &headers, service, &container).await {
            return Err("Terminal permission or service state changed.");
        }
        let path = format!("/tmp/.ferrix-terminal-{}", connection.simple());
        let options = CreateExecOptions {
            attach_stdin: Some(true), attach_stdout: Some(true), attach_stderr: Some(true),
            tty: Some(true), privileged: Some(false),
            env: Some(vec!["TERM=xterm-256color".to_owned(),"PS1=$ ".to_owned(),format!("FERRIX_TERMINAL_ID={connection}")]),
            cmd: Some(vec!["/bin/sh".into(),"-c".into(),"umask 077; echo $$ > \"$1\" || exit 1; exec /bin/sh -i".into(),"ferrix-terminal".into(),path]),
            ..Default::default()
        };
        let exec = tokio::time::timeout(Duration::from_secs(10),state.docker.create_exec(&container,options)).await
            .map_err(|_|"Opening the terminal timed out.")?
            .map_err(|_|"Could not open /bin/sh in this service. The image must include a shell and writable /tmp.")?;
        exec_id = Some(exec.id.clone());
        sqlx::query("UPDATE terminal_connections SET exec_id=$2,container_id=$3 WHERE id=$1")
            .bind(connection).bind(&exec.id).bind(&container).execute(&state.pool).await
            .map_err(|_|"Could not register the terminal connection.")?;
        let attached = tokio::time::timeout(Duration::from_secs(10),state.docker.start_exec(&exec.id,Some(StartExecOptions{tty:true,..Default::default()})))
            .await.map_err(|_|"Starting the shell timed out.")?.map_err(|_|"Could not start a shell in this service.")?;
        let StartExecResults::Attached { mut output,mut input } = attached else {
            return Err("Docker did not attach the terminal.");
        };
        // Wait for the interactive shell prompt: its initial TTY setup may flush
        // input sent immediately after Docker attaches.
        let first = tokio::time::timeout(Duration::from_secs(10), output.next()).await
            .map_err(|_|"The shell did not become ready.")?
            .ok_or("The shell exited before becoming ready.")?
            .map_err(|_|"Could not read the shell prompt.")?;
        if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Binary(first.into_bytes()))).await,Ok(Ok(()))) {return Ok(());}
        if !send(&mut socket,json!({"type":"ready"})).await { return Ok(()); }
        let opened = Instant::now();
        let mut last_input = Instant::now();
        let mut check = tokio::time::interval(Duration::from_secs(10));
        check.tick().await;
        loop {
            tokio::select! {
                _ = check.tick() => {
                    if opened.elapsed()>Duration::from_secs(1800) || last_input.elapsed()>Duration::from_secs(300) {
                        return Err("Terminal closed after its time or inactivity limit.");
                    }
                    if !valid(&state,&id,&headers,service,&container).await {
                        return Err("Terminal closed because your session, permission or service changed.");
                    }
                    let renewed = sqlx::query("UPDATE terminal_connections SET expires_at=now()+interval '60 seconds' WHERE id=$1 AND closed_at IS NULL AND expires_at>now()")
                        .bind(connection).execute(&state.pool).await;
                    if !renewed.is_ok_and(|r|r.rows_affected()==1) { return Err("Terminal could not verify its connection."); }
                }
                line = output.next() => {
                    match line {
                        Some(Ok(line)) => {
                            if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Binary(line.into_bytes()))).await,Ok(Ok(()))) { break; }
                        }
                        Some(Err(_)) => return Err("The Docker terminal disconnected."),
                        None => break,
                    }
                }
                message = socket.recv() => {
                    match message {
                        Some(Ok(Message::Text(text))) => match serde_json::from_str::<Input>(&text) {
                            Ok(Input::Input { data }) if data.len()<=8192 => {
                                last_input=Instant::now();
                                if !matches!(tokio::time::timeout(Duration::from_secs(5),input.write_all(data.as_bytes())).await,Ok(Ok(()))) { break; }
                            }
                            Ok(Input::Resize { cols,rows }) if (2..=300).contains(&cols) && (2..=120).contains(&rows) => {
                                if !matches!(tokio::time::timeout(Duration::from_secs(3),state.docker.resize_exec(&exec.id,ResizeExecOptions{width:cols,height:rows})).await,Ok(Ok(()))) {
                                    return Err("Could not resize the terminal.");
                                }
                            }
                            _ => return Err("Invalid terminal message."),
                        },
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                        _ => return Err("Invalid terminal frame."),
                    }
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(1),input.write_all(b"\x03\nexit\n")).await;
        let _ = tokio::time::timeout(Duration::from_secs(1),input.shutdown()).await;
        Ok::<_,&'static str>(())
    }.await;
    if let Err(message) = result {
        send(&mut socket, json!({"type":"error","message":message})).await;
    }
    let cleaned = if let Some(exec) = exec_id {
        cleanup(&state, service, &container, &exec, connection).await
    } else {
        true
    };
    let _ = sqlx::query("UPDATE terminal_connections SET closed_at=CASE WHEN $2 THEN now() ELSE NULL END,expires_at=now() WHERE id=$1").bind(connection).bind(cleaned).execute(&state.pool).await;
    send(&mut socket, json!({"type":"closed"})).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.close()).await;
}

async fn cleanup(
    state: &AppState,
    service: Uuid,
    container: &str,
    exec: &str,
    connection: Uuid,
) -> bool {
    let work = async {
        if let Err(error) = worker::inspect_owned(state, service, container).await {
            if error.0 == StatusCode::CONFLICT {
                return true;
            }
            return matches!(
                state.docker.inspect_container(container, None).await,
                Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404,
                    ..
                })
            );
        }
        let running = match state.docker.inspect_exec(exec).await {
            Ok(info) => info.running == Some(true),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => false,
            Err(_) => return false,
        };
        let path = format!("/tmp/.ferrix-terminal-{}", connection.simple());
        let script = "if [ \"$3\" = yes ] && read -r p < \"$1\"; then case \"$p\" in ''|*[!0-9]*) ;; *) if tr '\\000' '\\n' < /proc/$p/environ 2>/dev/null | grep -Fqx \"FERRIX_TERMINAL_ID=$2\"; then kill -HUP \"$p\" 2>/dev/null; fi ;; esac; fi; rm -f \"$1\"";
        if let Ok(exec) = state
            .docker
            .create_exec(
                container,
                CreateExecOptions {
                    cmd: Some(vec![
                        "/bin/sh".into(),
                        "-c".into(),
                        script.into(),
                        "ferrix-cleanup".into(),
                        path,
                        connection.to_string(),
                        if running { "yes" } else { "no" }.into(),
                    ]),
                    ..Default::default()
                },
            )
            .await
        {
            return state
                .docker
                .start_exec(
                    &exec.id,
                    Some(StartExecOptions {
                        detach: true,
                        ..Default::default()
                    }),
                )
                .await
                .is_ok();
        }
        false
    };
    tokio::time::timeout(Duration::from_secs(10), work)
        .await
        .unwrap_or(false)
}
/// Expired leases recover shell cleanup even after an interrupted API process.
pub async fn run_terminal_worker(state: AppState, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {_=stop.changed()=>break,_=tick.tick()=>{}}
        type Lease = (Uuid, Option<Uuid>, Option<String>, Option<String>);
        let rows:Result<Vec<Lease>,_>=sqlx::query_as("SELECT id,service_id,container_id,exec_id FROM terminal_connections WHERE closed_at IS NULL AND expires_at<now() ORDER BY expires_at LIMIT 32").fetch_all(&state.pool).await;
        if let Ok(rows) = rows {
            for (connection, service, container, exec) in rows {
                if *stop.borrow() {
                    return;
                }
                if let (Some(service), Some(container), Some(exec)) = (service, container, exec)
                    && !cleanup(&state, service, &container, &exec, connection).await
                {
                    continue;
                }
                let _=sqlx::query("UPDATE terminal_connections SET closed_at=now() WHERE id=$1 AND expires_at<now()").bind(connection).execute(&state.pool).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use std::panic::AssertUnwindSafe;
    #[tokio::test]
    #[ignore = "requires Docker with nginx:alpine available"]
    async fn cleanup_targets_only_the_marked_shell_and_keeps_the_service_running() {
        let service = Uuid::new_v4();
        let connection = Uuid::new_v4();
        let docker = bollard::Docker::connect_with_defaults().unwrap();
        let container = docker
            .create_container(
                None::<bollard::query_parameters::CreateContainerOptions>,
                bollard::models::ContainerCreateBody {
                    image: Some("nginx:alpine".into()),
                    labels: Some(std::collections::HashMap::from([
                        ("io.ferrix.managed".into(), "true".into()),
                        ("io.ferrix.application".into(), service.to_string()),
                    ])),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .id;
        let result = AssertUnwindSafe(async {
            docker
                .start_container(
                    &container,
                    None::<bollard::query_parameters::StartContainerOptions>,
                )
                .await
                .unwrap();
            let state = AppState {
                docker: docker.clone(),
                pool: sqlx::postgres::PgPoolOptions::new()
                    .connect_lazy("postgres://ferrix@127.0.0.1/unused")
                    .unwrap(),
                gateway: Default::default(),
                auth: Arc::new(crate::auth::AuthConfig {
                    origin: "http://localhost:5173".into(),
                    secure: false,
                    cookie_name: "unused".into(),
                    key: [0; 32],
                }),
            };
            let path = format!("/tmp/.ferrix-terminal-{}", connection.simple());
            let exec = docker
                .create_exec(
                    &container,
                    CreateExecOptions {
                        attach_stdin: Some(true),
                        attach_stdout: Some(true),
                        tty: Some(true),
                        env: Some(vec![format!("FERRIX_TERMINAL_ID={connection}")]),
                        cmd: Some(vec![
                            "/bin/sh".into(),
                            "-c".into(),
                            "echo $$ > \"$1\"; exec /bin/sh -i".into(),
                            "ferrix-test".into(),
                            path,
                        ]),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let StartExecResults::Attached {
                mut output,
                input: _input,
            } = docker
                .start_exec(
                    &exec.id,
                    Some(StartExecOptions {
                        tty: true,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("must attach")
            };
            tokio::time::timeout(Duration::from_secs(5), output.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(cleanup(&state, service, &container, &exec.id, Uuid::new_v4()).await);
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(
                docker.inspect_exec(&exec.id).await.unwrap().running,
                Some(true)
            );
            assert!(cleanup(&state, service, &container, &exec.id, connection).await);
            let mut stopped = false;
            for _ in 0..30 {
                if docker.inspect_exec(&exec.id).await.unwrap().running == Some(false) {
                    stopped = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(stopped, "cleanup must terminate the marked shell");
            assert_eq!(
                docker
                    .inspect_container(&container, None)
                    .await
                    .unwrap()
                    .state
                    .unwrap()
                    .running,
                Some(true)
            );
        })
        .catch_unwind()
        .await;
        docker
            .remove_container(
                &container,
                Some(
                    bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                        .force(true)
                        .build(),
                ),
            )
            .await
            .unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error)
        }
    }
}
