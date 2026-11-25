use super::*;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
mod control;
pub(super) mod database;
mod notes;
mod removal;
pub(super) mod replacement;
mod transfer;

pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/environments/{id}/services", get(list))
        .route("/environments/{id}/service-status", get(control::statuses))
        .route("/services/{id}/environment", get(control::environment))
        .route("/services/{id}", get(application))
        .route("/services/{id}/configuration", get(configuration))
        .route("/services/{id}/runtime", get(super::runtime))
        .route("/services/{id}/notes", get(notes::read))
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
                .route("/services/{id}/notes", post(notes::save))
                .route("/services/{id}/move", post(transfer::move_service))
                .route("/services/{id}/clone", post(transfer::clone_service))
                .route("/services/{id}/deploy", post(deploy))
                .route("/services/{id}/rollback", post(rollback))
                .route("/services/{id}/control", post(control::change))
                .route(
                    "/services/{id}/environment",
                    post(control::save_environment),
                )
                .route("/services/{id}", axum::routing::delete(removal::remove))
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::sessions::protect,
                )),
        )
}
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DatabaseEngine {
    #[default]
    Postgres,
    Mongodb,
    Mariadb,
    Redis,
}
impl DatabaseEngine {
    pub(super) fn image(self) -> &'static str {
        match self {
            Self::Postgres => "postgres:17-alpine",
            Self::Mongodb => "mongo:8",
            Self::Mariadb => "mariadb:11.4",
            Self::Redis => "redis:8-alpine",
        }
    }
    pub(super) fn port(self) -> i32 {
        match self {
            Self::Postgres => 5432,
            Self::Mongodb => 27017,
            Self::Mariadb => 3306,
            Self::Redis => 6379,
        }
    }
    pub(super) fn scheme(self) -> &'static str {
        match self {
            Self::Postgres => "postgresql",
            Self::Mongodb => "mongodb",
            Self::Mariadb => "mysql",
            Self::Redis => "redis",
        }
    }
    pub(super) fn reserved(self, key: &str) -> bool {
        match self {
            Self::Postgres => key.starts_with("POSTGRES_") || key.starts_with("PG"),
            Self::Mongodb => key.starts_with("MONGO"),
            Self::Mariadb => key.starts_with("MARIADB_") || key.starts_with("MYSQL_"),
            Self::Redis => key.starts_with("REDIS"),
        }
    }
}
pub(super) fn random_password() -> String {
    let mut random = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random);
    hex::encode(random)
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct DatabaseConfig {
    #[serde(default)]
    pub engine: DatabaseEngine,
    #[serde(default)]
    pub root_password: Option<String>,
    pub database_name: String,
    pub username: String,
    pub password: String,
}
impl DatabaseConfig {
    pub(super) fn validate(&self) -> Result<(), ApiError> {
        if self.engine == DatabaseEngine::Redis {
            if self.database_name != "0" || self.username != "default" {
                return Err(invalid(
                    "Redis uses database 0 and the default authenticated user.",
                ));
            }
        } else {
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
        }
        if self.engine == DatabaseEngine::Mariadb
            && (self.username.eq_ignore_ascii_case("root") || self.username.len() > 32)
        {
            return Err(invalid(
                "MariaDB application user must not be root and must use at most 32 characters.",
            ));
        }
        if self.engine == DatabaseEngine::Mariadb
            && !self.root_password.as_ref().is_some_and(|v| {
                v.len() >= 16 && v.len() <= 128 && !v.chars().any(char::is_control)
            })
        {
            return Err(invalid(
                "MariaDB requires a root password of 16–128 bytes without control characters.",
            ));
        }
        if self.engine != DatabaseEngine::Mariadb && self.root_password.is_some() {
            return Err(invalid(
                "A separate root password is only supported for MariaDB.",
            ));
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
        #[serde(default)]
        engine: DatabaseEngine,
        #[serde(default = "default_database")]
        database_name: String,
        #[serde(default = "default_database")]
        username: String,
    },
}
async fn list(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
) -> Result<Json<Vec<Application>>, ApiError> {
    environment_access(&state, &id, environment).await?;
    let rows:Vec<Application>=sqlx::query_as(&format!("SELECT {APP_COLUMNS} FROM applications a WHERE environment_id=$1 AND service_kind IN ('application','database') AND ferrix_service_permission($2,$3,a.id) IS NOT NULL ORDER BY created_at DESC,id")).bind(environment).bind(id.user_id).bind(id.organization_id).fetch_all(&state.pool).await?;
    Ok(Json(
        rows.into_iter()
            .map(|app| app.with_gateway(&state))
            .collect(),
    ))
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
    let (kind, label, image, port, cipher, env): (_, _, _, _, _, Option<Vec<u8>>) = match input {
        NewService::Application { config } => {
            return create_application(State(state), id, Path(environment), Json(config)).await;
        }
        NewService::Database {
            name: label,
            engine,
            database_name,
            username,
        } => {
            let config = DatabaseConfig {
                engine,
                database_name: if engine == DatabaseEngine::Redis {
                    "0".into()
                } else {
                    database_name
                },
                username: if engine == DatabaseEngine::Redis {
                    "default".into()
                } else {
                    username
                },
                password: random_password(),
                root_password: (engine == DatabaseEngine::Mariadb).then(random_password),
            };
            config.validate()?;
            (
                "database",
                name(&label)?,
                engine.image(),
                engine.port(),
                seal(&state, service, &config)?,
                None,
            )
        }
    };
    let mut tx = state.pool.begin().await?;
    let reserved = quotas::reserve(&state, &mut tx).await?;
    sqlx::query("INSERT INTO applications(id,environment_id,name,image,container_port,service_kind,service_cipher,env_cipher) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(service).bind(environment).bind(label).bind(image).bind(port).bind(kind).bind(cipher).bind(env).execute(&mut *tx).await.map_err(database_error)?;
    quotas::check(&state, &mut tx, reserved).await?;
    tx.commit().await?;
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
    crate::auth::permissions::developer(&state, &id, service).await?;
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
            let mut url = url::Url::parse(&format!(
                "{}://localhost:{}",
                config.engine.scheme(),
                config.engine.port()
            ))
            .map_err(|_| internal())?;
            url.set_host(Some(&host)).map_err(|_| internal())?;
            url.set_path(&config.database_name);
            url.set_username(&config.username).map_err(|_| internal())?;
            url.set_password(Some(&config.password))
                .map_err(|_| internal())?;
            if config.engine == DatabaseEngine::Mongodb {
                url.query_pairs_mut().append_pair("authSource", "admin");
            }
            Ok(Json(
                json!({"kind":"database","name":app.name,"engine":config.engine,"root_password":config.root_password,"database_name":config.database_name,"username":config.username,"password":config.password,"connection_uri":url.as_str(),"credentials_locked":locked}),
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
async fn save(
    State(state): State<AppState>,
    id: Identity,
    Path(service): Path<Uuid>,
    Json(value): Json<Value>,
) -> Result<Json<Application>, ApiError> {
    crate::auth::permissions::developer(&state, &id, service).await?;
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
            let previous = open::<DatabaseConfig>(&state, service, &old)?;
            if previous.engine != edit.config.engine {
                return Err(invalid(
                    "Database engine cannot be changed. Create a new service instead.",
                ));
            }
            if locked && previous != edit.config {
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
        _ => return Err(missing()),
    }
    Ok(Json(app_access(&state, &id, service).await?))
}
