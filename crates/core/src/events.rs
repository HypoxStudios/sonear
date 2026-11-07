use sonear_soloist::Status;
use sonear_spotify::RateLimit;

use crate::account::AccountState;
use crate::playback::PlaybackModel;

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
    RateLimit(RateLimit),
    /// A short message for the user, shown as a toast.
    Notice(String),
}
