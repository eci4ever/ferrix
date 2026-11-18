use super::*;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
pub(super) mod compose;
pub(super) mod database;
mod removal;

pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/environments/{id}/services", get(list))
        .route("/services/{id}", get(application))
        .route("/services/{id}/configuration", get(configuration))
        .route("/services/{id}/runtime", get(super::runtime))
        .route("/services/{id}/deployments", get(history))
        .route("/services/{id}/logs", get(super::logs))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::sessions::identify,
        ))
        .merge(
            Router::new()
                .route("/environments/{id}/services", post(create))
                .route("/services/{id}/configuration", post(save))
                .route("/services/{id}/deploy", post(deploy))
                .route("/services/{id}", axum::routing::delete(removal::remove))
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::sessions::protect,
                )),
        )
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct DatabaseConfig {
    pub database_name: String,
    pub username: String,
    pub password: String,
}
impl DatabaseConfig {
    fn validate(&self) -> Result<(), ApiError> {
        for value in [&self.database_name, &self.username] {
            if value.is_empty()
                || value.len() > 63
                || !value.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
            {
                return Err(invalid(
                    "Database and username must use 1–63 letters, digits or underscores, starting with a letter or underscore.",
                ));
            }
        }
        if self.password.len() < 16
            || self.password.len() > 128
            || self.password.chars().any(|c| c.is_control())
        {
            return Err(invalid(
                "Use a database password of 16–128 bytes without control characters.",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ComposeConfig {
    pub yaml: String,
}
// A distinct authenticated context prevents swapping service config and environment ciphertexts.
pub(super) fn seal<T: Serialize>(
    state: &AppState,
    id: Uuid,
    config: &T,
) -> Result<Vec<u8>, ApiError> {
    let plain = serde_json::to_vec(config).map_err(|_| internal())?;
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let aad = format!("ferrix:service:{id}");
    let encrypted = XChaCha20Poly1305::new((&state.auth.key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    Ok([nonce.to_vec(), encrypted].concat())
}
pub(super) fn open<T: DeserializeOwned>(
    state: &AppState,
    id: Uuid,
    cipher: &[u8],
) -> Result<T, ApiError> {
    if cipher.len() < 40 {
        return Err(internal());
    }
    let aad = format!("ferrix:service:{id}");
    let plain = XChaCha20Poly1305::new((&state.auth.key).into())
        .decrypt(
            XNonce::from_slice(&cipher[..24]),
            Payload {
                msg: &cipher[24..],
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    serde_json::from_slice(&plain).map_err(|_| internal())
}
fn default_database() -> String {
    "ferrix".into()
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NewService {
    Application {
        #[serde(flatten)]
        config: super::Configuration,
    },
    Database {
        name: String,
        #[serde(default = "default_database")]
        database_name: String,
        #[serde(default = "default_database")]
        username: String,
    },
    Compose {
        name: String,
        yaml: String,
        #[serde(default)]
        environment: String,
    },
}
async fn list(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
) -> Result<Json<Vec<Application>>, ApiError> {
    environment_access(&state, &id, environment).await?;
    Ok(Json(sqlx::query_as(&format!("SELECT {APP_COLUMNS} FROM applications a WHERE environment_id=$1 ORDER BY created_at DESC,id")).bind(environment).fetch_all(&state.pool).await?))
}
async fn create(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
    Json(input): Json<NewService>,
) -> Result<(StatusCode, Json<Application>), ApiError> {
    id.manager()?;
    environment_access(&state, &id, environment).await?;
    let service = Uuid::new_v4();
    let (kind, label, image, port, cipher, env) = match input {
        NewService::Application { config } => {
            return create_application(State(state), id, Path(environment), Json(config)).await;
        }
        NewService::Database {
            name: label,
            database_name,
            username,
        } => {
            let mut random = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut random);
            let config = DatabaseConfig {
                database_name,
                username,
                password: hex::encode(random),
            };
            config.validate()?;
            (
                "database",
                name(&label)?,
                "postgres:17-alpine",
                5432,
                seal(&state, service, &config)?,
                None,
            )
        }
        NewService::Compose {
            name: label,
            yaml,
            environment,
        } => {
            id.admin()?;
            compose::validate(&yaml)?;
            let values = parse_environment(&environment)?;
            (
                "compose",
                name(&label)?,
                "compose",
                80,
                seal(&state, service, &ComposeConfig { yaml })?,
                encrypt(&state, service, &values)?,
            )
        }
    };
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind,service_cipher,env_cipher) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(service).bind(environment).bind(label).bind(image).bind(port).bind(kind).bind(cipher).bind(env).execute(&state.pool).await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(app_access(&state, &id, service).await?),
    ))
}
async fn cipher(state: &AppState, service: Uuid) -> Result<Vec<u8>, ApiError> {
    sqlx::query_scalar::<_, Option<Vec<u8>>>("SELECT service_cipher FROM applications WHERE id=$1")
        .bind(service)
        .fetch_one(&state.pool)
        .await?
        .ok_or_else(internal)
}
async fn configuration(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    let app = app_access(&state, &id, service).await?;
    match app.kind.as_str() {
        "application" => super::configuration(State(state), id, Path(service)).await,
        "database" => {
            let config: DatabaseConfig = open(&state, service, &cipher(&state, service).await?)?;
            let locked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1)",
            )
            .bind(service)
            .fetch_one(&state.pool)
            .await?;
            let host = format!("ferrix-{}", service.simple());
            let mut url = url::Url::parse("postgresql://localhost:5432").map_err(|_| internal())?;
            url.set_host(Some(&host)).map_err(|_| internal())?;
            url.set_path(&config.database_name);
            url.set_username(&config.username).map_err(|_| internal())?;
            url.set_password(Some(&config.password))
                .map_err(|_| internal())?;
            Ok(Json(
                json!({"kind":"database","name":app.name,"database_name":config.database_name,"username":config.username,"password":config.password,"connection_uri":url.as_str(),"credentials_locked":locked}),
            ))
        }
        "compose" => {
            id.admin()?;
            let config: ComposeConfig = open(&state, service, &cipher(&state, service).await?)?;
            let env: Option<Vec<u8>> =
                sqlx::query_scalar("SELECT env_cipher FROM applications WHERE id=$1")
                    .bind(service)
                    .fetch_one(&state.pool)
                    .await?;
            Ok(Json(
                json!({"kind":"compose","name":app.name,"yaml":config.yaml,"environment":decrypt(&state,service,env.as_deref())?.join("\n")}),
            ))
        }
        _ => Err(missing()),
    }
}
#[derive(Deserialize)]
struct DatabaseEdit {
    name: String,
    #[serde(flatten)]
    config: DatabaseConfig,
}
#[derive(Deserialize)]
struct ComposeEdit {
    name: String,
    yaml: String,
    #[serde(default)]
    environment: String,
}
async fn save(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(value): Json<Value>,
) -> Result<Json<Application>, ApiError> {
    id.manager()?;
    let app = app_access(&state, &id, service).await?;
    match app.kind.as_str() {
        "application" => {
            return super::save_configuration(
                State(state),
                id,
                Path(service),
                Json(
                    serde_json::from_value(value)
                        .map_err(|_| invalid("Invalid application configuration."))?,
                ),
            )
            .await;
        }
        "database" => {
            let edit: DatabaseEdit = serde_json::from_value(value)
                .map_err(|_| invalid("Invalid database configuration."))?;
            edit.config.validate()?;
            let mut tx = state.pool.begin().await?;
            let old: Vec<u8> = sqlx::query_scalar(
                "SELECT service_cipher FROM applications WHERE id=$1 FOR UPDATE",
            )
            .bind(service)
            .fetch_one(&mut *tx)
            .await?;
            let locked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM deployments WHERE application_id=$1)",
            )
            .bind(service)
            .fetch_one(&mut *tx)
            .await?;
            if locked && open::<DatabaseConfig>(&state, service, &old)? != edit.config {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "Database credentials are locked after the first deployment is queued.",
                ));
            }
            sqlx::query(
                "UPDATE applications SET name=$1,service_cipher=$2,updated_at=now() WHERE id=$3",
            )
            .bind(name(&edit.name)?)
            .bind(seal(&state, service, &edit.config)?)
            .bind(service)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            tx.commit().await?;
        }
        "compose" => {
            id.admin()?;
            let edit: ComposeEdit = serde_json::from_value(value)
                .map_err(|_| invalid("Invalid Compose configuration."))?;
            compose::validate(&edit.yaml)?;
            let values = parse_environment(&edit.environment)?;
            sqlx::query("UPDATE applications SET name=$1,service_cipher=$2,env_cipher=$3,updated_at=now() WHERE id=$4").bind(name(&edit.name)?).bind(seal(&state,service,&ComposeConfig{yaml:edit.yaml})?).bind(encrypt(&state,service,&values)?).bind(service).execute(&state.pool).await.map_err(database_error)?;
        }
        _ => return Err(missing()),
    }
    Ok(Json(app_access(&state, &id, service).await?))
}
pub(super) fn labels(
    app: Uuid,
    deployment: Uuid,
    organization: Uuid,
    project: Uuid,
    environment: Uuid,
) -> HashMap<String, String> {
    HashMap::from([
        ("io.ferrix.managed".into(), "true".into()),
        ("io.ferrix.application".into(), app.to_string()),
        ("io.ferrix.deployment".into(), deployment.to_string()),
        ("io.ferrix.organization".into(), organization.to_string()),
        ("io.ferrix.project".into(), project.to_string()),
        ("io.ferrix.environment".into(), environment.to_string()),
    ])
}
