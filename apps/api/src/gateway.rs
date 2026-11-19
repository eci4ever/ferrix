use crate::{ApiError, AppState};
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bollard::models::{ContainerCreateBody, EndpointSettings};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct GatewayConfig {
    suffix: Option<String>,
    port: u16,
    local: bool,
    https: bool,
    network: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Resource {
    Site(Uuid),
    Application(Uuid),
    Student(Uuid),
}
impl GatewayConfig {
    pub fn from_env() -> Result<Self, String> {
        let ip = std::env::var("FERRIX_PUBLIC_IP").unwrap_or_default();
        let mode = std::env::var("FERRIX_AUTO_DOMAIN")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                if ip.is_empty() {
                    String::new()
                } else {
                    "sslip.io".into()
                }
            });
        if mode.is_empty() || mode == "off" {
            return Ok(Self::default());
        }
        let local = mode == "localhost";
        let suffix = if local {
            "localhost".to_owned()
        } else if mode == "sslip.io" {
            let ip: Ipv4Addr = ip
                .parse()
                .map_err(|_| "Set FERRIX_PUBLIC_IP to the VPS public IPv4 address.")?;
            if ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip == Ipv4Addr::BROADCAST
            {
                return Err("FERRIX_PUBLIC_IP must be a public unicast IPv4 address.".into());
            }
            format!("{}.sslip.io", ip.to_string().replace('.', "-"))
        } else {
            return Err("FERRIX_AUTO_DOMAIN must be localhost, sslip.io or off.".into());
        };
        let network = std::env::var("FERRIX_INGRESS_NETWORK")
            .map_err(|_| "Set FERRIX_INGRESS_NETWORK for automatic domains.")?;
        if network.is_empty()
            || network.len() > 128
            || !network
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err("Invalid FERRIX_INGRESS_NETWORK.".into());
        }
        let https = local && std::env::var("FERRIX_WEBSITE_HTTPS").as_deref() == Ok("true");
        let port_key = if https {
            "FERRIX_WEBSITE_HTTPS_PORT"
        } else {
            "FERRIX_WEBSITE_HTTP_PORT"
        };
        let port = std::env::var(port_key)
            .unwrap_or_else(|_| if https { "443".into() } else { "9080".into() })
            .parse::<u16>()
            .map_err(|_| "Invalid FERRIX_WEBSITE_HTTP_PORT.")?;
        if port == 0 {
            return Err("Website HTTP port must be nonzero.".into());
        }
        Ok(Self {
            suffix: Some(suffix),
            port,
            local,
            https,
            network: Some(network),
        })
    }
    pub(crate) fn ingress_network(&self) -> Option<&str> {
        self.network.as_deref()
    }
    pub(crate) fn url(&self, resource: Resource) -> Option<String> {
        let host = self.hostname(resource)?;
        let secure = !self.local || self.https;
        let scheme = if secure { "https" } else { "http" };
        let standard_port = if secure { 443 } else { 80 };
        Some(if self.local && self.port != standard_port {
            format!("{scheme}://{host}:{}/", self.port)
        } else {
            format!("{scheme}://{host}/")
        })
    }

    fn hostname(&self, resource: Resource) -> Option<String> {
        let (prefix, id) = match resource {
            Resource::Site(id) => ("site", id),
            Resource::Application(id) => ("app", id),
            Resource::Student(id) => ("student", id),
        };
        Some(format!(
            "{prefix}-{}.{}",
            id.simple(),
            self.suffix.as_ref()?
        ))
    }
    pub(crate) fn resource(&self, host: &str) -> Option<Resource> {
        let prefix = host.strip_suffix(&format!(".{}", self.suffix.as_ref()?))?;
        let (kind, id) = prefix.split_once('-')?;
        if id.len() != 32 {
            return None;
        }
        let id = Uuid::parse_str(id).ok()?;
        let resource = match kind {
            "site" => Resource::Site(id),
            "app" => Resource::Application(id),
            "student" => Resource::Student(id),
            _ => return None,
        };
        (self.hostname(resource).as_deref() == Some(host)).then_some(resource)
    }
    pub(crate) fn reserved(&self, host: &str) -> bool {
        self.suffix
            .as_ref()
            .is_some_and(|suffix| host.ends_with(&format!(".{suffix}")))
    }
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Application is unavailable. Deploy or start it and check its automatic-domain network.",
    )
}
fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Website not found.")
}
fn host(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::HOST)?
        .to_str()
        .ok()?
        .parse::<axum::http::uri::Authority>()
        .ok()
        .map(|a| a.host().trim_end_matches('.').to_ascii_lowercase())
}

pub fn public_router(state: AppState) -> Router {
    let files = crate::static_websites::public_router_with_config(
        state.pool.clone(),
        state.gateway.clone(),
    );
    Router::new()
        .route("/_ferrix/route", get(route))
        .with_state(state)
        .merge(files)
}
async fn route(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let host = host(&headers).ok_or_else(missing)?;
    let mut upstream = "unused.invalid:80".to_owned();
    let kind = if let Some(Resource::Student(id)) = state.gateway.resource(&host) {
        upstream = crate::student_hosting::upstream(&state, id).await?;
        "application"
    } else if let Some(Resource::Application(id)) = state.gateway.resource(&host) {
        let row:Option<(String,i32)>=sqlx::query_as("SELECT a.active_container_id,d.container_port FROM applications a JOIN deployments d ON d.id=a.active_deployment_id AND d.application_id=a.id WHERE a.id=$1 AND a.service_kind='application' AND a.active_container_id IS NOT NULL").bind(id).fetch_optional(&state.pool).await?;
        let (container, port) = row.ok_or_else(missing)?;
        let info = crate::deployments::inspect_gateway_container(&state, id, &container).await?;
        if info.state.and_then(|s| s.running) != Some(true) {
            return Err(unavailable());
        }
        let network = state.gateway.network.as_ref().ok_or_else(unavailable)?;
        let ip = info
            .network_settings
            .and_then(|n| n.networks)
            .and_then(|mut n| n.remove(network))
            .and_then(|n| n.ip_address)
            .and_then(|ip| ip.parse::<IpAddr>().ok())
            .ok_or_else(unavailable)?;
        if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
            return Err(unavailable());
        }
        upstream = SocketAddr::new(ip, u16::try_from(port).map_err(|_| unavailable())?).to_string();
        "application"
    } else {
        let id = match state.gateway.resource(&host) {
            Some(Resource::Site(id)) => Some(id),
            _ => None,
        };
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM static_websites WHERE (domain=$1 OR id=$2) AND active_release_id IS NOT NULL)").bind(host).bind(id).fetch_one(&state.pool).await?;
        if !exists {
            return Err(missing());
        }
        "static"
    };
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert("x-ferrix-service-type", kind.parse().unwrap());
    response.headers_mut().insert(
        "x-ferrix-upstream",
        upstream.parse().map_err(|_| unavailable())?,
    );
    Ok(response)
}

pub(crate) async fn attach(
    state: &AppState,
    config: &mut ContainerCreateBody,
) -> Result<(), String> {
    let Some(name) = state.gateway.network.as_ref() else {
        return Ok(());
    };
    let network = tokio::time::timeout(
        Duration::from_secs(5),
        state.docker.inspect_network(name, None),
    )
    .await
    .map_err(|_| "Ingress network lookup timed out.")?
    .map_err(|_| "Start the Ferrix Caddy proxy to create its ingress network.")?;
    if network
        .labels
        .as_ref()
        .and_then(|l| l.get("io.ferrix.ingress"))
        != Some(name)
    {
        return Err("Ingress network ownership could not be verified.".into());
    }
    config
        .networking_config
        .as_mut()
        .ok_or("Missing networking configuration.")?
        .endpoints_config
        .get_or_insert_with(HashMap::new)
        .insert(name.clone(), EndpointSettings::default());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_names_are_exact_and_resource_specific() {
        let c = GatewayConfig {
            suffix: Some("localhost".into()),
            port: 9080,
            local: true,
            https: false,
            network: None,
        };
        let id = Uuid::new_v4();
        let host = c.hostname(Resource::Site(id)).unwrap();
        assert_eq!(c.resource(&host), Some(Resource::Site(id)));
        assert!(c.resource(&format!("{host}.evil.test")).is_none());
        assert!(
            c.resource(&format!("app-{}.localhost", id.hyphenated()))
                .is_none()
        );
        assert!(
            c.url(Resource::Application(id))
                .unwrap()
                .starts_with("http://app-")
        );
        assert!(c.url(Resource::Site(id)).unwrap().ends_with(":9080/"));
        let secure = GatewayConfig {
            https: true,
            port: 443,
            ..c.clone()
        };
        assert_eq!(
            secure.url(Resource::Site(id)),
            Some(format!("https://{host}/"))
        );
        let remote = GatewayConfig {
            suffix: Some("203-0-113-10.sslip.io".into()),
            local: false,
            ..c
        };
        assert!(
            remote
                .url(Resource::Site(id))
                .unwrap()
                .starts_with("https://site-")
        );
    }
}
