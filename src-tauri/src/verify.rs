//! `GetRoleCredentials` + STS `GetCallerIdentity`, and shell-ready credential
//! export (SPEC.md §6). No `aws` binary involved.

use aws_config::BehaviorVersion;
use aws_sdk_sso::config::Region as SsoRegion;
use aws_sdk_sso::Client as SsoClient;
use aws_sdk_sts::config::{Credentials, Region as StsRegion};
use aws_sdk_sts::Client as StsClient;
use serde::Serialize;

use crate::awsconfig::{self, ManagedProfile};
use crate::error::{AppError, AppResult};
use crate::sso::cache;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub arn: String,
    pub account_id: String,
    pub user_id: String,
}

struct ResolvedCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
}

/// Looks up a managed profile's account id / role name / region by name via
/// `~/.aws/config`.
fn resolve_profile(profile_name: &str) -> AppResult<ManagedProfile> {
    let text = awsconfig::read_config_text()?;
    let session = awsconfig::first_sso_session(&text)
        .ok_or_else(|| AppError::msg("No SSO session configured"))?;
    awsconfig::managed_profiles(&text, &session.name)
        .into_iter()
        .find(|p| p.profile_name == profile_name)
        .ok_or_else(|| AppError::msg(format!("Unknown profile '{profile_name}'")))
}

async fn get_role_credentials(
    sso_region: &str,
    session_name: &str,
    account_id: &str,
    role_name: &str,
) -> AppResult<ResolvedCredentials> {
    let access_token = cache::read(session_name)
        .and_then(|e| e.access_token)
        .ok_or_else(|| AppError::msg("No cached SSO access token — please sign in"))?;

    let cfg = aws_config::defaults(BehaviorVersion::latest())
        .region(SsoRegion::new(sso_region.to_string()))
        .no_credentials()
        .load()
        .await;
    let client = SsoClient::new(&cfg);

    let resp = client
        .get_role_credentials()
        .access_token(&access_token)
        .account_id(account_id)
        .role_name(role_name)
        .send()
        .await
        .map_err(|e| {
            if let Some(svc) = e.as_service_error() {
                if svc.is_unauthorized_exception() {
                    return AppError::msg("SSO session expired — please sign in again");
                }
            }
            AppError::msg(format!(
                "Failed to get role credentials: {}",
                aws_sdk_sso::error::DisplayErrorContext(&e)
            ))
        })?;

    let creds = resp
        .role_credentials()
        .ok_or_else(|| AppError::msg("AWS SSO returned no role credentials"))?;

    Ok(ResolvedCredentials {
        access_key_id: creds.access_key_id().unwrap_or_default().to_string(),
        secret_access_key: creds.secret_access_key().unwrap_or_default().to_string(),
        session_token: creds.session_token().unwrap_or_default().to_string(),
    })
}

/// `GetRoleCredentials` then STS `GetCallerIdentity` with those static
/// credentials — proves the profile actually resolves to a working identity.
pub async fn verify_profile(profile_name: &str) -> AppResult<Identity> {
    let profile = resolve_profile(profile_name)?;
    let text = awsconfig::read_config_text()?;
    let session = awsconfig::first_sso_session(&text)
        .ok_or_else(|| AppError::msg("No SSO session configured"))?;

    let creds = get_role_credentials(
        &session.sso_region,
        &session.name,
        &profile.account_id,
        &profile.role_name,
    )
    .await?;

    let credentials = Credentials::new(
        creds.access_key_id,
        creds.secret_access_key,
        Some(creds.session_token),
        None,
        crate::branding::APP_NAME,
    );

    let sts_cfg = aws_config::defaults(BehaviorVersion::latest())
        .region(StsRegion::new(profile.region.clone()))
        .credentials_provider(credentials)
        .load()
        .await;
    let sts_client = StsClient::new(&sts_cfg);

    let resp = sts_client.get_caller_identity().send().await.map_err(|e| {
        AppError::msg(format!(
            "STS GetCallerIdentity failed: {}",
            aws_sdk_sts::error::DisplayErrorContext(&e)
        ))
    })?;

    Ok(Identity {
        arn: resp.arn().unwrap_or_default().to_string(),
        account_id: resp.account().unwrap_or_default().to_string(),
        user_id: resp.user_id().unwrap_or_default().to_string(),
    })
}

/// Formats a profile's temporary role credentials as shell-ready export
/// lines (SPEC.md §6): `env` -> `export VAR='...'`, `powershell` -> `$env:VAR
/// = "..."`.
pub async fn export_credentials(profile_name: &str, format: &str) -> AppResult<String> {
    let profile = resolve_profile(profile_name)?;
    let text = awsconfig::read_config_text()?;
    let session = awsconfig::first_sso_session(&text)
        .ok_or_else(|| AppError::msg("No SSO session configured"))?;

    let creds = get_role_credentials(
        &session.sso_region,
        &session.name,
        &profile.account_id,
        &profile.role_name,
    )
    .await?;

    Ok(match format {
        "powershell" => format!(
            "$env:AWS_ACCESS_KEY_ID = \"{}\"\n$env:AWS_SECRET_ACCESS_KEY = \"{}\"\n$env:AWS_SESSION_TOKEN = \"{}\"\n",
            creds.access_key_id, creds.secret_access_key, creds.session_token
        ),
        _ => format!(
            "export AWS_ACCESS_KEY_ID='{}'\nexport AWS_SECRET_ACCESS_KEY='{}'\nexport AWS_SESSION_TOKEN='{}'\n",
            creds.access_key_id, creds.secret_access_key, creds.session_token
        ),
    })
}
