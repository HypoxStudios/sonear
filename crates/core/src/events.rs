use sonear_soloist::Status;
use sonear_spotify::RateLimit;

use crate::account::AccountState;
use crate::playback::PlaybackModel;
use crate::youtube::{ToolsStatus, YtAccountState};

/// Everything the UI reacts to, published on one broadcast channel.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Playback(Box<PlaybackModel>),
    Soloist(Status),
    /// Days until the installed Soloist build expires; `None` if unknown.
    SoloistExpiry {
        days_left: Option<i64>,
    },
    Account(AccountState),
    /// Saved-music hearts changed.
    Likes,
    HistoryChanged,
    /// The user's play queue changed.
    Queue,
    RateLimit(RateLimit),
    /// The collected tracks of a playlist Spotify won't list changed.
    SeenTracks {
        playlist_id: String,
    },
    /// Cached account data was deleted, so UI caches must drop theirs.
    AccountDataWiped,
    /// The YT Music sign-in changed.
    YoutubeAccount(YtAccountState),
    /// The YouTube Music playback tools are being installed or changed state.
    Tools(ToolsStatus),
    /// A short message for the user, shown as a toast.
    Notice(String),
}
