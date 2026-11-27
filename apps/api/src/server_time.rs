use axum::Json;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct ServerTime {
    unix_ms: i64,
    timezone: String,
    utc_offset: String,
}

impl ServerTime {
    fn at(now: DateTime<Utc>, timezone: Tz) -> Self {
        Self {
            unix_ms: now.timestamp_millis(),
            timezone: timezone.name().to_owned(),
            utc_offset: format!("UTC{}", now.with_timezone(&timezone).format("%:z")),
        }
    }
}

fn resolve_timezone(configured: Option<&str>, system: Option<&str>) -> Tz {
    configured
        .and_then(|name| name.parse::<Tz>().ok())
        .or_else(|| system.and_then(|name| name.parse::<Tz>().ok()))
        .unwrap_or(chrono_tz::UTC)
}

pub(crate) fn system_timezone() -> Tz {
    resolve_timezone(
        std::env::var("TZ").ok().as_deref(),
        iana_time_zone::get_timezone().ok().as_deref(),
    )
}
pub(crate) async fn get(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> Result<Json<ServerTime>, crate::ApiError> {
    let timezone = crate::auth::platform_settings::timezone(&state)
        .await?
        .unwrap_or_else(system_timezone);
    Ok(Json(ServerTime::at(Utc::now(), timezone)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn reports_iana_zone_and_fractional_offsets_from_server_timestamp() {
        let now = Utc.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).unwrap();
        let kl = ServerTime::at(now, chrono_tz::Asia::Kuala_Lumpur);
        assert_eq!(kl.timezone, "Asia/Kuala_Lumpur");
        assert_eq!(kl.utc_offset, "UTC+08:00");
        assert_eq!(kl.unix_ms, now.timestamp_millis());
        let kathmandu = ServerTime::at(now, chrono_tz::Asia::Kathmandu);
        assert_eq!(kathmandu.utc_offset, "UTC+05:45");
    }

    #[test]
    fn configured_container_timezone_takes_precedence() {
        assert_eq!(
            resolve_timezone(Some("Asia/Kuala_Lumpur"), Some("Etc/UTC")),
            chrono_tz::Asia::Kuala_Lumpur
        );
        assert_eq!(
            resolve_timezone(Some("invalid"), Some("Europe/London")),
            chrono_tz::Europe::London
        );
        assert_eq!(resolve_timezone(None, None), chrono_tz::UTC);
    }

    #[test]
    fn offset_tracks_daylight_saving() {
        let winter = Utc.with_ymd_and_hms(2026, 1, 17, 12, 0, 0).unwrap();
        let summer = Utc.with_ymd_and_hms(2026, 7, 17, 12, 0, 0).unwrap();
        assert_eq!(
            ServerTime::at(winter, chrono_tz::Europe::London).utc_offset,
            "UTC+00:00"
        );
        assert_eq!(
            ServerTime::at(summer, chrono_tz::Europe::London).utc_offset,
            "UTC+01:00"
        );
    }
}
