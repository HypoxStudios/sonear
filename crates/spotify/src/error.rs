use std::time::Duration;

use rust_i18n::t;
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
    #[error("not a Spotify ID: {0}")]
    InvalidId(String),
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

impl SpotifyError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::NeedsLogin => t!("error.needs_login"),
            Self::Forbidden => t!("error.forbidden"),
            Self::NotFound => t!("error.not_found"),
            Self::RateLimited { retry_after } => {
                t!("error.rate_limited", seconds = retry_after.as_secs())
            }
            Self::QuotaExceeded => t!("error.quota"),
            Self::Network(detail) => t!("error.network", detail = detail),
            Self::Api { status, message } => {
                t!("error.api", status = status, message = message)
            }
            Self::Decode(detail) => t!("error.decode", detail = detail),
            Self::InvalidId(id) => t!("error.invalid_id", id = id),
        }
        .into_owned()
    }
}

impl AuthError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::PortBusy => t!("auth.port_busy"),
            Self::Cancelled => t!("auth.cancelled"),
            Self::TimedOut => t!("auth.timed_out"),
            Self::Denied(reason) => t!("auth.denied", reason = reason),
            Self::Listen(detail) => t!("auth.listen", detail = detail),
            Self::Spotify(error) => return error.user_message(),
        }
        .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<SpotifyError> {
        vec![
            SpotifyError::NeedsLogin,
            SpotifyError::Forbidden,
            SpotifyError::NotFound,
            SpotifyError::RateLimited {
                retry_after: Duration::from_secs(7),
            },
            SpotifyError::QuotaExceeded,
            SpotifyError::Network("offline".into()),
            SpotifyError::Api {
                status: 500,
                message: "boom".into(),
            },
            SpotifyError::Decode("bad".into()),
            SpotifyError::InvalidId("x".into()),
        ]
    }

    #[test]
    fn english_user_messages_match_display() {
        for error in samples() {
            assert_eq!(error.user_message(), error.to_string());
        }

        for error in [
            AuthError::PortBusy,
            AuthError::Cancelled,
            AuthError::TimedOut,
            AuthError::Denied("access_denied".into()),
            AuthError::Listen("nope".into()),
            AuthError::Spotify(SpotifyError::NeedsLogin),
        ] {
            assert_eq!(error.user_message(), error.to_string());
        }
    }

    #[test]
    fn spanish_messages_keep_the_details() {
        let network = t!("error.network", locale = "es", detail = "offline");
        let limited = t!("error.rate_limited", locale = "es", seconds = 7);
        let api = t!("error.api", locale = "es", status = 500, message = "boom");

        assert_eq!(network, "No se pudo conectar con Spotify: offline");
        assert!(limited.contains("7 s"));
        assert!(api.contains("500") && api.contains("boom"));
    }
}
