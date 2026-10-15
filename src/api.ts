import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---- Types (mirrors SPEC.md §6 exactly, camelCase) ----

export interface SsoSession {
  name: string;
  startUrl: string;
  ssoRegion: string;
}

export interface TokenStatus {
  valid: boolean;
  expiresAt: string;
  remainingSeconds: number;
  canRefresh: boolean;
}

export interface AppState {
  cliInstalled: boolean;
  session: SsoSession | null;
  token: TokenStatus | null;
}

export interface LoginPrompt {
  userCode: string;
  verificationUri: string;
  verificationUriComplete: string;
  browserOpened: boolean;
  wslviewMissing: boolean;
}

export interface AccountInfo {
  accountId: string;
  accountName: string;
  email: string;
  roles: string[];
}

export interface Selection {
  accountId: string;
  accountName: string;
  roleName: string;
}

export interface ProfileInfo {
  profileName: string;
  accountId: string;
  roleName: string;
  region: string;
  isDefault: boolean;
}

export interface Identity {
  arn: string;
  accountId: string;
  userId: string;
}

export interface CliStatus {
  installed: boolean;
  version: string | null;
}

export type ExportFormat = "env" | "powershell";

// ---- Command wrappers ----

export function getAppState(): Promise<AppState> {
  return invoke<AppState>("get_app_state");
}

export function saveSsoSession(
  name: string,
  startUrl: string,
  ssoRegion: string,
): Promise<AppState> {
  return invoke<AppState>("save_sso_session", { name, startUrl, ssoRegion });
}

export function ssoLoginStart(): Promise<LoginPrompt> {
  return invoke<LoginPrompt>("sso_login_start");
}

export function ssoLoginWait(): Promise<TokenStatus> {
  return invoke<TokenStatus>("sso_login_wait");
}

export function ssoLoginCancel(): Promise<null> {
  return invoke<null>("sso_login_cancel");
}

export function ssoLogout(): Promise<AppState> {
  return invoke<AppState>("sso_logout");
}

export function refreshTokenNow(): Promise<TokenStatus> {
  return invoke<TokenStatus>("refresh_token_now");
}

export function listAccounts(): Promise<AccountInfo[]> {
  return invoke<AccountInfo[]>("list_accounts");
}

export function getManagedProfiles(): Promise<ProfileInfo[]> {
  return invoke<ProfileInfo[]>("get_managed_profiles");
}

export function applyProfiles(
  selections: Selection[],
  defaultProfile: string | null,
  region: string,
): Promise<ProfileInfo[]> {
  return invoke<ProfileInfo[]>("apply_profiles", {
    selections,
    defaultProfile,
    region,
  });
}

export function verifyProfile(profileName: string): Promise<Identity> {
  return invoke<Identity>("verify_profile", { profileName });
}

export function exportCredentials(
  profileName: string,
  format: ExportFormat,
): Promise<string> {
  return invoke<string>("export_credentials", { profileName, format });
}

export function checkCli(): Promise<CliStatus> {
  return invoke<CliStatus>("check_cli");
}

export function copyText(text: string): Promise<null> {
  return invoke<null>("copy_text", { text });
}

/**
 * Subscribe to the backend "token-status-changed" event (emitted by the tray
 * watcher every 60s and after login/logout/refresh). Returns the unlisten
 * function.
 */
export function subscribeTokenStatus(
  handler: (status: TokenStatus) => void,
): Promise<UnlistenFn> {
  return listen<TokenStatus>("token-status-changed", (event) => {
    handler(event.payload);
  });
}
