//! Typed access to Spotify Soloist: protocol, client, installer and supervisor.

pub mod client;
pub mod install;
pub mod protocol;
pub mod supervisor;
pub mod version;

rust_i18n::i18n!("locales", fallback = "en");

pub use client::{ClientError, ClientEvent, SoloistClient};
pub use protocol::{Command, Event};
pub use supervisor::{FailReason, Status, Supervisor, SupervisorConfig};

#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(test)]
mod i18n_tests;
