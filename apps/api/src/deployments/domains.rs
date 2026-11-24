mod private;
use super::*;
use hickory_resolver::TokioResolver;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/services/{id}/domains", get(list))
        .route(
            "/services/{id}/domains/{domain_id}/tunnel-config",
            get(private::config),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::sessions::identify,
        ))
        .merge(
            Router::new()
                .route("/services/{id}/domains", post(create))
                .route("/services/{id}/domains/{domain_id}/check", post(check))
                .route(
                    "/services/{id}/domains/{domain_id}/preferred",
                    post(preferred),
                )
                .route(
                    "/services/{id}/domains/{domain_id}",
                    axum::routing::delete(remove),
                )
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::sessions::protect,
                )),
        )
}

#[derive(Serialize, sqlx::FromRow)]
struct Domain {
    id: Uuid,
    hostname: String,
    route_kind: String,
    canonical: bool,
    http_status: String,
    tunnel_status: String,
    verification_token: String,
    verified_at: Option<DateTime<Utc>>,
    dns_status: String,
    tls_status: String,
    certificate_expires_at: Option<DateTime<Utc>>,
    message: Option<String>,
    checked_at: Option<DateTime<Utc>>,
}
const COLUMNS: &str = "id,hostname,route_kind,canonical,http_status,tunnel_status,verification_token,verified_at,dns_status,CASE WHEN tls_status='active' AND certificate_expires_at <= now() THEN 'unavailable' ELSE tls_status END AS tls_status,certificate_expires_at,message,checked_at";

async fn access(state: &AppState, id: &Identity, app: Uuid) -> Result<Application, ApiError> {
    let row = app_access(state, id, app).await?;
    if row.kind != "application" {
        return Err(missing());
    }
    Ok(row)
}
fn configured(state: &AppState) -> Result<(IpAddr, u16, bool, bool), ApiError> {
    state.gateway.domain_target().ok_or_else(|| {
        invalid("Configure Ferrix automatic domains and the Caddy ingress network first.")
    })
}
async fn list(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let application = access(&state, &id, app).await?;
    let rows: Vec<Domain> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM application_domains WHERE application_id=$1 ORDER BY created_at,id"
    ))
    .bind(app)
    .fetch_all(&state.pool)
    .await?;
    let target = state.gateway.domain_target();
    Ok(Json(
        json!({"domains":rows,"automatic_url":application.automatic_url,
        "target_ip":target.map(|t| t.0.to_string()),"https_enabled":target.is_some_and(|t| t.2),
        "private":state.gateway.private_mode(),"local":target.is_some_and(|t| t.3),"port":target.map(|t| t.1),"dashboard_origin":state.auth.origin,"diagnostic_url":application.host_port.map(|p|format!("http://127.0.0.1:{p}/"))}),
    ))
}
async fn preferred(
    State(state): State<AppState>,
    id: Identity,
    Path((app, domain)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    access(&state, &id, app).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .execute(&mut *tx)
        .await?;
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM application_domains WHERE id=$1 AND application_id=$2 AND verified_at IS NOT NULL AND dns_status='ready')").bind(domain).bind(app).fetch_one(&mut *tx).await?;
    if !valid {
        return Err(invalid(
            "Verify domain ownership and DNS before choosing the preferred URL.",
        ));
    }
    sqlx::query("UPDATE application_domains SET canonical=false WHERE application_id=$1")
        .bind(app)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE application_domains SET canonical=true WHERE id=$1 AND application_id=$2")
        .bind(domain)
        .bind(app)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
fn hostname(value: &str, local: bool) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() > 253
        || !value.contains('.')
        || !matches!(url::Host::parse(&value), Ok(url::Host::Domain(ref name)) if name == &value)
        || value.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        || value.ends_with(".local")
        || value.ends_with(".internal")
        || value.ends_with(".test")
        || (local && !value.ends_with(".localhost"))
        || (!local && value.ends_with(".localhost"))
    {
        return Err(invalid(if local {
            "Enter a local hostname such as myapp.localhost, without a scheme, port or path."
        } else {
            "Enter a public hostname such as app.example.com, without a scheme, port or path. Use punycode for international names."
        }));
    }
    Ok(value)
}
#[derive(Deserialize)]
struct NewDomain {
    hostname: String,
    #[serde(default = "direct_kind")]
    route_kind: String,
    #[serde(default)]
    canonical: bool,
}
fn direct_kind() -> String {
    "direct".into()
}
async fn create(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
    Json(input): Json<NewDomain>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    id.manager()?;
    if access(&state, &id, app).await?.worker_mode {
        return Err(invalid(
            "Background workers do not publish website domains.",
        ));
    }
    let (_, _, _, local) = configured(&state)?;
    if !matches!(input.route_kind.as_str(), "direct" | "lan" | "tunnel") {
        return Err(invalid("Choose direct, LAN or Cloudflare Tunnel routing."));
    }
    if input.route_kind != "direct" && !state.gateway.private_mode() {
        return Err(invalid(
            "Enable private ingress mode before adding LAN/tunnel mappings.",
        ));
    }
    let hostname = hostname(&input.hostname, local)?;
    let dashboard = url::Url::parse(&state.auth.origin).ok();
    if state.gateway.reserved_domain(&hostname)
        || dashboard.as_ref().and_then(|u| u.host_str()) == Some(hostname.as_str())
    {
        return Err(invalid("This hostname is reserved by Ferrix."));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(missing)?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_domains WHERE application_id=$1")
            .bind(app)
            .fetch_one(&mut *tx)
            .await?;
    if count >= 10 {
        return Err(invalid(
            "Each application supports up to 10 custom domains.",
        ));
    }
    let domain = Uuid::new_v4();
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    if input.canonical {
        sqlx::query("UPDATE application_domains SET canonical=false WHERE application_id=$1")
            .bind(app)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO application_domains(id,application_id,hostname,verification_token,route_kind,canonical,tunnel_status) VALUES($1,$2,$3,$4,$5,$6,CASE WHEN $5='tunnel' THEN 'configuration_required' ELSE 'not_applicable' END)")
        .bind(domain).bind(app).bind(hostname).bind(hex::encode(bytes)).bind(input.route_kind).bind(input.canonical).execute(&mut *tx).await.map_err(domain_error)?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":domain}))))
}
fn domain_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        ApiError(
            StatusCode::CONFLICT,
            "This domain is already added here or verified by another application.",
        )
    } else {
        error.into()
    }
}
async fn remove(
    State(state): State<AppState>,
    id: Identity,
    Path((app, domain)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    id.manager()?;
    access(&state, &id, app).await?;
    if sqlx::query("DELETE FROM application_domains WHERE id=$1 AND application_id=$2")
        .bind(domain)
        .bind(app)
        .execute(&state.pool)
        .await?
        .rows_affected()
        == 0
    {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn dns(host: &str, token: &str, expected: IpAddr, local: bool) -> Result<(), &'static str> {
    if local {
        return Ok(());
    }
    let mut builder = TokioResolver::builder_tokio().map_err(|_| "DNS resolver is unavailable.")?;
    builder.options_mut().ip_strategy = hickory_resolver::config::LookupIpStrategy::Ipv4AndIpv6;
    let resolver = builder.build();
    dns_lookup(&resolver, host, token, expected).await
}
async fn dns_lookup(
    resolver: &TokioResolver,
    host: &str,
    token: &str,
    expected: IpAddr,
) -> Result<(), &'static str> {
    let address = resolver
        .lookup_ip(format!("{host}."))
        .await
        .map_err(|_| "No DNS address found. Add the A record and wait for DNS propagation.")?;
    let addresses: Vec<IpAddr> = address.iter().collect();
    if !addresses_match(&addresses, expected) {
        return Err(
            "DNS does not point exclusively to this Ferrix server. Check A and AAAA records and disable CDN proxying while verifying.",
        );
    }
    let proof = resolver
        .txt_lookup(format!("_ferrix-challenge.{host}."))
        .await
        .map_err(|_| "Add the TXT ownership record and wait for DNS propagation.")?;
    if !proof
        .iter()
        .any(|record| record.txt_data().concat() == token.as_bytes())
    {
        return Err("TXT ownership record does not match this application's verification token.");
    }
    Ok(())
}
fn addresses_match(addresses: &[IpAddr], expected: IpAddr) -> bool {
    !addresses.is_empty() && addresses.iter().all(|ip| *ip == expected)
}
fn probe_client() -> Result<reqwest::ClientBuilder, &'static str> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    if let Ok(path) = std::env::var("FERRIX_INGRESS_CA_CERT")
        && !path.is_empty()
    {
        let metadata = std::fs::metadata(&path)
            .map_err(|_| "Could not read the configured public ingress CA certificate.")?;
        if !metadata.is_file() || metadata.len() > 65536 {
            return Err("Invalid ingress CA certificate file.");
        }
        let bytes = std::fs::read(path).map_err(|_| "Could not read ingress CA certificate.")?;
        let certificate = reqwest::Certificate::from_pem(&bytes)
            .map_err(|_| "Invalid public ingress CA certificate.")?;
        builder = builder.add_root_certificate(certificate);
    }
    Ok(builder)
}
async fn certificate(host: &str, target: IpAddr, port: u16) -> Result<DateTime<Utc>, &'static str> {
    // Connect only to the operator-configured ingress address. Never follow redirects
    // or use a hostname's DNS result as an outbound probe destination.
    let client = probe_client()?
        .timeout(Duration::from_secs(25))
        .tls_info(true)
        .resolve(host, SocketAddr::new(target, port))
        .build()
        .map_err(|_| "Could not initialize the HTTPS verifier.")?;
    let response = client.head(format!("https://{host}:{port}/")).send().await.map_err(|_| "HTTPS could not be verified yet. Caddy may still be issuing a certificate. Check ports 80/443, DNS, CA restrictions and Caddy logs. For local HTTPS, trust the Caddy root certificate.")?;
    let der = response
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .and_then(|info| info.peer_certificate())
        .ok_or("HTTPS did not provide certificate details.")?;
    let (_, certificate) = x509_parser::parse_x509_certificate(der)
        .map_err(|_| "Could not read the HTTPS certificate.")?;
    DateTime::from_timestamp(certificate.validity().not_after.timestamp(), 0)
        .ok_or("Invalid certificate expiry.")
}
async fn check(
    State(state): State<AppState>,
    id: Identity,
    Path((app, domain)): Path<(Uuid, Uuid)>,
) -> Result<Json<Domain>, ApiError> {
    id.manager()?;
    let application = access(&state, &id, app).await?;
    let (target, port, https, local) = configured(&state)?;
    let row: Domain = sqlx::query_as(&format!("UPDATE application_domains SET checked_at=now() WHERE id=$1 AND application_id=$2 AND (checked_at IS NULL OR checked_at < now()-interval '60 seconds') RETURNING {COLUMNS}"))
        .bind(domain).bind(app).fetch_optional(&state.pool).await?.ok_or(ApiError(StatusCode::CONFLICT, "Domain not found or checked recently. Wait 60 seconds before checking again."))?;
    if row.route_kind != "direct" {
        return private::check(&state, app, row, target, port, https).await;
    }
    let checked = tokio::time::timeout(
        Duration::from_secs(8),
        dns(&row.hostname, &row.verification_token, target, local),
    )
    .await;
    let (dns_status, message) = match checked {
        Ok(Ok(())) => ("ready", None),
        Ok(Err(message)) => ("mismatch", Some(message)),
        Err(_) => (
            "unavailable",
            Some("DNS lookup timed out. Try again after propagation."),
        ),
    };
    // Authorization for Caddy is persisted before probing TLS, so on-demand issuance
    // can call the fast database-only ask endpoint while this request is in flight.
    sqlx::query("UPDATE application_domains SET dns_status=$1,verified_at=CASE WHEN $1='ready' THEN coalesce(verified_at,now()) ELSE verified_at END,tls_status='pending',certificate_expires_at=NULL,message=$2 WHERE id=$3 AND application_id=$4")
        .bind(dns_status).bind(message).bind(domain).bind(app).execute(&state.pool).await.map_err(domain_error)?;
    let (tls_status, expires, message) = if !https {
        (
            "disabled",
            None,
            message.or(Some(
                "HTTPS is disabled in this local ingress configuration.",
            )),
        )
    } else if dns_status != "ready" {
        ("pending", None, message)
    } else if application.active_container_id.is_none() {
        (
            "pending",
            None,
            Some("Deploy the application before Caddy can issue its certificate."),
        )
    } else {
        match certificate(&row.hostname, target, port).await {
            Ok(expiry) => (
                "active",
                Some(expiry),
                Some("HTTPS verified with a trusted certificate."),
            ),
            Err(message) => ("unavailable", None, Some(message)),
        }
    };
    let row = sqlx::query_as(&format!("UPDATE application_domains SET tls_status=$1,certificate_expires_at=$2,message=$3 WHERE id=$4 AND application_id=$5 RETURNING {COLUMNS}"))
        .bind(tls_status).bind(expires).bind(message).bind(domain).bind(app).fetch_optional(&state.pool).await?.ok_or_else(missing)?;
    Ok(Json(row))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostnames_are_exact_and_cannot_supply_probe_destinations() {
        assert_eq!(
            hostname(" App.Example.COM ", false).unwrap(),
            "app.example.com"
        );
        for name in [
            "https://a.example.com",
            "a.example.com:443",
            "*.example.com",
            "127.0.0.1",
            "127.1",
            "0x7f.1",
            "localhost",
            "a..com",
            "-a.com",
            "a.com/",
            "a.com.",
            "a.localhost",
            "a.internal",
            "a.local",
            "a.test",
        ] {
            assert!(hostname(name, false).is_err(), "{name}");
        }
        assert!(hostname("demo.localhost", true).is_ok());
        assert!(hostname("demo.example.com", true).is_err());
        let expected = "203.0.113.10".parse().unwrap();
        assert!(addresses_match(&[expected], expected));
        assert!(!addresses_match(&[], expected));
        assert!(!addresses_match(
            &[expected, "127.0.0.1".parse().unwrap()],
            expected
        ));
    }
    #[tokio::test]
    async fn dns_checks_both_address_families_and_chunked_ownership_proof() {
        use hickory_resolver::{
            config::{LookupIpStrategy, NameServerConfigGroup, ResolverConfig},
            name_server::TokioConnectionProvider,
            proto::{
                op::{Message, MessageType},
                rr::{
                    RData, Record, RecordType,
                    rdata::{A, AAAA, TXT},
                },
            },
        };
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            loop {
                let (len, peer) = socket.recv_from(&mut buffer).await.unwrap();
                let query = Message::from_vec(&buffer[..len]).unwrap();
                let mut reply = Message::new();
                reply
                    .set_id(query.id())
                    .set_message_type(MessageType::Response)
                    .set_recursion_desired(true)
                    .set_recursion_available(true);
                for question in query.queries() {
                    reply.add_query(question.clone());
                    let host = question.name().to_utf8();
                    let value = match question.query_type() {
                        RecordType::A => Some(RData::A(A("203.0.113.10".parse().unwrap()))),
                        RecordType::AAAA if host == "mixed.example.com." => {
                            Some(RData::AAAA(AAAA("::1".parse().unwrap())))
                        }
                        RecordType::TXT => {
                            Some(RData::TXT(TXT::new(vec!["proof-".into(), "token".into()])))
                        }
                        _ => None,
                    };
                    if let Some(value) = value {
                        reply.add_answer(Record::from_rdata(question.name().clone(), 60, value));
                    }
                }
                socket
                    .send_to(&reply.to_vec().unwrap(), peer)
                    .await
                    .unwrap();
            }
        });
        let config = ResolverConfig::from_parts(
            None,
            vec![],
            NameServerConfigGroup::from_ips_clear(&["127.0.0.1".parse().unwrap()], port, true),
        );
        let mut builder =
            TokioResolver::builder_with_config(config, TokioConnectionProvider::default());
        builder.options_mut().ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        let resolver = builder.build();
        let expected = "203.0.113.10".parse().unwrap();
        assert!(
            dns_lookup(&resolver, "valid.example.com", "proof-token", expected)
                .await
                .is_ok()
        );
        assert!(
            dns_lookup(&resolver, "valid.example.com", "wrong-token", expected)
                .await
                .is_err()
        );
        assert!(
            dns_lookup(&resolver, "mixed.example.com", "proof-token", expected)
                .await
                .is_err()
        );
        server.abort();
    }
}
