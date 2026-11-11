//! YouTube Music for Sonear: the InnerTube catalogue, the yt-dlp stream
//! resolver, the mpv controller and the installer for both tools.
//!
//! Has no UI dependencies and no dependency on the rest of Sonear.

pub mod auth;
pub mod error;
pub mod innertube;
pub mod models;
pub mod mpv;
pub mod resolver;
pub mod tools;

rust_i18n::i18n!("locales", fallback = "en");

pub use auth::Session;
pub use error::{ToolError, YoutubeError};
pub use innertube::{Client, SearchFilter};
pub use resolver::{Resolver, StreamUrl};

#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(test)]
mod i18n_tests;
