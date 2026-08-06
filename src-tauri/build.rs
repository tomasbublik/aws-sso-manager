use std::env;
use std::fs;
use std::path::Path;

/// Reads ../branding.json and emits `branding_generated.rs` (constants used by
/// the app, see src/branding.rs). Keeping this in build.rs means the Rust
/// constants are always regenerated from the single source of truth with no
/// committed generated file.
fn gen_branding() {
    // Which branding file to apply. Defaults to the generic `branding.json`;
    // override for a company-specific build, e.g.
    //   BRANDING_FILE=branding.acme.json npm run tauri build
    // (the env var must be set for the same invocation so this and the JS
    // generator read the same source).
    let branding_file = env::var("BRANDING_FILE").unwrap_or_else(|_| "branding.json".to_string());
    println!("cargo:rerun-if-env-changed=BRANDING_FILE");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let branding_path = Path::new(&manifest_dir).join("..").join(&branding_file);
    println!("cargo:rerun-if-changed={}", branding_path.display());

    // Neutral fallbacks so the crate still builds if branding.json is absent.
    let mut app_name = "AWS SSO Manager".to_string();
    let mut app_id = "com.example.aws-sso-manager".to_string();

    if let Ok(text) = fs::read_to_string(&branding_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(v) = json.get("appName").and_then(|v| v.as_str()) {
                app_name = v.to_string();
            }
            if let Some(v) = json.get("appId").and_then(|v| v.as_str()) {
                app_id = v.to_string();
            }
        }
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("branding_generated.rs");
    let contents = format!(
        "pub const APP_NAME: &str = {app_name:?};\n\
         pub const APP_ID: &str = {app_id:?};\n"
    );
    fs::write(&dest, contents).expect("failed to write branding_generated.rs");
}

fn main() {
    gen_branding();
    tauri_build::build()
}
