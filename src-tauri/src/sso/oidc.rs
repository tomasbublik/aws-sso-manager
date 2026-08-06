//! SSO OIDC device-authorization flow (SPEC.md §4).
//!
//! Talks directly to the `ssooidc` service in the sso_region with an
//! unauthenticated client — no `aws` binary involved. The resulting token is
//! written to the CLI-compatible cache (see `cache.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_ssooidc::config::Region;
use aws_sdk_ssooidc::Client as OidcClient;
use chrono::Utc;
use serde::Serialize;

use super::cache::{self, TokenCacheEntry};
use crate::error::{AppError, AppResult};

const CLIENT_NAME: &str = crate::branding::APP_NAME;
const SCOPES: &str = "sso:account:access";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_GRANT: &str = "refresh_token";

#[derive(Debug, Clone)]
pub struct SsoSession {
    pub name: String,
    pub start_url: String,
    pub sso_region: String,
}

/// Everything the UI needs to show while the user confirms in the browser,
/// plus what `poll_for_token` needs to finish the flow.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthorization {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    #[serde(skip)]
    pub device_code: String,
    #[serde(skip)]
    pub interval_seconds: u64,
    #[serde(skip)]
    pub expires_in_seconds: u64,
    #[serde(skip)]
    pub client_id: String,
    #[serde(skip)]
    pub client_secret: String,
}

async fn oidc_client(sso_region: &str) -> OidcClient {
    let cfg = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(sso_region.to_string()))
        .no_credentials()
        .load()
        .await;
    OidcClient::new(&cfg)
}

/// Reuse the client registration from the cache file while it is still valid
/// (with a safety margin), otherwise register a fresh public client.
async fn ensure_registration(
    client: &OidcClient,
    session: &SsoSession,
) -> AppResult<(String, String, Option<String>)> {
    if let Some(entry) = cache::read(&session.name) {
        if let (Some(id), Some(secret), Some(expires)) = (
            entry.client_id.as_ref(),
            entry.client_secret.as_ref(),
            entry.registration_expires_at.as_deref().and_then(cache::parse_timestamp),
        ) {
            if expires > Utc::now() + chrono::Duration::minutes(15) {
                return Ok((
                    id.clone(),
                    secret.clone(),
                    entry.registration_expires_at.clone(),
                ));
            }
        }
    }

    let resp = client
        .register_client()
        .client_name(CLIENT_NAME)
        .client_type("public")
        .scopes(SCOPES)
        .send()
        .await
        .map_err(|e| AppError::msg(format!("Client registration with AWS SSO failed: {}", aws_sdk_ssooidc::error::DisplayErrorContext(&e))))?;

    let client_id = resp
        .client_id()
        .ok_or_else(|| AppError::msg("AWS SSO returned no client id"))?
        .to_string();
    let client_secret = resp
        .client_secret()
        .ok_or_else(|| AppError::msg("AWS SSO returned no client secret"))?
        .to_string();
    let registration_expires_at = chrono::DateTime::<Utc>::from_timestamp(resp.client_secret_expires_at(), 0)
        .map(cache::format_timestamp);

    Ok((client_id, client_secret, registration_expires_at))
}

/// Step 1 of login: register (or reuse) the client and start device
/// authorization. Returns the code/URL for the UI; the browser is opened by
/// the caller.
pub async fn start_device_authorization(session: &SsoSession) -> AppResult<DeviceAuthorization> {
    let client = oidc_client(&session.sso_region).await;
    let (client_id, client_secret, registration_expires_at) =
        ensure_registration(&client, session).await?;

    // Persist the registration immediately so it can be reused even if the
    // user abandons this login attempt.
    let mut entry = cache::read(&session.name).unwrap_or_default();
    entry.start_url = Some(session.start_url.clone());
    entry.region = Some(session.sso_region.clone());
    entry.client_id = Some(client_id.clone());
    entry.client_secret = Some(client_secret.clone());
    entry.registration_expires_at = registration_expires_at;
    cache::write(&session.name, &entry)?;

    let resp = client
        .start_device_authorization()
        .client_id(&client_id)
        .client_secret(&client_secret)
        .start_url(&session.start_url)
        .send()
        .await
        .map_err(|e| {
            AppError::msg(format!(
                "Could not start the SSO device login (check the start URL and SSO region): {}",
                aws_sdk_ssooidc::error::DisplayErrorContext(&e)
            ))
        })?;

    Ok(DeviceAuthorization {
        user_code: resp.user_code().unwrap_or_default().to_string(),
        verification_uri: resp.verification_uri().unwrap_or_default().to_string(),
        verification_uri_complete: resp.verification_uri_complete().unwrap_or_default().to_string(),
        device_code: resp.device_code().unwrap_or_default().to_string(),
        interval_seconds: resp.interval().max(1) as u64,
        expires_in_seconds: resp.expires_in().max(30) as u64,
        client_id,
        client_secret,
    })
}

/// Step 2 of login: poll `CreateToken` until the user confirms in the browser.
/// `cancelled` lets the UI abort the wait. On success the token is written to
/// the CLI-compatible cache and the final entry is returned.
pub async fn poll_for_token(
    session: &SsoSession,
    auth: &DeviceAuthorization,
    cancelled: Arc<AtomicBool>,
) -> AppResult<TokenCacheEntry> {
    let client = oidc_client(&session.sso_region).await;
    let mut interval = auth.interval_seconds;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(auth.expires_in_seconds);

    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(AppError::msg("Login was cancelled"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(AppError::msg(
                "The device code expired before the login was confirmed. Please try again.",
            ));
        }

        match client
            .create_token()
            .client_id(&auth.client_id)
            .client_secret(&auth.client_secret)
            .grant_type(DEVICE_GRANT)
            .device_code(&auth.device_code)
            .send()
            .await
        {
            Ok(resp) => {
                let access_token = resp
                    .access_token()
                    .ok_or_else(|| AppError::msg("AWS SSO returned no access token"))?
                    .to_string();
                let expires_at = Utc::now() + chrono::Duration::seconds(resp.expires_in() as i64);

                let mut entry = cache::read(&session.name).unwrap_or_default();
                entry.start_url = Some(session.start_url.clone());
                entry.region = Some(session.sso_region.clone());
                entry.access_token = Some(access_token);
                entry.expires_at = Some(cache::format_timestamp(expires_at));
                if let Some(rt) = resp.refresh_token() {
                    entry.refresh_token = Some(rt.to_string());
                }
                cache::write(&session.name, &entry)?;
                return Ok(entry);
            }
            Err(err) => {
                if let Some(svc) = err.as_service_error() {
                    if svc.is_authorization_pending_exception() {
                        // Expected while the user is still in the browser.
                    } else if svc.is_slow_down_exception() {
                        interval += 5;
                    } else if svc.is_expired_token_exception() {
                        return Err(AppError::msg(
                            "The device code expired before the login was confirmed. Please try again.",
                        ));
                    } else if svc.is_access_denied_exception() {
                        return Err(AppError::msg("The login request was denied."));
                    } else {
                        return Err(AppError::msg(format!(
                            "SSO login failed: {}",
                            aws_sdk_ssooidc::error::DisplayErrorContext(&err)
                        )));
                    }
                } else {
                    return Err(AppError::msg(format!(
                        "Network error while waiting for SSO login: {}",
                        aws_sdk_ssooidc::error::DisplayErrorContext(&err)
                    )));
                }
            }
        }

        // Sleep in short slices so cancellation stays responsive.
        let mut remaining = interval;
        while remaining > 0 && !cancelled.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
            remaining = remaining.saturating_sub(1);
        }
    }
}

/// Silently extend the session using the refresh token, if one was issued.
pub async fn refresh_token(session: &SsoSession) -> AppResult<TokenCacheEntry> {
    let entry = cache::read(&session.name)
        .ok_or_else(|| AppError::msg("No cached SSO session to refresh"))?;
    let (client_id, client_secret, refresh) = match (
        entry.client_id.clone(),
        entry.client_secret.clone(),
        entry.refresh_token.clone(),
    ) {
        (Some(id), Some(secret), Some(rt)) => (id, secret, rt),
        _ => return Err(AppError::msg("This SSO session cannot be refreshed silently — a new login is required")),
    };

    let client = oidc_client(&session.sso_region).await;
    let resp = client
        .create_token()
        .client_id(client_id)
        .client_secret(client_secret)
        .grant_type(REFRESH_GRANT)
        .refresh_token(refresh)
        .send()
        .await
        .map_err(|e| {
            AppError::msg(format!(
                "Refreshing the SSO session failed — a new login is required: {}",
                aws_sdk_ssooidc::error::DisplayErrorContext(&e)
            ))
        })?;

    let access_token = resp
        .access_token()
        .ok_or_else(|| AppError::msg("AWS SSO returned no access token on refresh"))?
        .to_string();
    let expires_at = Utc::now() + chrono::Duration::seconds(resp.expires_in() as i64);

    let mut updated = entry;
    updated.access_token = Some(access_token);
    updated.expires_at = Some(cache::format_timestamp(expires_at));
    if let Some(rt) = resp.refresh_token() {
        updated.refresh_token = Some(rt.to_string());
    }
    cache::write(&session.name, &updated)?;
    Ok(updated)
}
