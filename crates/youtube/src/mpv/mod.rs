//! Controls an mpv process over its JSON IPC.

mod client;
#[cfg(windows)]
mod process;
pub mod protocol;

pub use client::{AudioDevice, MpvClient};
#[cfg(windows)]
pub use process::{Mpv, MpvOptions};
pub use protocol::{EndReason, MpvEvent};
