# AWS SSO Manager

Desktop manager for AWS IAM Identity Center CLI profiles, authenticated through your identity provider (e.g. Office 365, Okta). Sign in once via the app, select AWS accounts and roles from checkboxes, and all `aws` CLI commands in any terminal just work -- no need to run `aws sso login` repeatedly.

Cross-platform (macOS, Linux, Windows), built with **Tauri v2** -- a Rust backend and a React + TypeScript frontend in a ~10 MB binary.

## Features

- **One login, many profiles** -- pick any number of accounts/roles with checkboxes and apply them at once.
- **Dynamic discovery** -- accounts and roles are fetched live from AWS, so nothing is hard-coded and new accounts appear automatically.
- **Tray with expiry watch** -- a background watcher tracks token expiry, silently refreshes when possible, and notifies you before a re-login is needed.
- **Safe `~/.aws/config` edits** -- only profiles the app manages are touched; foreign profiles, comments, and a foreign `[default]` are never overwritten, and a backup is made before the first write.
- **Optional default profile** -- mark one profile as `[default]` so `aws` works without `--profile`.
- **Credential export** -- copy shell-ready `env` / PowerShell exports for tools that don't understand SSO profiles.
- **White-label ready** -- re-brand for any organization from a single `branding.json` (see below).

## How it works

- **Native OIDC device flow**: App implements SSO login natively without shelling out to the AWS CLI binary.
- **CLI-compatible token cache**: Access tokens are stored in `~/.aws/sso/cache/` in the exact format AWS CLI v2 reads.
- **Unified sso-session**: All selected profiles share one cached login; picking accounts/roles just writes new `[profile ...]` sections to `~/.aws/config`.
- **Minimal dependencies**: The app requires AWS CLI v2 to be installed (for your own `aws` commands), but does not invoke it directly; runs standalone in the system tray.

## Prerequisites

- **AWS CLI v2** (required for actual use; the app does not ship it).
- **SSO Start URL** and **SSO Region**: Find these in your AWS Access Portal -> "Access keys" dialog.

## Development

```bash
npm install
npm run tauri dev       # Run frontend + backend in dev mode
```

In `src-tauri`:
```bash
cargo test              # Run Rust unit tests
```

Frontend tests and type checks:
```bash
npm run build           # TypeScript + Vite build
```

## Building releases

Tag a commit with `v*` (e.g., `v0.1.0`) and push. GitHub Actions automatically builds and publishes:
- **macOS**: `.dmg` (universal binary covering both Apple Silicon + Intel).
- **Linux**: `.deb` + `.AppImage`.
- **Windows**: `.msi`.

**macOS note**: Builds are unsigned. On first launch, right-click the app -> "Open" to pass Gatekeeper.

## White-labeling

This app can be re-branded for a different company without touching source code:

1. Edit `branding.json` at the repo root -- `appName`, `appId`, `idpLabel`, `companyName`,
   `supportUrl`, `icon`, and `defaults` (`sessionName`, `startUrl`, `ssoRegion`, `regionPresets`).
2. Run `npm run brand` to regenerate `src/branding.ts` and patch `tauri.conf.json` +
   `index.html` from the new values.
3. Commit the regenerated files. Because Tauri reads `tauri.conf.json` at startup (not
   `branding.json`), the `productName`/`identifier` change is only picked up by the next
   `tauri build` if you've run `npm run brand` and committed the result first.
4. The app icon is regenerated separately -- it is not derived automatically from
   `branding.json`. Run `npx tauri icon <path-to-source-image>` after pointing `icon` in
   `branding.json` at the new source asset.

The committed `branding.json` is intentionally generic so the repo stays shareable. To
build a company-specific variant without overwriting that default, keep its values in a
separate file (e.g. `branding.acme.json`) and select it via the `BRANDING_FILE` env var --
both the JS generator and the Rust `build.rs` honour it, so set it for the whole build:

```bash
BRANDING_FILE=branding.acme.json npm run brand       # regenerate branded config
BRANDING_FILE=branding.acme.json npm run tauri build # bundle the branded app
```

Any `branding.*.json` other than the generic `branding.json` is git-ignored, so private
company branding never lands in the public repository.

Leave `BRANDING_FILE` unset for the default generic build.

## Running in WSL

When running in WSL, the manager tries `wslview` (provided by
[wslu](https://wslutiliti.es/wslu/install.html)) to open the sign-in page in
your default Windows browser. Install wslu for your distribution and ensure
`wslview` is on the manager's `PATH`, with WSL Windows interoperability enabled.
You can check it from your WSL terminal with:

```bash
wslview 'https://aws.amazon.com'
```

If `wslview` is unavailable or fails, the manager falls back to its normal URL
opener. When `wslview` is missing in WSL, the login screen also recommends
installing wslu, alongside the manual login link and code.
If neither opener succeeds, the login screen tells you to open the
displayed URL and enter the code manually. Authorization completes in the
Windows browser; the manager keeps polling AWS and saves credentials in WSL.

## Manual end-to-end test

1. Open AWS SSO Manager.
2. Sign in with your identity provider (follow the browser prompt).
3. Select one or more accounts/roles; click "Apply".
4. In a **fresh terminal**, run:
   ```bash
   aws sts get-caller-identity --profile <profile_name>
   ```
   This must succeed without running `aws sso login` -- the cached token is already there.
5. Verify the output shows the expected AWS account ID and role.

## Architecture

See [SPEC.md](SPEC.md) for the full technical specification: the CLI-compatible token
cache format, the OIDC device flow, the `~/.aws/config` editing rules, the IPC command
API between the Rust backend and the frontend, and the branding/white-labeling layer.

## License

Released under the [MIT License](LICENSE).
