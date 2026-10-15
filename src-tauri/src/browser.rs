//! Prefer the Windows browser when the Linux app is running under WSL.

#[derive(Default)]
pub struct WindowsBrowserResult {
    pub opened: bool,
    pub wslview_missing: bool,
}

#[cfg(target_os = "linux")]
pub fn try_windows_browser(url: &str) -> WindowsBrowserResult {
    let is_wsl = std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::env::var_os("WSL_INTEROP").is_some()
        || std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|release| release.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false);

    if !is_wsl {
        return WindowsBrowserResult::default();
    }

    // Pass the URL directly as an argument, never through a shell. Suppress
    // helper output because the authorization URL contains a login code.
    let result = std::process::Command::new("wslview")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    WindowsBrowserResult {
        opened: result.as_ref().map(|status| status.success()).unwrap_or(false),
        wslview_missing: matches!(result, Err(error) if error.kind() == std::io::ErrorKind::NotFound),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn try_windows_browser(_url: &str) -> WindowsBrowserResult {
    WindowsBrowserResult::default()
}
