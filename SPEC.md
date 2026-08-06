# AWS SSO Manager — Technical Specification

Desktop app (macOS / Linux / Windows) that lets a user log in to the company
AWS IAM Identity Center once (via whatever identity provider the company has
configured, e.g. Office 365 or Okta), pick AWS accounts/roles via checkboxes,
and have working AWS CLI profiles in `~/.aws/config` — so all `aws` commands
in any terminal just work.

**Stack**: Tauri v2, Rust backend (AWS SDK for Rust), React + TypeScript frontend (Vite).
No shelling out to the `aws` binary — SSO OIDC device flow is implemented natively.
The app only *checks* that AWS CLI v2 is installed (the user needs it for their own commands).

## 1. How it works (core insight)

All profiles share one `sso-session` (e.g. `acme`). One SSO login = one cached
token that covers every account/role. "Connecting to profiles" therefore means:

1. Ensure a valid SSO access token exists in `~/.aws/sso/cache/` (AWS-CLI-compatible format).
2. Ensure a `[profile ...]` entry exists in `~/.aws/config` for each selected account/role.

The AWS CLI then mints temporary credentials on demand. The app does not need to
keep running for the CLI to work; the tray merely watches token expiry.

## 2. Repository layout

```
aws-sso-manager/
├── SPEC.md                  # this file
├── branding.json            # white-label source of truth (see §11)
├── scripts/
│   └── gen-branding.mjs     # generates src/branding.ts, patches tauri.conf.json + index.html
├── src/                     # React frontend (owned by frontend agent)
│   ├── App.tsx              # single-window UI, see §7
│   ├── api.ts               # typed wrappers over Tauri invoke() — mirrors §6 exactly
│   ├── branding.ts          # AUTO-GENERATED from branding.json — do not edit directly
│   └── ...
├── src-tauri/
│   ├── Cargo.toml
│   ├── build.rs              # generates the Rust branding constants at compile time
│   ├── tauri.conf.json      # productName / identifier — patched by `npm run brand`
│   └── src/
│       ├── lib.rs           # plugin + command registration, state, tray setup
│       ├── error.rs         # AppError -> serializable string errors
│       ├── branding.rs      # exposes crate::branding::{APP_NAME, APP_ID} (see §11)
│       ├── sso/             # OWNED BY CORE (already implemented — do not rewrite)
│       │   ├── mod.rs
│       │   ├── cache.rs     # CLI-compatible token cache read/write
│       │   └── oidc.rs      # device flow login + refresh
│       ├── awsconfig.rs     # ~/.aws/config line-preserving parser/writer
│       ├── discovery.rs     # ListAccounts / ListAccountRoles
│       ├── verify.rs        # GetRoleCredentials + STS GetCallerIdentity, export creds
│       ├── tray.rs          # tray icon, expiry watcher, notifications
│       └── commands.rs      # #[tauri::command] handlers (thin wrappers)
└── .github/workflows/build.yml  # release build matrix (see §9)
```

## 3. Token cache — AWS CLI compatibility (CRITICAL)

Path: `~/.aws/sso/cache/<sha1_hex(session_name)>.json` where `session_name` is the
literal sso-session name (e.g. `acme`), UTF-8, SHA1 hex lowercase.

JSON fields (exactly these names):

```json
{
  "startUrl": "https://acme.awsapps.com/start",
  "region": "<sso_region>",
  "accessToken": "...",
  "expiresAt": "2026-08-05T21:30:00Z",
  "clientId": "...",
  "clientSecret": "...",
  "registrationExpiresAt": "2026-11-03T12:00:00Z",
  "refreshToken": "..."        // optional, only if issued
}
```

`expiresAt` format: `%Y-%m-%dT%H:%M:%SZ`, always UTC. File permissions 0600, dir 0700.
This is what botocore/AWS CLI v2 reads; compatibility is verified by
`aws sts get-caller-identity --profile ...` succeeding without `aws sso login`.

## 4. SSO login flow (implemented in `sso/oidc.rs`)

Device authorization grant against `ssooidc` in the **sso_region**, unauthenticated
clients (`aws_config` with `.no_credentials()`):

1. `RegisterClient` (client_name = the app name constant, e.g. `AWS SSO Manager`, type `public`, scopes `["sso:account:access"]`).
   Registration is reused from the cache file while `registrationExpiresAt` is valid.
2. `StartDeviceAuthorization` (clientId, clientSecret, startUrl).
3. Open `verificationUriComplete` in the system browser (tauri-plugin-opener);
   UI also shows `userCode` + `verificationUri` for manual entry.
4. Poll `CreateToken` (grant `urn:ietf:params:oauth:grant-type:device_code`)
   every `interval` seconds; `AuthorizationPendingException` → keep waiting,
   `SlowDownException` → interval += 5, `ExpiredTokenException` → abort with clear error.
5. Write cache file (§3).

Refresh: if `refreshToken` present, `CreateToken` with grant `refresh_token`.
Used by the tray watcher to extend the session silently; on failure fall back
to prompting a full login.

## 5. `~/.aws/config` editing rules (`awsconfig.rs`)

- Line-based editor. Never reformat, reorder, or drop anything the app does not own.
  Only replace/insert whole sections (`[profile X]` / `[sso-session Y]`) that the app manages.
- A profile is "managed" iff it has `sso_session = <our session name>`.
- Before the first modifying write in an app run, copy `config` to `config.bak` (overwrite).
- Written profile shape (matches company guide):

```ini
[profile acme-admin-users]
sso_session = acme
sso_account_id = 111122223333
sso_role_name = admin-users
region = eu-central-1
output = json
```

- `[sso-session <name>]` section written if missing:
  `sso_start_url`, `sso_region`, `sso_registration_scopes = sso:account:access`.
- **Profile naming**: `<accountName>-<roleName>` with longest-overlap collapse:
  suffix of accountName that equals a prefix of roleName is not repeated.
  `acme-admin` + `admin-users` → `acme-admin-users`. No overlap → plain join.
  (Split on `-`; compare whole dash-separated segments, case-insensitive.)
- **Default profile**: if the user marks a profile as default, write a `[default]`
  section with the same keys (sso_session/account/role/region/output). Only touch
  `[default]` if it is absent or already managed (has our `sso_session`); otherwise
  refuse and report — never overwrite a foreign `[default]`.
- Removing a profile removes only its managed section.
- If `~/.aws/config` does not exist, create it (0600).

## 6. IPC command API (contract between backend and frontend)

All commands are `#[tauri::command]`, async where I/O-bound, return
`Result<T, String>` (errors are human-readable English sentences).
Serde: `#[serde(rename_all = "camelCase")]` on all payload structs.

| Command | Args | Returns |
|---|---|---|
| `get_app_state` | – | `AppState` |
| `save_sso_session` | `name, startUrl, ssoRegion` | `AppState` (writes sso-session section) |
| `sso_login_start` | – | `LoginPrompt { userCode, verificationUri, verificationUriComplete }` (also opens browser) |
| `sso_login_wait` | – | `TokenStatus` (blocks until token obtained or flow fails/expires) |
| `sso_login_cancel` | – | `null` (aborts pending poll) |
| `sso_logout` | – | `AppState` (deletes cache file) |
| `refresh_token_now` | – | `TokenStatus` |
| `list_accounts` | – | `AccountInfo[]` — aggregated roles per account, sorted by name |
| `get_managed_profiles` | – | `ProfileInfo[]` |
| `apply_profiles` | `selections: Selection[], defaultProfile: string \| null, region: string` | `ProfileInfo[]` (idempotent; also removes managed profiles NOT in `selections`) |
| `verify_profile` | `profileName` | `Identity { arn, accountId, userId }` |
| `export_credentials` | `profileName, format: "env" \| "powershell"` | `string` (shell-ready export lines) |
| `check_cli` | – | `CliStatus { installed, version }` |
| `copy_text` | `text` | `null` (copies to system clipboard via tauri-plugin-clipboard-manager on the Rust side — the frontend must NOT add any npm clipboard package) |

```ts
interface AppState {
  cliInstalled: boolean;
  session: { name: string; startUrl: string; ssoRegion: string } | null; // from ~/.aws/config
  token: TokenStatus | null;
}
interface TokenStatus { valid: boolean; expiresAt: string; remainingSeconds: number; canRefresh: boolean; }
interface AccountInfo { accountId: string; accountName: string; email: string; roles: string[]; }
interface Selection { accountId: string; accountName: string; roleName: string; }
interface ProfileInfo { profileName: string; accountId: string; roleName: string; region: string; isDefault: boolean; }
```

Backend event → frontend: `token-status-changed` with `TokenStatus` payload
(emitted by tray watcher every 60 s and after login/logout/refresh).

`verify_profile` / `export_credentials` do NOT shell out: they call SSO
`GetRoleCredentials` with the cached access token, then (for verify) STS
`GetCallerIdentity` with those static credentials. Export formats:
`env` → three `export VAR=...` lines (+ `export AWS_PROFILE=...` omitted);
`powershell` → `$env:VAR = "..."` lines.

## 7. UI (single window, ~800×600, React)

State machine driven by `AppState`:

1. **First-run wizard** (no `session` in config): form for session name
   (default from `branding.defaults.sessionName`, e.g. `acme`), SSO start URL,
   SSO region; short helper text taken from the company guide (where to find
   the values: access portal → Access keys dialog).
2. **Login screen** (session known, token missing/expired): big "Sign in with
   `<idpLabel>` SSO" button (idpLabel from branding, e.g. "SSO" or "Office 365")
   → `sso_login_start` → show the user code prominently with "Confirm in your
   browser…" spinner → `sso_login_wait`.
3. **Main screen** (token valid):
   - Header: session name, token expiry countdown (from `token-status-changed`),
     Refresh + Logout buttons; warning banner if AWS CLI v2 not installed.
   - Account list from `list_accounts`: one row per account (name, ID, email),
     checkbox per role; roles already in config are pre-checked (match against
     `get_managed_profiles`).
   - Default-profile radio (optional) among checked roles; region select
     (default `eu-central-1`) applied to written profiles.
   - "Apply" → `apply_profiles` → per-profile ✓/✗ via `verify_profile`,
     showing the resolved ARN on success.
   - Per-profile row actions: "Copy env exports" (`export_credentials` →
     clipboard via tauri-plugin-clipboard-manager, format by OS).
   - Footer hint: `export AWS_PROFILE=<name>` / usable command examples.
4. Errors surface as dismissible inline banners, never modal `alert()`.

Styling: plain hand-written CSS (no UI framework), system font stack, light/dark
via `prefers-color-scheme`. Keep it clean and compact — a tool, not a marketing page.

## 8. Tray (`tray.rs`)

- Tray icon with menu: status line ("SSO: valid, expires in 5 h 12 m" / "SSO: expired"),
  "Open AWS SSO Manager", "Refresh session", "Quit".
- Background watcher task (tokio, every 60 s): recompute status, emit
  `token-status-changed`, update tray menu text.
- At <30 min remaining: try silent refresh if `canRefresh`; if that fails or
  is unavailable, send one desktop notification (tauri-plugin-notification)
  "AWS SSO session expires in 30 minutes — click to re-login". Second (final)
  notification at expiry. No notification spam: each threshold fires once per session.
- Closing the window hides it (app keeps running in tray); Quit from tray exits.

## 9. CI / packaging (`.github/workflows/build.yml`)

- Trigger: push tag `v*` + `workflow_dispatch`.
- Matrix: `macos-latest` (aarch64 + x86_64 dmg), `ubuntu-22.04` (deb + AppImage),
  `windows-latest` (msi). Use `tauri-apps/tauri-action@v0` with `npm`.
- Ubuntu needs the standard Tauri v2 apt deps (`libwebkit2gtk-4.1-dev`, etc.).
- Unsigned builds are fine for v1 (document Gatekeeper right-click-open in README).

## 10. Testing

- Rust unit tests: profile-name collapse rule, config editor (round-trip fixture
  with foreign profiles + comments must survive byte-identical outside managed
  sections), cache filename sha1, expiresAt formatting.
- `cargo check` and `npm run build` must pass; `cargo test` green.
- Manual E2E (documented in README): login → apply → `aws sts get-caller-identity
  --profile ...` in a fresh terminal without running `aws sso login`.

## 11. Branding / white-labeling layer

This app is built to be re-branded per company without touching source code. There
are two distinct layers of "company-specific" data — do not confuse them:

- **Build-time branding** (`branding.json`, committed, one value per build/release):
  product identity — app name, app id/bundle identifier, the identity-provider label
  shown on the login button, company name, support URL, app icon source, and the
  *default* values pre-filled into the first-run wizard (session name, start URL,
  SSO region, region presets). This is baked into the binary at build time.
- **Per-user runtime config** (`~/.aws/config`, never committed): the actual
  sso-session and profiles a given user has set up on their machine, written by the
  app at runtime per §5. Two different employees at the same company share the same
  build-time branding but have independently-populated `~/.aws/config` files.

Pipeline, source of truth → generated artifacts:

```
branding.json
  └─ scripts/gen-branding.mjs ("npm run brand", also runs as the npm prebuild hook)
       ├─ src/branding.ts            # `branding` object consumed by the React frontend
       ├─ src-tauri/tauri.conf.json  # patches productName + identifier
       └─ index.html                 # patches <title>
  └─ src-tauri/build.rs (Cargo build script, reads branding.json directly)
       └─ src-tauri/src/branding.rs  # exposes crate::branding::{APP_NAME, APP_ID}
```

Frontend code imports `{ branding }` from `src/branding.ts` (e.g.
`branding.appName`, `branding.idpLabel`, `branding.defaults.sessionName`).
Rust code references `crate::branding::APP_NAME` / `crate::branding::APP_ID`
directly — no hardcoded product-name string literals belong anywhere else in
either codebase.

To re-brand: edit `branding.json`, run `npm run brand`, commit the regenerated
`src/branding.ts` + patched `tauri.conf.json`/`index.html`, then regenerate the
icon separately with `npx tauri icon <path>` before the next `tauri build`. See
README.md → "White-labeling" for the user-facing walkthrough.
