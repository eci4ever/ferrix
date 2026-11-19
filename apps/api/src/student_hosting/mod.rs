mod runtime;
use crate::{ApiError, AppState, auth::Identity};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand::RngCore;
pub use runtime::run_worker;
pub(crate) use runtime::upstream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

fn invalid(message: &'static str) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, message)
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Hosting operation failed. Check server status and retry.",
    )
}
fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Student hosting not found.")
}
pub(crate) fn secret() -> String {
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    hex::encode(b)
}
fn seal(state: &AppState, id: Uuid, plain: &str) -> Result<Vec<u8>, ApiError> {
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new((&state.auth.key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain.as_bytes(),
                aad: format!("ferrix:hosting:{id}").as_bytes(),
            },
        )
        .map_err(|_| unavailable())?;
    Ok([nonce.to_vec(), cipher].concat())
}
fn open(state: &AppState, id: Uuid, cipher: &[u8]) -> Result<String, ApiError> {
    if cipher.len() < 40 {
        return Err(unavailable());
    }
    let plain = XChaCha20Poly1305::new((&state.auth.key).into())
        .decrypt(
            XNonce::from_slice(&cipher[..24]),
            Payload {
                msg: &cipher[24..],
                aad: format!("ferrix:hosting:{id}").as_bytes(),
            },
        )
        .map_err(|_| unavailable())?;
    String::from_utf8(plain).map_err(|_| unavailable())
}
#[derive(sqlx::FromRow, Serialize)]
pub(crate) struct Hosting {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub status: String,
    pub database_name: String,
    pub database_user: String,
    #[serde(skip)]
    pub password_cipher: Vec<u8>,
    pub container_id: Option<String>,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
const SELECT: &str = "SELECT h.user_id,u.name,u.email,h.status,h.database_name,h.database_user,h.password_cipher,h.container_id,h.error,h.created_at FROM student_hostings h JOIN auth_users u ON u.id=h.user_id";
pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/student-hosting", get(list))
        .route("/student-hosting/settings", get(settings).post(configure))
        .route("/student-hosting/{id}", get(details))
        .route("/student-hosting/{id}/approve", post(approve))
        .route("/student-hosting/{id}/suspend", post(suspend))
        .route("/student-hosting/{id}/reject", post(reject))
        .route("/student-hosting/{id}/upload", post(upload))
        .layer(DefaultBodyLimit::max(18 * 1024 * 1024))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::sessions::protect,
        ))
}
#[derive(Deserialize)]
pub(crate) struct Registration {
    name: String,
    email: String,
    password: String,
}
pub(crate) async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Registration>,
) -> Result<StatusCode, ApiError> {
    let enabled: bool = sqlx::query_scalar(
        "SELECT enabled AND registration_open FROM student_hosting_settings WHERE singleton",
    )
    .fetch_one(&state.pool)
    .await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Student registration is closed.",
        ));
    }
    crate::auth::accounts::throttle(
        &state.pool,
        &format!(
            "student-register:{}",
            headers
                .get("x-ferrix-peer")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("local")
        ),
        60,
    )
    .await?;
    let name = crate::auth::accounts::name(&input.name)?;
    let email = crate::auth::accounts::email(&input.email)?;
    let hash = crate::auth::accounts::hash(input.password).await?;
    let user = Uuid::new_v4();
    let org = Uuid::new_v4();
    let short = &user.simple().to_string()[..24];
    let cipher = seal(&state, user, &secret())?;
    let mut tx = state.pool.begin().await?;
    let inserted=sqlx::query("INSERT INTO auth_users(id,name,email,password_hash) VALUES($1,$2,$3,$4) ON CONFLICT(email) DO NOTHING").bind(user).bind(&name).bind(email).bind(hash).execute(&mut *tx).await?.rows_affected();
    // Same response for existing emails; never turn an existing platform user into a student.
    if inserted == 1 {
        sqlx::query("INSERT INTO auth_organizations(id,name) VALUES($1,$2)")
            .bind(org)
            .bind(
                format!("Hosting: {name}")
                    .chars()
                    .take(80)
                    .collect::<String>(),
            )
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO auth_members(organization_id,user_id,role) VALUES($1,$2,'member')",
        )
        .bind(org)
        .bind(user)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO student_hostings(user_id,organization_id,database_name,database_user,password_cipher) VALUES($1,$2,$3,$4,$5)").bind(user).bind(org).bind(format!("student_{short}")).bind(format!("s_{short}")).bind(cipher).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::ACCEPTED)
}
pub(crate) async fn public_status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let open: bool = sqlx::query_scalar(
        "SELECT enabled AND registration_open FROM student_hosting_settings WHERE singleton",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"registration_open":open})))
}
async fn access(state: &AppState, id: &Identity, user: Uuid) -> Result<Hosting, ApiError> {
    if id.system_role != "admin" && (!id.hosting_student || id.user_id != user) {
        return Err(missing());
    }
    sqlx::query_as(&format!("{SELECT} WHERE h.user_id=$1"))
        .bind(user)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(missing)
}
fn metadata(state: &AppState, h: &Hosting) -> Value {
    let mut v = serde_json::to_value(h).unwrap();
    v["automatic_url"] = json!(
        state
            .gateway
            .url(crate::gateway::Resource::Student(h.user_id))
    );
    v
}
async fn list(State(state): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    let rows: Vec<Hosting> = if id.system_role == "admin" {
        sqlx::query_as(&format!("{SELECT} ORDER BY h.created_at DESC LIMIT 500"))
            .fetch_all(&state.pool)
            .await?
    } else {
        vec![access(&state, &id, id.user_id).await?]
    };
    Ok(Json(json!(
        rows.iter().map(|h| metadata(&state, h)).collect::<Vec<_>>()
    )))
}
async fn details(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let h = access(&state, &id, user).await?;
    let mut v = metadata(&state, &h);
    if h.status == "active" {
        v["database_password"] = json!(open(&state, user, &h.password_cipher)?);
        v["database_host"] = json!("mariadb");
        v["database_port"] = json!(3306);
    }
    Ok(Json(v))
}
async fn settings(State(state): State<AppState>, id: Identity) -> Result<Json<Value>, ApiError> {
    id.admin()?;
    let row:(bool,bool,String,Option<String>)=sqlx::query_as("SELECT enabled,registration_open,server_state,server_error FROM student_hosting_settings WHERE singleton").fetch_one(&state.pool).await?;
    Ok(Json(
        json!({"enabled":row.0,"registration_open":row.1,"server_state":row.2,"error":row.3}),
    ))
}
#[derive(Deserialize)]
struct Settings {
    registration_open: bool,
}
async fn configure(
    State(state): State<AppState>,
    id: Identity,
    Json(input): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    sqlx::query("UPDATE student_hosting_settings SET enabled=true,registration_open=$1,updated_at=now() WHERE singleton").bind(input.registration_open).execute(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn approve(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    access(&state, &id, user).await?;
    let n=sqlx::query("UPDATE student_hostings SET status='queued',error=NULL,approved_by=$1,approved_at=now() WHERE user_id=$2 AND status IN ('pending','failed','suspended')").bind(id.user_id).bind(user).execute(&state.pool).await?.rows_affected();
    if n == 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "This hosting is already active, rejected or being provisioned.",
        ));
    }
    Ok(StatusCode::ACCEPTED)
}
async fn reject(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    let n = sqlx::query(
        "UPDATE student_hostings SET status='rejected' WHERE user_id=$1 AND status='pending'",
    )
    .bind(user)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Only pending registrations can be rejected.",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn suspend(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    let mut tx = state.pool.begin().await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM student_hostings WHERE user_id=$1 FOR UPDATE")
            .bind(user)
            .fetch_optional(&mut *tx)
            .await?;
    if status.as_deref() != Some("active") {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Only active hosting can be suspended.",
        ));
    }
    runtime::suspend(&state, user).await?;
    sqlx::query("UPDATE student_hostings SET status='suspended' WHERE user_id=$1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct Upload {
    files: Vec<File>,
}
#[derive(Deserialize)]
pub(super) struct File {
    path: String,
    hex: String,
}
async fn upload(
    State(state): State<AppState>,
    id: Identity,
    Path(user): Path<Uuid>,
    Json(input): Json<Upload>,
) -> Result<StatusCode, ApiError> {
    let h = access(&state, &id, user).await?;
    if h.status != "active" {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Hosting must be active before uploading files.",
        ));
    }
    let files = validate_files(input.files)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        runtime::upload(&state, &h, files),
    )
    .await
    .map_err(|_| unavailable())??;
    Ok(StatusCode::NO_CONTENT)
}
fn validate_files(files: Vec<File>) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    if files.is_empty() || files.len() > 256 {
        return Err(invalid("Upload 1–256 files, up to 8 MiB total."));
    }
    let mut total = 0;
    let mut result = Vec::new();
    let mut paths = std::collections::HashSet::new();
    for f in files {
        if f.path.len() > 100
            || f.path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".." || p.starts_with('.'))
            || !f
                .path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b))
            || !paths.insert(f.path.clone())
        {
            return Err(invalid(
                "Use unique relative file paths without hidden files or parent directories.",
            ));
        }
        if f.hex.len() > 16 * 1024 * 1024 {
            return Err(invalid("Upload exceeds 8 MiB."));
        }
        let data = hex::decode(f.hex).map_err(|_| invalid("Invalid file data."))?;
        total += data.len();
        if total > 8 * 1024 * 1024 {
            return Err(invalid("Upload exceeds 8 MiB."));
        }
        result.push((f.path, data));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uploads_reject_traversal_hidden_paths_duplicates_and_invalid_content() {
        for path in [
            "../index.php",
            "/index.php",
            "a/../../b.php",
            ".env",
            "a/.config",
            "a\\b.php",
            "",
            "a//b.php",
        ] {
            assert!(
                validate_files(vec![File {
                    path: path.into(),
                    hex: "00".into()
                }])
                .is_err()
            );
        }
        assert!(
            validate_files(vec![File {
                path: "index.php".into(),
                hex: "not-hex".into()
            }])
            .is_err()
        );
        assert!(
            validate_files(vec![
                File {
                    path: "index.php".into(),
                    hex: "00".into()
                },
                File {
                    path: "index.php".into(),
                    hex: "01".into()
                }
            ])
            .is_err()
        );
        let files = validate_files(vec![File {
            path: "nested/index.php".into(),
            hex: hex::encode("<?php echo 'ok';"),
        }])
        .unwrap();
        assert_eq!(files[0].0, "nested/index.php");
    }
    #[tokio::test]
    async fn database_secrets_cannot_be_swapped_between_students_or_tampered() {
        let state = AppState {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
                .unwrap(),
            docker: bollard::Docker::connect_with_local_defaults().unwrap(),
            gateway: Default::default(),
            auth: std::sync::Arc::new(crate::auth::AuthConfig {
                origin: "http://127.0.0.1:5173".into(),
                secure: false,
                cookie_name: "test".into(),
                key: [7; 32],
            }),
        };
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        let mut cipher = seal(&state, alice, "private-database-password").unwrap();
        assert_eq!(
            open(&state, alice, &cipher).unwrap(),
            "private-database-password"
        );
        assert!(open(&state, bob, &cipher).is_err());
        assert!(open(&state, Uuid::nil(), &cipher).is_err());
        cipher[24] ^= 1;
        assert!(open(&state, alice, &cipher).is_err());
    }
}
