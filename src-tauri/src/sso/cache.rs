//! AWS-CLI-compatible SSO token cache.
//!
//! The AWS CLI v2 / botocore reads SSO tokens from
//! `~/.aws/sso/cache/<sha1_hex(session_name)>.json`. Field names and the
//! `expiresAt` timestamp format must match exactly, otherwise `aws` commands
//! will not pick up the session (see SPEC.md §3).

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fs;
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

pub const EXPIRES_AT_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// One entry in the SSO token cache. The AWS CLI stores both the OIDC client
/// registration and the issued token in the same file, so both are kept here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenCacheEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

pub fn format_timestamp(ts: DateTime<Utc>) -> String {
    ts.format(EXPIRES_AT_FORMAT).to_string()
}

/// Parse the timestamp formats seen in real cache files: `2026-08-05T21:30:00Z`
/// (botocore) as well as full RFC 3339 with offset or fractional seconds.
pub fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(s, EXPIRES_AT_FORMAT)
        .ok()
        .map(|naive| naive.and_utc())
}

pub fn cache_dir() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::msg("Cannot determine home directory"))?;
    Ok(home.join(".aws").join("sso").join("cache"))
}

pub fn cache_file_name(session_name: &str) -> String {
    let digest = Sha1::digest(session_name.as_bytes());
    format!("{}.json", hex::encode(digest))
}

pub fn cache_path(session_name: &str) -> AppResult<PathBuf> {
    Ok(cache_dir()?.join(cache_file_name(session_name)))
}

pub fn read(session_name: &str) -> Option<TokenCacheEntry> {
    let path = cache_path(session_name).ok()?;
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn write(session_name: &str, entry: &TokenCacheEntry) -> AppResult<()> {
    let dir = cache_dir()?;
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    let path = dir.join(cache_file_name(session_name));
    let json = serde_json::to_string(entry)?;
    fs::write(&path, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn delete(session_name: &str) -> AppResult<()> {
    let path = cache_path(session_name)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn cache_file_name_is_sha1_hex() {
        // Well-known SHA1 test vector.
        assert_eq!(
            cache_file_name("abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d.json"
        );
    }

    #[test]
    fn timestamp_roundtrip_matches_botocore_format() {
        let ts = Utc.with_ymd_and_hms(2026, 8, 5, 21, 30, 0).unwrap();
        let formatted = format_timestamp(ts);
        assert_eq!(formatted, "2026-08-05T21:30:00Z");
        assert_eq!(parse_timestamp(&formatted), Some(ts));
    }

    #[test]
    fn parses_rfc3339_with_offset() {
        let parsed = parse_timestamp("2026-08-05T23:30:00+02:00").unwrap();
        assert_eq!(format_timestamp(parsed), "2026-08-05T21:30:00Z");
    }

    #[test]
    fn serializes_with_camel_case_fields() {
        let entry = TokenCacheEntry {
            start_url: Some("https://acme.awsapps.com/start".into()),
            region: Some("eu-west-1".into()),
            access_token: Some("tok".into()),
            expires_at: Some("2026-08-05T21:30:00Z".into()),
            client_id: Some("cid".into()),
            client_secret: Some("csec".into()),
            registration_expires_at: None,
            refresh_token: None,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["startUrl"], "https://acme.awsapps.com/start");
        assert_eq!(json["accessToken"], "tok");
        assert_eq!(json["expiresAt"], "2026-08-05T21:30:00Z");
        assert_eq!(json["clientId"], "cid");
        assert!(json.get("refreshToken").is_none());
    }
}
