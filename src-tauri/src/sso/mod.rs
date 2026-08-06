//! Native AWS SSO support: CLI-compatible token cache + OIDC device flow.

pub mod cache;
pub mod oidc;

use chrono::Utc;
use serde::Serialize;

pub use oidc::{DeviceAuthorization, SsoSession};

/// Token state as reported to the frontend and tray (SPEC.md §6).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatus {
    pub valid: bool,
    pub expires_at: String,
    pub remaining_seconds: i64,
    pub can_refresh: bool,
}

/// Compute the current token status for a session, or `None` when no token
/// has ever been cached.
pub fn token_status(session_name: &str) -> Option<TokenStatus> {
    let entry = cache::read(session_name)?;
    let can_refresh = entry.refresh_token.is_some()
        && entry.client_id.is_some()
        && entry.client_secret.is_some();
    let expires_at = entry.expires_at.as_deref()?.to_string();
    let remaining = cache::parse_timestamp(&expires_at)
        .map(|ts| (ts - Utc::now()).num_seconds())
        .unwrap_or(0);
    Some(TokenStatus {
        valid: entry.access_token.is_some() && remaining > 0,
        expires_at,
        remaining_seconds: remaining.max(0),
        can_refresh,
    })
}
