use super::*;
use crate::deployments::worker::{Job, inspect_owned};
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
            .map_err(|_| "Could not read the current service container.")?;
    if let Some(previous) = previous {
        let info = inspect_owned(state, job.application_id, &previous)
            .await
            .map_err(|_| "Could not verify the previous instance container.")?;
        sqlx::query(
            "UPDATE deployments SET restore_container_id=$1,restore_was_running=$3 WHERE id=$2 AND status='running'",
        )
        .bind(&previous)
        .bind(job.id)
        .bind(info.state.as_ref().and_then(|s|s.running)==Some(true))
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not persist service recovery information.")?;
        state
            .docker
            .update_container(
                &previous,
                bollard::models::ContainerUpdateBody {
                    restart_policy: Some(bollard::models::RestartPolicy {
                        name: Some(bollard::models::RestartPolicyNameEnum::NO),
                        maximum_retry_count: None,
                    }),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Could not disable the old instance restart policy safely.")?;
        state
            .docker
            .stop_container(&previous, None)
            .await
            .or_else(|e| match e {
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 304, ..
                } => Ok(()),
                e => Err(e),
            })
            .map_err(|_| "Could not stop the previous instance safely.")?;
    }
    Ok(())
}
pub(in crate::deployments) async fn restore_pending(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<(), String> {
    let pending:Vec<(Uuid,Uuid,String,Option<String>,bool)>=sqlx::query_as("SELECT d.id,d.application_id,d.restore_container_id,a.active_container_id,d.restore_was_running FROM deployments d JOIN applications a ON a.id=d.application_id WHERE d.status='failed' AND d.restore_container_id IS NOT NULL")
        .fetch_all(&mut *conn).await.map_err(|_|"Could not read service recovery information.")?;
    for (deployment, app, previous, active, was_running) in pending {
        if active.as_deref() == Some(&previous) {
            let info = inspect_owned(state, app, &previous)
                .await
                .map_err(|_| "Could not verify the service container for recovery.")?;
            let running = info.state.and_then(|s| s.running) == Some(true);
            if was_running && !running {
                let cipher:Option<Vec<u8>>=sqlx::query_scalar("SELECT service_cipher FROM deployments WHERE id=(SELECT active_deployment_id FROM applications WHERE id=$1) AND service_kind='application'").bind(app).fetch_optional(&mut *conn).await.map_err(|_|"Could not read previous Application snapshot.")?.flatten();
                if let Some(cipher) = cipher {
                    let runtime =
                        super::super::application_runtime::decode(state, app, Some(&cipher))
                            .map_err(|_| "Could not decrypt recovery settings.")?;
                    super::super::application_runtime::ensure_single_writer(state, app, &runtime)
                        .await?;
                }
            }
            state
                .docker
                .update_container(
                    &previous,
                    bollard::models::ContainerUpdateBody {
                        restart_policy: Some(bollard::models::RestartPolicy {
                            name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                            maximum_retry_count: None,
                        }),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| "Could not restore the previous instance restart policy.")?;
            if was_running && !running {
                state
                    .docker
                    .start_container(&previous, None)
                    .await
                    .map_err(|_| "Could not restart the previous instance.")?;
            }
        }
        sqlx::query("UPDATE deployments SET restore_container_id=NULL,logs=logs || E'Previous instance preserved/restored.\\n' WHERE id=$1").bind(deployment).execute(&mut *conn).await.map_err(|_|"Could not persist service recovery.")?;
    }
    Ok(())
}
