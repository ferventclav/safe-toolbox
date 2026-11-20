//! Dates as the files on disk hold them. The earlier C# build wrote local times with or without an offset
//! ("2026-11-18T23:58:27.7793272+05:30", "2026-11-18T23:58:27"), so reading accepts both.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde::{Deserialize, Deserializer, Serializer};

pub type Stamp = DateTime<Local>;

pub fn now() -> Stamp {
    Local::now()
}

pub fn parse(s: &str) -> Option<Stamp> {
    let s = s.trim();
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Local));
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return Local.from_local_datetime(&n).earliest();
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest())
}

pub fn format(t: &Stamp) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
}

/// serde helpers for `Option<Stamp>` fields.
pub mod opt {
    use super::*;

    pub fn serialize<S: Serializer>(v: &Option<Stamp>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(t) => s.serialize_str(&format(t)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Stamp>, D::Error> {
        let v: Option<String> = Option::deserialize(d)?;
        Ok(v.as_deref().and_then(parse))
    }
}

/// serde helpers for `Option<NaiveDate>` fields written as "2026-11-18" or a full date-time.
pub mod opt_date {
    use super::*;

    pub fn serialize<S: Serializer>(v: &Option<NaiveDate>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(d) => s.serialize_str(&d.format("%Y-%m-%d").to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<NaiveDate>, D::Error> {
        let v: Option<String> = Option::deserialize(d)?;
        Ok(v.as_deref().and_then(|s| parse(s).map(|t| t.date_naive())))
    }
}

/// "today at 00:38", "yesterday at 23:09", "Monday at 01:36", "12 Nov at 14:20".
pub fn friendly(t: &Stamp) -> String {
    let today = Local::now().date_naive();
    let d = t.date_naive();
    let hm = t.format("%H:%M");
    let days = (today - d).num_days();
    if days == 0 {
        format!("today at {hm}")
    } else if days == 1 {
        format!("yesterday at {hm}")
    } else if (2..7).contains(&days) {
        format!("{} at {hm}", t.format("%A"))
    } else {
        format!("{} at {hm}", t.format("%-d %b"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_shapes_the_csharp_build_wrote() {
        assert!(parse("2026-11-18T23:58:27.7793272+05:30").is_some());
        assert!(parse("2026-11-18T23:58:27").is_some());
        assert_eq!(parse("2026-11-18").unwrap().format("%Y-%m-%d").to_string(), "2026-11-18");
        assert!(parse("not a date").is_none());
    }
}
