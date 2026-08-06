use std::fmt;

/// Application error carrying a human-readable message that is safe to show in the UI.
#[derive(Debug)]
pub struct AppError(pub String);

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn msg(m: impl Into<String>) -> Self {
        AppError(m.into())
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError(format!("I/O error: {e}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError(format!("JSON error: {e}"))
    }
}

impl From<AppError> for String {
    fn from(e: AppError) -> Self {
        e.0
    }
}
