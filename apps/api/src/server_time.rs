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

pub(crate) async fn get() -> Json<ServerTime> {
    let timezone = iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse::<Tz>().ok())
        .unwrap_or(chrono_tz::UTC);
    Json(ServerTime::at(Utc::now(), timezone))
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
