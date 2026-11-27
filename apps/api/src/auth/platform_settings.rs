//! Installation-scoped configuration. OAuth remains inactive.
use super::*;
use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn invalid() -> ApiError {
    ApiError(
        StatusCode::UNPROCESSABLE_ENTITY,
        "Check the setting values and try again.",
    )
}
fn crypt(
    state: &AppState,
    section: &str,
    bytes: &[u8],
    encrypt: bool,
) -> Result<Vec<u8>, ApiError> {
    let aad = format!("ferrix:platform-settings:{section}");
    let cipher = XChaCha20Poly1305::new((&state.auth.key).into());
    if encrypt {
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let body = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| internal())?;
        Ok([nonce.to_vec(), body].concat())
    } else {
        if bytes.len() < 40 {
            return Err(internal());
        }
        cipher
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| internal())
    }
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Timezone {
    timezone: String,
}
fn smtp_provider() -> String {
    "smtp".to_owned()
}
fn smtp_port() -> u16 {
    587
}
fn smtp_security() -> String {
    "starttls".to_owned()
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Email {
    #[serde(default = "smtp_provider")]
    pub(super) provider: String,
    #[serde(default)]
    pub(super) host: String,
    #[serde(default = "smtp_port")]
    pub(super) port: u16,
    #[serde(default = "smtp_security")]
    pub(super) security: String,
    #[serde(default)]
    pub(super) username: String,
    #[serde(default)]
    pub(super) from_address: String,
    #[serde(default)]
    pub(super) from_name: String,
    #[serde(default)]
    pub(super) password: Option<String>,
    #[serde(default)]
    pub(super) resend_api_key: Option<String>,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Social {
    google_client_id: String,
    github_client_id: String,
    #[serde(default)]
    google_client_secret: Option<String>,
    #[serde(default)]
    github_client_secret: Option<String>,
}
fn defaults(section: &str) -> Result<Value, ApiError> {
    match section {
        "timezone" => Ok(json!({"timezone":""})),
        "email" => Ok(
            json!({"provider":"smtp","resend_api_key":null,"host":"","port":587,"security":"starttls","username":"","from_address":"","from_name":"","password":null}),
        ),
        "social" => Ok(
            json!({"google_client_id":"","github_client_id":"","google_client_secret":null,"github_client_secret":null}),
        ),
        _ => Err(ApiError(StatusCode::NOT_FOUND, "Unknown settings section.")),
    }
}
fn decode(state: &AppState, section: &str, bytes: Option<Vec<u8>>) -> Result<Value, ApiError> {
    bytes
        .map(|bytes| {
            crypt(state, section, &bytes, false)
                .and_then(|plain| serde_json::from_slice(&plain).map_err(|_| internal()))
        })
        .unwrap_or_else(|| defaults(section))
}
async fn read(state: &AppState, section: &str) -> Result<(i64, Value), ApiError> {
    let row: Option<(i64, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT revision,config_cipher FROM platform_settings WHERE section=$1")
            .bind(section)
            .fetch_optional(&state.pool)
            .await?;
    let (revision, bytes) = row.unwrap_or((0, None));
    Ok((revision, decode(state, section, bytes)?))
}
pub(super) async fn email(state: &AppState) -> Result<Email, ApiError> {
    let (_, value) = read(state, "email").await?;
    serde_json::from_value(value).map_err(|_| internal())
}
pub(crate) async fn timezone(state: &AppState) -> Result<Option<chrono_tz::Tz>, ApiError> {
    let (_, value) = read(state, "timezone").await?;
    let name = value["timezone"].as_str().unwrap_or("");
    if name.is_empty() {
        Ok(None)
    } else {
        name.parse().map(Some).map_err(|_| internal())
    }
}
fn public(mut value: Value, revision: i64) -> Value {
    for key in [
        "password",
        "resend_api_key",
        "google_client_secret",
        "github_client_secret",
    ] {
        if let Some(secret) = value.as_object_mut().unwrap().remove(key) {
            value[format!("{key}_configured")] =
                json!(secret.as_str().is_some_and(|v| !v.is_empty()));
        }
    }
    value["revision"] = json!(revision);
    value
}
pub(crate) async fn get(
    State(state): State<AppState>,
    id: Identity,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    id.admin()?;
    let mut result = json!({"email_delivery_available":true,"social_login_available":false,"timezones":chrono_tz::TZ_VARIANTS.iter().map(|zone|zone.name()).collect::<Vec<_>>()});
    for section in ["timezone", "email", "social"] {
        let (revision, value) = read(&state, section).await?;
        result[section] = public(value, revision);
    }
    result["timezone"]["effective_timezone"] = json!(
        timezone(&state)
            .await?
            .map(|zone| zone.name().to_owned())
            .unwrap_or_else(|| crate::server_time::system_timezone().name().to_owned())
    );
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(result)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Update {
    revision: i64,
    values: Value,
}
fn text(value: &str, max: usize) -> bool {
    value.chars().count() <= max && !value.chars().any(char::is_control)
}
fn secret(value: &mut Option<String>, previous: &Value, key: &str) -> Result<(), ApiError> {
    if value.is_none() {
        *value = previous[key].as_str().map(str::to_owned);
    }
    if value.as_deref() == Some("") {
        *value = None;
    }
    if value.as_ref().is_some_and(|v| !text(v, 4096)) {
        return Err(invalid());
    }
    Ok(())
}
fn prepare(section: &str, values: Value, previous: &Value) -> Result<Value, ApiError> {
    match section {
        "timezone" => {
            let config: Timezone = serde_json::from_value(values).map_err(|_| invalid())?;
            if !config.timezone.is_empty() && config.timezone.parse::<chrono_tz::Tz>().is_err() {
                return Err(invalid());
            }
            Ok(json!(config))
        }
        "email" => {
            let mut config: Email = serde_json::from_value(values).map_err(|_| invalid())?;
            if !matches!(config.provider.as_str(), "smtp" | "google" | "resend") {
                return Err(invalid());
            }
            if !text(&config.host, 253)
                || !text(&config.username, 320)
                || !text(&config.from_name, 120)
                || config.port == 0
                || !matches!(config.security.as_str(), "starttls" | "tls")
            {
                return Err(invalid());
            }
            if !config.host.is_empty()
                && (config.host.contains(['/', '@', ':', ' '])
                    || url::Host::parse(&config.host).is_err())
            {
                return Err(invalid());
            }
            if !config.from_address.is_empty() {
                config.from_address = accounts::email(&config.from_address)?;
            }
            secret(&mut config.password, previous, "password")?;
            secret(&mut config.resend_api_key, previous, "resend_api_key")?;
            if config.provider == "google" {
                config.host = "smtp.gmail.com".to_owned();
                config.port = 587;
                config.security = "starttls".to_owned();
                config.username = config.from_address.clone();
            }
            Ok(json!(config))
        }
        "social" => {
            let mut config: Social = serde_json::from_value(values).map_err(|_| invalid())?;
            if !text(&config.google_client_id, 2048) || !text(&config.github_client_id, 2048) {
                return Err(invalid());
            }
            secret(
                &mut config.google_client_secret,
                previous,
                "google_client_secret",
            )?;
            secret(
                &mut config.github_client_secret,
                previous,
                "github_client_secret",
            )?;
            Ok(json!(config))
        }
        _ => Err(ApiError(StatusCode::NOT_FOUND, "Unknown settings section.")),
    }
}
pub(crate) async fn save(
    State(state): State<AppState>,
    id: Identity,
    Path(section): Path<String>,
    Json(input): Json<Update>,
) -> Result<StatusCode, ApiError> {
    id.admin()?;
    id.fresh()?;
    defaults(&section)?;
    let mut tx = state.pool.begin().await?;
    management::authorize(&mut tx, &id, "platform").await?;
    sqlx::query("INSERT INTO platform_settings(section) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(&section)
        .execute(&mut *tx)
        .await?;
    let (revision, bytes): (i64, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT revision,config_cipher FROM platform_settings WHERE section=$1 FOR UPDATE",
    )
    .bind(&section)
    .fetch_one(&mut *tx)
    .await?;
    if input.revision != revision {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Settings changed. Refresh before saving again.",
        ));
    }
    let previous = decode(&state, &section, bytes)?;
    let config = prepare(&section, input.values, &previous)?;
    let bytes = crypt(
        &state,
        &section,
        &serde_json::to_vec(&config).map_err(|_| internal())?,
        true,
    )?;
    sqlx::query("UPDATE platform_settings SET revision=revision+1,config_cipher=$2,updated_at=now() WHERE section=$1").bind(&section).bind(bytes).execute(&mut *tx).await?;
    log(
        &mut tx,
        id.user_id,
        id.organization_id,
        &format!("platform.settings.{section}_updated"),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_configuration_and_preserves_or_clears_secrets() {
        let before = json!({"password":"stored-secret"});
        let mut input = defaults("email").unwrap();
        input.as_object_mut().unwrap().remove("password");
        let stored = prepare("email", input.clone(), &before).unwrap();
        assert_eq!(stored["password"], "stored-secret");
        let redacted = public(stored, 1);
        assert_eq!(redacted["password_configured"], true);
        assert!(redacted.get("password").is_none());
        input["password"] = json!("");
        assert!(prepare("email", input.clone(), &before).unwrap()["password"].is_null());
        input["host"] = json!("smtp.example.com\r\nInjected: x");
        assert!(prepare("email", input, &before).is_err());
        assert!(prepare("timezone", json!({"timezone":"invalid"}), &json!({})).is_err());
        assert!(
            prepare(
                "timezone",
                json!({"timezone":"Asia/Kuala_Lumpur"}),
                &json!({})
            )
            .is_ok()
        );
    }
}
