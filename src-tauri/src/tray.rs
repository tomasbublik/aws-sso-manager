//! Tray icon, background expiry watcher and notifications (SPEC.md §8).

use std::time::Duration;

use tauri::menu::{Menu, MenuItem, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

use crate::awsconfig;
use crate::sso::{self, SsoSession, TokenStatus};

/// Below this many remaining seconds we try a silent refresh / fire the
/// "expires soon" notification (SPEC.md §8: "< 30 min").
const WARN_THRESHOLD_SECONDS: i64 = 30 * 60;
const POLL_INTERVAL: Duration = Duration::from_secs(60);

fn status_text(status: Option<&TokenStatus>) -> String {
    match status {
        None => "SSO: not signed in".to_string(),
        Some(s) if !s.valid => "SSO: expired".to_string(),
        Some(s) => {
            let total = s.remaining_seconds.max(0);
            format!("SSO: valid, expires in {} h {} m", total / 3600, (total % 3600) / 60)
        }
    }
}

fn notify<R: Runtime>(app: &AppHandle<R>, body: &str) {
    let _ = app
        .notification()
        .builder()
        .title(crate::branding::APP_NAME)
        .body(body)
        .show();
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Registers the tray icon + menu and spawns the background expiry watcher.
/// Called once from `lib.rs`'s `.setup()`.
pub fn init<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let status_item = MenuItemBuilder::with_id("status", status_text(None))
        .enabled(false)
        .build(app)?;
    let open_item =
        MenuItemBuilder::with_id("open", format!("Open {}", crate::branding::APP_NAME)).build(app)?;
    let refresh_item = MenuItemBuilder::with_id("refresh", "Refresh session").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = Menu::with_items(app, &[&status_item, &open_item, &refresh_item, &quit_item])?;

    // The watcher task and the "Refresh session" click handler each need
    // their own handle to the status item (both are cheap Arc clones of the
    // same underlying native menu item).
    let status_item_for_menu = status_item.clone();
    let status_item_for_watcher = status_item;

    let mut tray_builder = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip(crate::branding::APP_NAME)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            "refresh" => {
                let app = app.clone();
                let status_item = status_item_for_menu.clone();
                tauri::async_runtime::spawn(async move {
                    manual_refresh(&app, &status_item).await;
                });
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        tray_builder = tray_builder.icon(icon.clone());
    }

    tray_builder.build(app)?;

    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        watch_loop(handle, status_item_for_watcher).await;
    });

    Ok(())
}

async fn manual_refresh<R: Runtime>(app: &AppHandle<R>, status_item: &MenuItem<R>) {
    let Ok(text) = awsconfig::read_config_text() else {
        return;
    };
    let Some(session_info) = awsconfig::first_sso_session(&text) else {
        return;
    };
    let session = SsoSession::from(&session_info);
    let _ = sso::oidc::refresh_token(&session).await;

    if let Some(status) = sso::token_status(&session_info.name) {
        let _ = status_item.set_text(status_text(Some(&status)));
        let _ = app.emit("token-status-changed", &status);
    }
}

async fn watch_loop<R: Runtime>(app: AppHandle<R>, status_item: MenuItem<R>) {
    let mut warn_fired = false;
    let mut expiry_fired = false;
    loop {
        tick(&app, &status_item, &mut warn_fired, &mut expiry_fired).await;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn tick<R: Runtime>(
    app: &AppHandle<R>,
    status_item: &MenuItem<R>,
    warn_fired: &mut bool,
    expiry_fired: &mut bool,
) {
    let Ok(text) = awsconfig::read_config_text() else {
        return;
    };
    let Some(session_info) = awsconfig::first_sso_session(&text) else {
        let _ = status_item.set_text(status_text(None));
        return;
    };

    let mut status = sso::token_status(&session_info.name);
    let Some(current) = status.clone() else {
        let _ = status_item.set_text(status_text(None));
        return;
    };

    if current.remaining_seconds > WARN_THRESHOLD_SECONDS {
        // Fresh login (or plenty of time left) — reset one-shot flags.
        *warn_fired = false;
        *expiry_fired = false;
    } else if current.remaining_seconds > 0 {
        if !*warn_fired {
            let mut refreshed = false;
            if current.can_refresh {
                let session = SsoSession::from(&session_info);
                if sso::oidc::refresh_token(&session).await.is_ok() {
                    refreshed = true;
                    status = sso::token_status(&session_info.name);
                }
            }
            if !refreshed {
                notify(app, "AWS SSO session expires in 30 minutes — click to re-login");
            }
            *warn_fired = true;
        }
    } else if !*expiry_fired {
        notify(app, "AWS SSO session has expired — please sign in again");
        *expiry_fired = true;
    }

    let _ = status_item.set_text(status_text(status.as_ref()));
    if let Some(status) = &status {
        let _ = app.emit("token-status-changed", status);
    }
}
