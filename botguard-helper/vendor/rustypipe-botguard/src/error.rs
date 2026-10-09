use std::borrow::Cow;

/// Error type for the rustypipe-botguard library
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Error executing JavaScript
    #[error("javscript: {0}")]
    Js(Cow<'static, str>),
    /// File I/O error
    #[error("file I/O: {0}")]
    Io(#[from] std::io::Error),
    /// HTTP error
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    /// Could not parse snapshot file
    #[error("invalid snapshot: {0}")]
    InvalidSnapshot(Cow<'static, str>),
    /// Could not parse challenge data
    #[error("invalid challenge data: {0}")]
    InvalidChallenge(Cow<'static, str>),
    /// Could not parse challenge response data
    #[error("invalid challenge response: {0}")]
    InvalidResponse(Cow<'static, str>),
    /// Generated an invalid PO token
    #[error("invalid potoken: {0}")]
    InvalidPoToken(Cow<'static, str>),
}

impl From<deno_core::error::CoreError> for Error {
    fn from(value: deno_core::error::CoreError) -> Self {
        Self::Js(value.to_string().into())
    }
}
