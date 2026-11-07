//! Spotify Web API: PKCE login, token refresh, rate limits, models and endpoints.
//!
//! Has no UI dependencies and no dependency on the rest of Sonear.

pub mod api;
pub mod auth;
pub mod client;
pub mod error;
pub mod models;
pub mod secret;

rust_i18n::i18n!("locales", fallback = "en");

pub use auth::{Auth, LoginRequest, TokenStore};
pub use client::{RateLimit, SpotifyClient};
pub use error::SpotifyError;
pub use secret::Secret;

/// A request that never answers would otherwise hold a concurrency permit, or
/// the token lock, forever and stall every later call.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

pub const API_BASE: &str = "https://api.spotify.com/v1/";
pub const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";
pub const REDIRECT_URI: &str = "http://127.0.0.1:27196/callback";
pub const REDIRECT_ADDR: &str = "127.0.0.1:27196";
pub const SCOPES: &str = "user-read-playback-state user-modify-playback-state \
user-read-currently-playing user-read-recently-played user-top-read user-library-read \
user-library-modify playlist-read-private playlist-read-collaborative user-follow-read";

#[cfg(test)]
mod i18n_tests;
