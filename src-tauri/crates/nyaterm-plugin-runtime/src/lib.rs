//! Tauri-independent installation, manifest validation and sidecar transport.
pub mod diagnostics;
pub mod manifest;
pub mod marketplace;
pub mod monitoring;
pub mod package;
pub mod probe;
pub mod registry;
pub mod sidecar;
pub mod trust;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Runtime(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
