//! Bounded, post-commit invitation delivery. Tokens remain usable on provider failure.
use super::{AppState, platform_settings};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
use serde::Serialize;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(15);
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Delivery {
    Sent,
    Failed,
    NotConfigured,
}

impl platform_settings::Email {
    pub(super) fn ready(&self) -> bool {
        !self.from_address.is_empty()
            && match self.provider.as_str() {
                "resend" => self
                    .resend_api_key
                    .as_ref()
                    .is_some_and(|s| !s.trim().is_empty()),
                "google" | "smtp" => {
                    !self.host.is_empty()
                        && !self.username.is_empty()
                        && self.password.as_ref().is_some_and(|s| !s.trim().is_empty())
                }
                _ => false,
            }
    }
    fn sender(&self) -> Result<Mailbox, ()> {
        Ok(Mailbox::new(
            (!self.from_name.is_empty()).then(|| self.from_name.clone()),
            self.from_address.parse().map_err(|_| ())?,
        ))
    }
    fn smtp(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>, ()> {
        // Google cannot be redirected by historical or stale SMTP values.
        let (host, port, security, username) = if self.provider == "google" {
            (
                "smtp.gmail.com",
                587,
                "starttls",
                self.from_address.as_str(),
            )
        } else {
            (
                self.host.as_str(),
                self.port,
                self.security.as_str(),
                self.username.as_str(),
            )
        };
        let builder = match security {
            "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host),
            "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(host),
            _ => return Err(()),
        }
        .map_err(|_| ())?;
        Ok(builder
            .port(port)
            .timeout(Some(TIMEOUT))
            .credentials(Credentials::new(
                username.to_owned(),
                self.password.clone().ok_or(())?,
            ))
            .build())
    }
}

fn body(origin: &str, organization: &str, role: &str, token: &str) -> String {
    format!(
        "You have been invited to join {organization} on Ferrix as {role}.\n\nOpen {origin}/signup and enter this invitation token with the email address that received this message:\n\n{token}\n\nThis invitation expires in two days. If you already have a Ferrix account, use your current password. If you did not expect this invitation, you can ignore this email.\n"
    )
}

async fn resend(
    config: &platform_settings::Email,
    endpoint: &str,
    invite: Uuid,
    token: &str,
    recipient: &str,
    text: &str,
) -> Result<(), ()> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ())?;
    let payload = json!({"from":config.sender()?.to_string(),"to":[recipient],"subject":"Your Ferrix organization invitation","text":text});
    let response = client
        .post(endpoint)
        .bearer_auth(config.resend_api_key.as_ref().ok_or(())?)
        .header(
            "Idempotency-Key",
            format!("invitation-{invite}-{}", super::digest(token)),
        )
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .await
        .map_err(|_| ())?;
    // Do not read/log provider bodies: they may contain tokens, addresses or credentials.
    if response.status().is_success() {
        Ok(())
    } else {
        Err(())
    }
}

async fn send(
    state: &AppState,
    invite: Uuid,
    org: Uuid,
    recipient: &str,
    role: &str,
    token: &str,
) -> Result<Delivery, ()> {
    let config = platform_settings::email(state).await.map_err(|_| ())?;
    if !config.ready() {
        return Ok(Delivery::NotConfigured);
    }
    let organization: String =
        sqlx::query_scalar("SELECT name FROM auth_organizations WHERE id=$1")
            .bind(org)
            .fetch_one(&state.pool)
            .await
            .map_err(|_| ())?;
    let text = body(&state.auth.origin, &organization, role, token);
    if config.provider == "resend" {
        resend(
            &config,
            "https://api.resend.com/emails",
            invite,
            token,
            recipient,
            &text,
        )
        .await?;
    } else {
        let message = Message::builder()
            .from(config.sender()?)
            .to(recipient.parse().map_err(|_| ())?)
            .subject("Your Ferrix organization invitation")
            .body(text)
            .map_err(|_| ())?;
        config.smtp()?.send(message).await.map_err(|_| ())?;
    }
    Ok(Delivery::Sent)
}

pub(super) async fn deliver(
    state: &AppState,
    invite: Uuid,
    org: Uuid,
    recipient: &str,
    role: &str,
    token: &str,
) -> Delivery {
    let delivery = tokio::time::timeout(TIMEOUT, send(state, invite, org, recipient, role, token))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(Delivery::Failed);
    // Only a non-sensitive outcome is logged; never log provider errors or invitation tokens.
    tracing::info!(invitation_id=%invite, ?delivery, "Invitation email attempt completed");
    delivery
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn config(provider: &str) -> platform_settings::Email {
        serde_json::from_value(json!({"provider":provider,"host":"smtp.gmail.com","username":"sender@gmail.com","from_address":"sender@gmail.com","from_name":"Ferrix QA","password":"app-password","resend_api_key":"re_test_only"})).unwrap()
    }
    #[test]
    fn readiness_and_google_secure_transport() {
        let mut google = config("google");
        assert!(google.ready());
        google.host = "untrusted.example.com".into();
        google.port = 25;
        google.security = "none".into();
        assert!(google.smtp().is_ok()); // provider always pins Gmail STARTTLS
        google.password = None;
        assert!(!google.ready());
        let mut resend = config("resend");
        assert!(resend.ready());
        resend.resend_api_key = None;
        assert!(!resend.ready());
        let mut smtp = config("smtp");
        smtp.security = "none".into();
        assert!(smtp.smtp().is_err());
        let text = body(
            "https://ferrix.example.com",
            "Example organization",
            "member",
            "test-token",
        );
        assert!(text.contains("https://ferrix.example.com/signup"));
        assert!(text.contains("test-token"));
        assert!(!text.contains("?token="));
    }
    #[tokio::test]
    async fn smtp_requires_starttls_before_sending_credentials() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            socket
                .get_mut()
                .write_all(b"220 localhost ESMTP\r\n")
                .await
                .unwrap();
            let mut hello = String::new();
            socket.read_line(&mut hello).await.unwrap();
            assert!(hello.starts_with("EHLO "));
            // Advertising authentication without STARTTLS must never reveal credentials.
            socket
                .get_mut()
                .write_all(b"250-localhost\r\n250 AUTH PLAIN LOGIN\r\n")
                .await
                .unwrap();
            let mut remainder = String::new();
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                socket.read_to_string(&mut remainder),
            )
            .await;
            assert!(!remainder.contains("AUTH"));
            assert!(!remainder.contains("app-password"));
        });
        let mut config = config("smtp");
        config.host = "127.0.0.1".into();
        config.port = port;
        let message = Message::builder()
            .from(config.sender().unwrap())
            .to("invitee@example.com".parse().unwrap())
            .subject("Invitation")
            .body("test token".to_owned())
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(3), config.smtp().unwrap().send(message))
                .await
                .unwrap()
                .is_err()
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn resend_delivers_exact_payload_and_handles_rejection_without_redirect() {
        use axum::{
            Json, Router,
            http::{HeaderMap, StatusCode},
            routing::post,
        };
        let app = Router::new()
            .route(
                "/emails",
                post(
                    |headers: HeaderMap, Json(payload): Json<Value>| async move {
                        assert_eq!(headers["authorization"], "Bearer re_test_only");
                        assert!(headers["idempotency-key"].to_str().unwrap().eq(&format!(
                            "invitation-{}-{}",
                            Uuid::nil(),
                            super::super::digest("token")
                        )));
                        assert_eq!(payload["to"], json!(["invitee@example.com"]));
                        assert_eq!(payload["from"], "Ferrix QA <sender@gmail.com>");
                        assert_eq!(payload["text"], "invitation body token");
                        StatusCode::OK
                    },
                ),
            )
            .route("/reject", post(|| async { StatusCode::TOO_MANY_REQUESTS }))
            .route(
                "/redirect",
                post(|| async { (StatusCode::TEMPORARY_REDIRECT, [("location", "/emails")]) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for (path, success) in [("emails", true), ("reject", false), ("redirect", false)] {
            assert_eq!(
                resend(
                    &config("resend"),
                    &format!("{base}/{path}"),
                    Uuid::nil(),
                    "token",
                    "invitee@example.com",
                    "invitation body token"
                )
                .await
                .is_ok(),
                success
            );
        }
        server.abort();
    }
}
