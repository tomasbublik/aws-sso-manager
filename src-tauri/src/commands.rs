//! `#[tauri::command]` handlers (SPEC.md §6) — thin wrappers around
//! `awsconfig`, `discovery`, `verify` and `sso::oidc`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::awsconfig::{self, ProfileWrite};
use crate::sso::{self, DeviceAuthorization, SsoSession, TokenStatus};

// ---------------------------------------------------------------------
// Payload types (SPEC.md §6) — camelCase on the wire.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub name: String,
    pub start_url: String,
    pub sso_region: String,
}

impl From<awsconfig::SsoSessionInfo> for SessionInfo {
    fn from(s: awsconfig::SsoSessionInfo) -> Self {
        Self {
            name: s.name,
            start_url: s.start_url,
            sso_region: s.sso_region,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub cli_installed: bool,
    pub session: Option<SessionInfo>,
    pub token: Option<TokenStatus>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub account_id: String,
    pub account_name: String,
    pub role_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub profile_name: String,
    pub account_id: String,
    pub role_name: String,
    pub region: String,
    pub is_default: bool,
}

impl From<awsconfig::ManagedProfile> for ProfileInfo {
    fn from(p: awsconfig::ManagedProfile) -> Self {
        Self {
            profile_name: p.profile_name,
            account_id: p.account_id,
            role_name: p.role_name,
            region: p.region,
            is_default: p.is_default,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    pub installed: bool,
    pub version: Option<String>,
}

// ---------------------------------------------------------------------
// Managed state for the in-flight device-authorization login.
// ---------------------------------------------------------------------

struct PendingLogin {
    session: SsoSession,
    auth: DeviceAuthorization,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct LoginState {
    pending: tokio::sync::Mutex<Option<PendingLogin>>,
}

// ---------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------

fn require_session(text: &str) -> Result<awsconfig::SsoSessionInfo, String> {
    awsconfig::first_sso_session(text)
        .ok_or_else(|| "No SSO session configured yet. Please complete setup first.".to_string())
}

fn run_check_cli() -> CliStatus {
    let mut cmd = std::process::Command::new("aws");
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.output() {
        Ok(output) => {
            let mut raw = String::from_utf8_lossy(&output.stdout).to_string();
            raw.push(' ');
            raw.push_str(&String::from_utf8_lossy(&output.stderr));
            // Typical output: "aws-cli/2.15.0 Python/3.11.6 ..."
            let version = raw
                .split_whitespace()
                .next()
                .and_then(|tok| tok.split_once('/'))
                .map(|(_, v)| v.to_string());
            CliStatus {
                installed: output.status.success(),
                version,
            }
        }
        Err(_) => CliStatus {
            installed: false,
            version: None,
        },
    }
}

async fn build_app_state() -> Result<AppState, String> {
    let text = awsconfig::read_config_text().map_err(String::from)?;
    let session = awsconfig::first_sso_session(&text);
    let token = session.as_ref().and_then(|s| sso::token_status(&s.name));
    let cli = tokio::task::spawn_blocking(run_check_cli)
        .await
        .map_err(|e| e.to_string())?;

    Ok(AppState {
        cli_installed: cli.installed,
        session: session.map(SessionInfo::from),
        token,
    })
}

// ---------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn get_app_state() -> Result<AppState, String> {
    build_app_state().await
}

#[tauri::command]
pub async fn save_sso_session(name: String, start_url: String, sso_region: String) -> Result<AppState, String> {
    let session = awsconfig::SsoSessionInfo {
        name,
        start_url,
        sso_region,
    };
    let text = awsconfig::read_config_text().map_err(String::from)?;
    let text = awsconfig::upsert_sso_session(&text, &session);
    awsconfig::write_config_text(&text).map_err(String::from)?;
    build_app_state().await
}

#[tauri::command]
pub async fn sso_login_start(
    app: AppHandle,
    state: State<'_, LoginState>,
) -> Result<DeviceAuthorization, String> {
    let text = awsconfig::read_config_text().map_err(String::from)?;
    let session_info = require_session(&text)?;
    let session = SsoSession::from(&session_info);

    let auth = sso::oidc::start_device_authorization(&session)
        .await
        .map_err(String::from)?;

    let _ = app
        .opener()
        .open_url(&auth.verification_uri_complete, None::<String>);

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut pending = state.pending.lock().await;
        *pending = Some(PendingLogin {
            session,
            auth: auth.clone(),
            cancel,
        });
    }

    Ok(auth)
}

#[tauri::command]
pub async fn sso_login_wait(app: AppHandle, state: State<'_, LoginState>) -> Result<TokenStatus, String> {
    let (session, auth, cancel) = {
        let pending = state.pending.lock().await;
        let p = pending
            .as_ref()
            .ok_or_else(|| "No SSO login is in progress".to_string())?;
        (p.session.clone(), p.auth.clone(), p.cancel.clone())
    };

    let result = sso::oidc::poll_for_token(&session, &auth, cancel).await;

    {
        let mut pending = state.pending.lock().await;
        *pending = None;
    }

    result.map_err(String::from)?;

    let status = sso::token_status(&session.name)
        .ok_or_else(|| "Login succeeded but the token could not be read back".to_string())?;
    let _ = app.emit("token-status-changed", &status);
    Ok(status)
}

#[tauri::command]
pub async fn sso_login_cancel(state: State<'_, LoginState>) -> Result<(), String> {
    let pending = state.pending.lock().await;
    if let Some(p) = pending.as_ref() {
        p.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn sso_logout(app: AppHandle) -> Result<AppState, String> {
    let text = awsconfig::read_config_text().map_err(String::from)?;
    let session = require_session(&text)?;

    sso::cache::delete(&session.name).map_err(String::from)?;

    let cleared = TokenStatus {
        valid: false,
        expires_at: String::new(),
        remaining_seconds: 0,
        can_refresh: false,
    };
    let _ = app.emit("token-status-changed", &cleared);

    build_app_state().await
}

#[tauri::command]
pub async fn refresh_token_now(app: AppHandle) -> Result<TokenStatus, String> {
    let text = awsconfig::read_config_text().map_err(String::from)?;
    let session_info = require_session(&text)?;
    let session = SsoSession::from(&session_info);

    sso::oidc::refresh_token(&session).await.map_err(String::from)?;

    let status = sso::token_status(&session.name)
        .ok_or_else(|| "Refresh succeeded but the token could not be read back".to_string())?;
    let _ = app.emit("token-status-changed", &status);
    Ok(status)
}

#[tauri::command]
pub async fn list_accounts() -> Result<Vec<crate::discovery::AccountInfo>, String> {
    let text = awsconfig::read_config_text().map_err(String::from)?;
    let session = require_session(&text)?;
    crate::discovery::list_accounts(&session.sso_region, &session.name)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn get_managed_profiles() -> Result<Vec<ProfileInfo>, String> {
    let text = awsconfig::read_config_text().map_err(String::from)?;
    let session = require_session(&text)?;
    Ok(awsconfig::managed_profiles(&text, &session.name)
        .into_iter()
        .map(ProfileInfo::from)
        .collect())
}

#[tauri::command]
pub async fn apply_profiles(
    selections: Vec<Selection>,
    default_profile: Option<String>,
    region: String,
) -> Result<Vec<ProfileInfo>, String> {
    let mut text = awsconfig::read_config_text().map_err(String::from)?;
    let session = require_session(&text)?;

    let mut selected_names = Vec::with_capacity(selections.len());
    for sel in &selections {
        let profile_name = awsconfig::profile_name_for(&sel.account_name, &sel.role_name);
        let write = ProfileWrite {
            profile_name: profile_name.clone(),
            account_id: sel.account_id.clone(),
            role_name: sel.role_name.clone(),
            region: region.clone(),
        };
        text = awsconfig::upsert_profile(&text, &session.name, &write)?;
        selected_names.push(profile_name);
    }

    // Remove managed profiles that are no longer selected.
    for existing in awsconfig::managed_profiles(&text, &session.name) {
        if !selected_names.contains(&existing.profile_name) {
            text = awsconfig::remove_profile(&text, &session.name, &existing.profile_name);
        }
    }

    match &default_profile {
        Some(name) => {
            let sel = selections
                .iter()
                .find(|s| awsconfig::profile_name_for(&s.account_name, &s.role_name) == *name)
                .ok_or_else(|| format!("Default profile '{name}' is not among the selected profiles"))?;
            let write = ProfileWrite {
                profile_name: name.clone(),
                account_id: sel.account_id.clone(),
                role_name: sel.role_name.clone(),
                region: region.clone(),
            };
            text = awsconfig::set_default(&text, &session.name, &write)?;
        }
        None => {
            text = awsconfig::remove_default(&text, &session.name);
        }
    }

    awsconfig::write_config_text(&text).map_err(String::from)?;

    Ok(awsconfig::managed_profiles(&text, &session.name)
        .into_iter()
        .map(ProfileInfo::from)
        .collect())
}

#[tauri::command]
pub async fn verify_profile(profile_name: String) -> Result<crate::verify::Identity, String> {
    crate::verify::verify_profile(&profile_name).await.map_err(String::from)
}

#[tauri::command]
pub async fn export_credentials(profile_name: String, format: String) -> Result<String, String> {
    crate::verify::export_credentials(&profile_name, &format)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn check_cli() -> Result<CliStatus, String> {
    tokio::task::spawn_blocking(run_check_cli)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}
