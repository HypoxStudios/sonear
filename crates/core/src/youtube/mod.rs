//! YouTube Music as a second source: the catalogue, a local library, the
//! queue rules and a player built on mpv. It talks to the rest of the core
//! through the same traits and Soloist-shaped events the other sources use.

pub mod account;
pub mod backend;
pub mod catalog;
pub mod convert;
pub mod deck;
pub mod engine;
pub mod library;
pub mod local;
pub mod tools;

pub use account::{SessionOutcome, SessionProvider, YoutubeAccount, YtAccountState};
pub use backend::YoutubeBackend;
pub use catalog::{LoadedContext, YoutubeCatalog};
pub use deck::{ContextInfo, Deck};
pub use engine::{AudioEngine, EngineLauncher, MpvLauncher, ResolverStreams, StreamSource};
pub use library::YoutubeLibrary;
pub use local::LocalLibrary;
pub use tools::{ToolsStatus, YoutubeTools};
