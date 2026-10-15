pub mod awsconfig;
pub mod branding;
mod browser;
pub mod commands;
pub mod discovery;
pub mod error;
pub mod sso;
pub mod tray;
pub mod verify;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(commands::LoginState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::save_sso_session,
            commands::sso_login_start,
            commands::sso_login_wait,
            commands::sso_login_cancel,
            commands::sso_logout,
            commands::refresh_token_now,
            commands::list_accounts,
            commands::get_managed_profiles,
            commands::apply_profiles,
            commands::verify_profile,
            commands::export_credentials,
            commands::check_cli,
            commands::copy_text,
        ])
        .setup(|app| {
            tray::init(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Keep the app running in the tray instead of quitting; only
                // the tray's "Quit" item actually exits (SPEC.md §8).
                window.hide().ok();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
