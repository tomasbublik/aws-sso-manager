//! `ListAccounts` / `ListAccountRoles` against the cached SSO access token
//! (SPEC.md §6). No `aws` binary involved — talks to the `sso` service
//! directly with an unauthenticated client carrying the bearer token.

use aws_config::BehaviorVersion;
use aws_sdk_sso::config::Region;
use aws_sdk_sso::Client as SsoClient;
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::sso::cache;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub account_id: String,
    pub account_name: String,
    pub email: String,
    pub roles: Vec<String>,
}

async fn sso_client(sso_region: &str) -> SsoClient {
    let cfg = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(sso_region.to_string()))
        .no_credentials()
        .load()
        .await;
    SsoClient::new(&cfg)
}

fn session_expired_err() -> AppError {
    AppError::msg("SSO session expired — please sign in again")
}

/// Loads the cached bearer token for `session_name`, or an error telling the
/// user to sign in.
fn access_token(session_name: &str) -> AppResult<String> {
    cache::read(session_name)
        .and_then(|e| e.access_token)
        .ok_or_else(|| AppError::msg("No cached SSO access token — please sign in"))
}

/// Lists every account/role pair the signed-in user can access, aggregated
/// per account and sorted by account name (roles sorted within each
/// account), per SPEC.md §6.
pub async fn list_accounts(sso_region: &str, session_name: &str) -> AppResult<Vec<AccountInfo>> {
    let access_token = access_token(session_name)?;
    let client = sso_client(sso_region).await;

    let mut accounts: Vec<AccountInfo> = Vec::new();
    let mut next_token: Option<String> = None;

    loop {
        let mut req = client.list_accounts().access_token(&access_token);
        if let Some(t) = &next_token {
            req = req.next_token(t);
        }
        let resp = req.send().await.map_err(map_list_accounts_err)?;

        for acct in resp.account_list() {
            let account_id = acct.account_id().unwrap_or_default().to_string();
            let account_name = acct.account_name().unwrap_or_default().to_string();
            let email = acct.email_address().unwrap_or_default().to_string();

            let mut roles = list_account_roles(&client, &access_token, &account_id).await?;
            roles.sort_by_key(|r| r.to_lowercase());

            accounts.push(AccountInfo {
                account_id,
                account_name,
                email,
                roles,
            });
        }

        next_token = resp.next_token().filter(|t| !t.is_empty()).map(str::to_string);
        if next_token.is_none() {
            break;
        }
    }

    accounts.sort_by_key(|a| a.account_name.to_lowercase());
    Ok(accounts)
}

async fn list_account_roles(
    client: &SsoClient,
    access_token: &str,
    account_id: &str,
) -> AppResult<Vec<String>> {
    let mut roles = Vec::new();
    let mut next_token: Option<String> = None;

    loop {
        let mut req = client
            .list_account_roles()
            .access_token(access_token)
            .account_id(account_id);
        if let Some(t) = &next_token {
            req = req.next_token(t);
        }
        let resp = req.send().await.map_err(map_list_roles_err)?;

        for role in resp.role_list() {
            if let Some(name) = role.role_name() {
                roles.push(name.to_string());
            }
        }

        next_token = resp.next_token().filter(|t| !t.is_empty()).map(str::to_string);
        if next_token.is_none() {
            break;
        }
    }
    Ok(roles)
}

fn map_list_accounts_err(
    e: aws_sdk_sso::error::SdkError<aws_sdk_sso::operation::list_accounts::ListAccountsError>,
) -> AppError {
    if let Some(svc) = e.as_service_error() {
        if svc.is_unauthorized_exception() {
            return session_expired_err();
        }
    }
    AppError::msg(format!(
        "Failed to list AWS accounts: {}",
        aws_sdk_sso::error::DisplayErrorContext(&e)
    ))
}

fn map_list_roles_err(
    e: aws_sdk_sso::error::SdkError<aws_sdk_sso::operation::list_account_roles::ListAccountRolesError>,
) -> AppError {
    if let Some(svc) = e.as_service_error() {
        if svc.is_unauthorized_exception() {
            return session_expired_err();
        }
    }
    AppError::msg(format!(
        "Failed to list roles: {}",
        aws_sdk_sso::error::DisplayErrorContext(&e)
    ))
}
