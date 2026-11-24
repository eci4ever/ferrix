use super::*;
// Public reachability probes never connect to private/link-local metadata endpoints.
fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_multicast()
                || v.is_broadcast()
                || v.octets()[0] == 0
                || v.octets()[0] >= 240
                || v.octets()[0] == 127
                || (v.octets()[0] == 100 && (64..=127).contains(&v.octets()[1]))
                || v.octets()[..2] == [198, 18])
        }
        IpAddr::V6(v) => {
            v.segments()[0] & 0xe000 == 0x2000
                && !v
                    .to_ipv4_mapped()
                    .is_some_and(|v| !public_address(IpAddr::V4(v)))
        }
    }
}
async fn ownership(
    host: &str,
    token: &str,
    target: IpAddr,
    lan: bool,
) -> Result<Vec<IpAddr>, &'static str> {
    let mut builder = TokioResolver::builder_tokio().map_err(|_| "DNS resolver unavailable.")?;
    builder.options_mut().ip_strategy = hickory_resolver::config::LookupIpStrategy::Ipv4AndIpv6;
    let resolver = builder.build();
    let proof = resolver
        .txt_lookup(format!("_ferrix-challenge.{host}."))
        .await
        .map_err(|_| "Publish the TXT ownership record in the DNS resolver used by Ferrix.")?;
    if !proof
        .iter()
        .any(|r| r.txt_data().concat() == token.as_bytes())
    {
        return Err("TXT ownership record does not match.");
    }
    let ips: Vec<_> = resolver
        .lookup_ip(format!("{host}."))
        .await
        .map_err(|_| "No DNS address found.")?
        .iter()
        .collect();
    if if lan {
        !addresses_match(&ips, target)
    } else {
        ips.is_empty() || ips.iter().any(|ip| !public_address(*ip))
    } {
        return Err(if lan {
            "LAN DNS must resolve exclusively to the configured Caddy ingress address."
        } else {
            "Tunnel DNS must resolve to public addresses. Configure the Cloudflare published hostname and DNS record."
        });
    }
    Ok(ips)
}
pub(super) async fn check(
    state: &AppState,
    app: Uuid,
    row: Domain,
    target: IpAddr,
    port: u16,
    https: bool,
) -> Result<Json<Domain>, ApiError> {
    let lan = row.route_kind == "lan";
    let owned = tokio::time::timeout(
        Duration::from_secs(8),
        ownership(&row.hostname, &row.verification_token, target, lan),
    )
    .await;
    let (dns_status, message, ips) = match owned {
        Ok(Ok(ips)) => ("ready", None, ips),
        Ok(Err(e)) => ("mismatch", Some(e), vec![]),
        Err(_) => ("unavailable", Some("DNS lookup timed out."), vec![]),
    };
    sqlx::query("UPDATE application_domains SET dns_status=$1,verified_at=CASE WHEN $1='ready' THEN coalesce(verified_at,now()) ELSE verified_at END,tls_status='pending',message=$2 WHERE id=$3 AND application_id=$4").bind(dns_status).bind(message).bind(row.id).bind(app).execute(&state.pool).await.map_err(domain_error)?;
    let mut tls = if https || !lan { "pending" } else { "disabled" };
    let mut http = "unverified";
    let mut expires = None;
    let mut message = message;
    if dns_status == "ready" {
        let ip = if lan { target } else { ips[0] };
        let probe_port = if lan { port } else { 443 };
        if https || !lan {
            match certificate(&row.hostname, ip, probe_port).await {
                Ok(e) => {
                    tls = "active";
                    expires = Some(e);
                }
                Err(e) => {
                    tls = "unavailable";
                    message = Some(e);
                }
            }
        }
        if tls == "active" || tls == "disabled" {
            let scheme = if tls == "disabled" { "http" } else { "https" };
            let response = probe_client()
                .map_err(|_| invalid("Check the configured public ingress CA certificate."))?
                .timeout(Duration::from_secs(10))
                .resolve(&row.hostname, SocketAddr::new(ip, probe_port))
                .build()
                .map_err(|_| internal())?
                .head(format!("{scheme}://{}:{probe_port}/", row.hostname))
                .send()
                .await;
            http = if response.is_ok_and(|r| r.status().is_success() || r.status().is_redirection())
            {
                "reachable"
            } else {
                "unavailable"
            };
            if http == "reachable" {
                message = Some(if lan {
                    "LAN HTTP endpoint reachable. Client devices must trust the Caddy CA for internal HTTPS."
                } else {
                    "Public HTTPS endpoint reachable. Cloudflare connector health is not queried; verify the tunnel in Cloudflare."
                });
            }
        }
    }
    sqlx::query_as(&format!("UPDATE application_domains SET tls_status=$1,certificate_expires_at=$2,http_status=$3,tunnel_status=CASE WHEN route_kind='tunnel' THEN 'unverified' ELSE 'not_applicable' END,message=$4 WHERE id=$5 AND application_id=$6 RETURNING {COLUMNS}")).bind(tls).bind(expires).bind(http).bind(message).bind(row.id).bind(app).fetch_one(&state.pool).await.map(Json).map_err(Into::into)
}
pub(super) async fn config(
    State(state): State<AppState>,
    id: Identity,
    Path((app, domain)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    access(&state, &id, app).await?;
    let host:String=sqlx::query_scalar("SELECT hostname FROM application_domains WHERE id=$1 AND application_id=$2 AND route_kind='tunnel'").bind(domain).bind(app).fetch_optional(&state.pool).await?.ok_or_else(missing)?;
    let secure = state.gateway.domain_target().is_some_and(|t| t.2);
    let origin = std::env::var("FERRIX_TUNNEL_ORIGIN").unwrap_or_else(|_| {
        if secure {
            "https://proxy:443".into()
        } else {
            "http://proxy:80".into()
        }
    });
    let url =
        url::Url::parse(&origin).map_err(|_| invalid("Invalid operator FERRIX_TUNNEL_ORIGIN."))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(invalid(
            "FERRIX_TUNNEL_ORIGIN must be an HTTP(S) origin without credentials or a path.",
        ));
    }
    let mut request = json!({"httpHostHeader":host});
    if url.scheme() == "https" {
        request["originServerName"] = json!(host);
        request["caPool"] = json!(
            std::env::var("FERRIX_TUNNEL_CA_POOL")
                .unwrap_or_else(|_| "/etc/cloudflared/caddy-root.crt".into())
        );
    }
    Ok(Json(
        json!({"hostname":host,"ingress_rule":{"hostname":host,"service":origin,"originRequest":request},"instructions":"Review and merge this single rule before your existing catch-all. Preserve all unrelated rules. Supply tunnel credentials directly to cloudflared; Ferrix does not store them. Mount only the public Caddy CA certificate at caPool for HTTPS origins. Keep TLS verification enabled.","dashboard_origin":state.auth.origin}),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_probes_reject_internal_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
        ] {
            assert!(!public_address(ip.parse().unwrap()));
        }
        assert!(public_address("1.1.1.1".parse().unwrap()));
    }
}

#[cfg(test)]
mod integration {
    use super::*;
    use crate::deployments::test_support::*;
    use futures_util::FutureExt;
    use std::panic::AssertUnwindSafe;
    #[tokio::test]
    #[ignore = "requires FERRIX_TEST_DATABASE_URL"]
    async fn private_mappings_are_scoped_and_export_verified_tls_settings() {
        let f = Fixture::new().await;
        let result=AssertUnwindSafe(async {
   let mut state=f.state.clone();state.gateway=crate::gateway::GatewayConfig::private_fixture().into();let app=crate::router(state.clone());let base=format!("/api/v1/services/{}/domains",f.service);
   let create=call(&app,"POST",&base,&f.cookie,json!({"hostname":"web.example.com","route_kind":"tunnel","canonical":true}),&[],true).await;assert_eq!(create.status,StatusCode::CREATED);let first=create.body["id"].as_str().unwrap();
   let lan=call(&app,"POST",&base,&f.cookie,json!({"hostname":"web.home.arpa","route_kind":"lan"}),&[],true).await;assert_eq!(lan.status,StatusCode::CREATED);
   let rule=call(&app,"GET",&format!("{base}/{first}/tunnel-config"),&f.cookie,Value::Null,&[],false).await;assert_eq!(rule.status,StatusCode::OK);assert_eq!(rule.body["ingress_rule"]["originRequest"]["originServerName"],"web.example.com");assert!(rule.body["ingress_rule"]["originRequest"].get("noTLSVerify").is_none());
   assert_eq!(call(&app,"POST",&format!("{base}/{first}/preferred"),&f.cookie,json!({}),&[],true).await.status,StatusCode::UNPROCESSABLE_ENTITY);
   sqlx::query("UPDATE application_domains SET verified_at=now(),dns_status='ready' WHERE application_id=$1").bind(f.service).execute(&f.state.pool).await.unwrap();
   let current=call(&app,"GET",&format!("/api/v1/services/{}",f.service),&f.cookie,Value::Null,&[],false).await;assert_eq!(current.body["automatic_url"],"https://web.example.com/");
   assert_eq!(call(&app,"DELETE",&format!("{base}/{first}"),&f.cookie,Value::Null,&[],true).await.status,StatusCode::NO_CONTENT);
   let rows=call(&app,"GET",&base,&f.cookie,Value::Null,&[],false).await;assert_eq!(rows.body["domains"].as_array().unwrap().len(),1);
   let lan_id=lan.body["id"].as_str().unwrap();assert_eq!(call(&app,"POST",&format!("{base}/{lan_id}/preferred"),&f.cookie,json!({}),&[],true).await.status,StatusCode::NO_CONTENT);
   let current=call(&app,"GET",&format!("/api/v1/services/{}",f.service),&f.cookie,Value::Null,&[],false).await;assert_eq!(current.body["automatic_url"],"https://web.home.arpa:8443/");
   assert_eq!(call(&app,"GET",&format!("/api/v1/services/{}/domains/{lan_id}/tunnel-config",Uuid::new_v4()),&f.cookie,Value::Null,&[],false).await.status,StatusCode::NOT_FOUND);
  }).catch_unwind().await;
        f.finish().await;
        if let Err(e) = result {
            std::panic::resume_unwind(e)
        }
    }
}
