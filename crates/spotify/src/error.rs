use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SpotifyError {
    #[error("Spotify needs you to log in again")]
    NeedsLogin,
    #[error("Spotify does not allow this request")]
    Forbidden,
    #[error("Spotify could not find that")]
    NotFound,
    #[error("Spotify asked Sonear to slow down for {} s", .retry_after.as_secs())]
    RateLimited { retry_after: Duration },
    #[error("the Spotify request quota for this developer account is used up")]
    QuotaExceeded,
    #[error("could not reach Spotify: {0}")]
    Network(String),
    #[error("Spotify answered {status}: {message}")]
    Api { status: u16, message: String },
    #[error("could not read Spotify's answer: {0}")]
    Decode(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AuthError {
    #[error("port 27196 is already in use, close the program using it and try again")]
    PortBusy,
    #[error("the login was cancelled")]
    Cancelled,
    #[error("the login timed out")]
    TimedOut,
    #[error("Spotify refused the login: {0}")]
    Denied(String),
    #[error("could not listen for the login: {0}")]
    Listen(String),
    #[error(transparent)]
    Spotify(#[from] SpotifyError),
}
