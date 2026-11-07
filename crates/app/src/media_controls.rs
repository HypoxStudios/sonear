//! Windows media controls (SMTC): the media keys, the volume flyout and the
//! lock screen. Windows routes the keys to the active media session, so no
//! global hotkey is registered.

use sonear_core::PlaybackModel;
use sonear_soloist::protocol::Position;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
    SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use windows::core::HSTRING;

/// Windows counts time in ticks of 100 nanoseconds.
const TICKS_PER_MS: i64 = 10_000;

/// What a media key asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    Play,
    Pause,
    Next,
    Previous,
    Stop,
    Seek(u64),
}

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("could not set up the media controls: {0}")]
    Windows(#[from] windows::core::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Closed,
}

/// The parts of the playback state the media controls show.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub status: Status,
    pub uri: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover: Option<String>,
    pub duration_ms: Option<u64>,
    pub anchor: Option<Position>,
    pub playing: bool,
}

impl Snapshot {
    pub fn of(model: &PlaybackModel) -> Self {
        let item = model.item.as_ref();
        let status = match item {
            None => Status::Closed,
            Some(_) if model.is_playing() => Status::Playing,
            Some(_) => Status::Paused,
        };

        Self {
            status,
            uri: item.map(|item| item.uri.clone()),
            title: item.map_or_else(String::new, |item| item.name().to_owned()),
            artist: item.map_or_else(String::new, |item| item.artist_line()),
            album: item
                .and_then(|item| item.album())
                .map_or_else(String::new, |album| album.name().to_owned()),
            cover: item.and_then(|item| item.cover_url("default").map(str::to_owned)),
            duration_ms: model.duration_ms(),
            anchor: model.position_anchor(),
            playing: model.is_playing(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TimelineKey {
    uri: String,
    duration_ms: u64,
    anchor: Option<Position>,
    playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayChange {
    Keep,
    Clear,
    Set,
}

/// What has to be sent to Windows to catch up with a new snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub status: Option<Status>,
    pub display: DisplayChange,
    pub timeline: bool,
}

/// What Windows was last told, so a snapshot only sends what differs.
#[derive(Debug, Default)]
pub struct Applied {
    status: Option<Status>,
    display_uri: Option<String>,
    timeline: Option<TimelineKey>,
}

impl Applied {
    /// Works out what to send for `next` and records it as sent.
    pub fn plan(&mut self, next: &Snapshot) -> Plan {
        let status = (self.status != Some(next.status)).then_some(next.status);

        self.status = Some(next.status);

        let Some(uri) = &next.uri else {
            let display = if self.display_uri.take().is_some() {
                DisplayChange::Clear
            } else {
                DisplayChange::Keep
            };

            self.timeline = None;

            return Plan {
                status,
                display,
                timeline: false,
            };
        };

        let display = if self.display_uri.as_ref() != Some(uri) && !next.title.is_empty() {
            self.display_uri = Some(uri.clone());

            DisplayChange::Set
        } else {
            DisplayChange::Keep
        };
        let timeline = next.duration_ms.is_some_and(|duration_ms| {
            let key = TimelineKey {
                uri: uri.clone(),
                duration_ms,
                anchor: next.anchor,
                playing: next.playing,
            };

            if self.timeline.as_ref() == Some(&key) {
                return false;
            }

            self.timeline = Some(key);

            true
        });

        Plan {
            status,
            display,
            timeline,
        }
    }
}

/// The system transport controls of one window.
pub struct MediaControls {
    controls: SystemMediaTransportControls,
    applied: Applied,
    pressed: i64,
    seek: i64,
}

impl MediaControls {
    /// Hooks the controls up to `window`. Commands from the keys arrive on
    /// `commands`, sent from a thread-pool thread.
    pub fn new(window: HWND, commands: UnboundedSender<MediaCommand>) -> Result<Self, MediaError> {
        let interop = windows::core::factory::<
            SystemMediaTransportControls,
            ISystemMediaTransportControlsInterop,
        >()?;

        // SAFETY: `window` is the live main window and this runs on its thread.
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(window)? };

        controls.SetIsEnabled(true)?;
        controls.SetIsPlayEnabled(true)?;
        controls.SetIsPauseEnabled(true)?;
        controls.SetIsNextEnabled(true)?;
        controls.SetIsPreviousEnabled(true)?;
        controls.SetIsStopEnabled(true)?;

        let presses = commands.clone();
        let pressed =
            controls.ButtonPressed(&TypedEventHandler::new(
                move |_,
                      args: windows::core::Ref<
                    SystemMediaTransportControlsButtonPressedEventArgs,
                >| {
                    let Some(args) = args.as_ref() else {
                        return Ok(());
                    };
                    let command = match args.Button()? {
                        SystemMediaTransportControlsButton::Play => Some(MediaCommand::Play),
                        SystemMediaTransportControlsButton::Pause => Some(MediaCommand::Pause),
                        SystemMediaTransportControlsButton::Next => Some(MediaCommand::Next),
                        SystemMediaTransportControlsButton::Previous => {
                            Some(MediaCommand::Previous)
                        }
                        SystemMediaTransportControlsButton::Stop => Some(MediaCommand::Stop),
                        _ => None,
                    };

                    if let Some(command) = command {
                        let _ = presses.send(command);
                    }

                    Ok(())
                },
            ))?;
        let seek = controls.PlaybackPositionChangeRequested(&TypedEventHandler::new(
            move |_, args: windows::core::Ref<PlaybackPositionChangeRequestedEventArgs>| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                let ticks = args.RequestedPlaybackPosition()?.Duration.max(0);

                let _ = commands.send(MediaCommand::Seek((ticks / TICKS_PER_MS) as u64));

                Ok(())
            },
        ))?;

        Ok(Self {
            controls,
            applied: Applied::default(),
            pressed,
            seek,
        })
    }

    /// Sends what changed since the last call. Does nothing when nothing did.
    pub fn update(&mut self, model: &PlaybackModel) {
        let snapshot = Snapshot::of(model);
        let plan = self.applied.plan(&snapshot);

        if let Err(error) = self.apply(&plan, &snapshot, model) {
            debug!(%error, "could not update the media controls");
        }
    }

    fn apply(
        &self,
        plan: &Plan,
        snapshot: &Snapshot,
        model: &PlaybackModel,
    ) -> windows::core::Result<()> {
        if let Some(status) = plan.status {
            self.controls.SetPlaybackStatus(match status {
                Status::Playing => MediaPlaybackStatus::Playing,
                Status::Paused => MediaPlaybackStatus::Paused,
                Status::Closed => MediaPlaybackStatus::Closed,
            })?;
        }

        match plan.display {
            DisplayChange::Keep => {}
            DisplayChange::Clear => self.controls.DisplayUpdater()?.ClearAll()?,
            DisplayChange::Set => self.show_track(snapshot)?,
        }

        if plan.timeline
            && let Some(duration_ms) = snapshot.duration_ms
        {
            let position_ms = model.position_ms_at(sonear_core::playback::now_ms());
            let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
            let ticks = |ms: u64| TimeSpan {
                Duration: ms as i64 * TICKS_PER_MS,
            };

            timeline.SetStartTime(ticks(0))?;
            timeline.SetMinSeekTime(ticks(0))?;
            timeline.SetEndTime(ticks(duration_ms))?;
            timeline.SetMaxSeekTime(ticks(duration_ms))?;
            timeline.SetPosition(ticks(position_ms))?;
            self.controls.UpdateTimelineProperties(&timeline)?;
        }

        Ok(())
    }

    fn show_track(&self, snapshot: &Snapshot) -> windows::core::Result<()> {
        let updater = self.controls.DisplayUpdater()?;

        updater.ClearAll()?;
        updater.SetType(MediaPlaybackType::Music)?;

        let music = updater.MusicProperties()?;

        music.SetTitle(&HSTRING::from(snapshot.title.as_str()))?;
        music.SetArtist(&HSTRING::from(snapshot.artist.as_str()))?;
        music.SetAlbumTitle(&HSTRING::from(snapshot.album.as_str()))?;

        if let Some(cover) = &snapshot.cover
            && let Ok(uri) = Uri::CreateUri(&HSTRING::from(cover.as_str()))
        {
            updater.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&uri)?)?;
        }

        updater.Update()
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        let _ = self.controls.RemoveButtonPressed(self.pressed);
        let _ = self
            .controls
            .RemovePlaybackPositionChangeRequested(self.seek);
        let _ = self.controls.SetIsEnabled(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(position_ms: u64) -> Option<Position> {
        Some(Position {
            position_ms,
            timestamp_ms: 1_000,
            speed: 1.0,
        })
    }

    fn playing(uri: &str) -> Snapshot {
        Snapshot {
            status: Status::Playing,
            uri: Some(uri.to_owned()),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            cover: None,
            duration_ms: Some(180_000),
            anchor: anchor(0),
            playing: true,
        }
    }

    fn nothing() -> Snapshot {
        Snapshot {
            status: Status::Closed,
            uri: None,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            cover: None,
            duration_ms: None,
            anchor: None,
            playing: false,
        }
    }

    #[test]
    fn a_new_track_sends_everything_once() {
        let mut applied = Applied::default();
        let song = playing("spotify:track:a");

        assert_eq!(
            applied.plan(&song),
            Plan {
                status: Some(Status::Playing),
                display: DisplayChange::Set,
                timeline: true,
            }
        );
        assert_eq!(
            applied.plan(&song),
            Plan {
                status: None,
                display: DisplayChange::Keep,
                timeline: false,
            }
        );
    }

    #[test]
    fn pausing_changes_the_status_and_freezes_the_timeline() {
        let mut applied = Applied::default();
        let mut song = playing("spotify:track:a");

        applied.plan(&song);

        song.status = Status::Paused;
        song.playing = false;

        assert_eq!(
            applied.plan(&song),
            Plan {
                status: Some(Status::Paused),
                display: DisplayChange::Keep,
                timeline: true,
            }
        );
    }

    #[test]
    fn a_new_anchor_resends_only_the_timeline() {
        let mut applied = Applied::default();
        let mut song = playing("spotify:track:a");

        applied.plan(&song);

        song.anchor = anchor(90_000);

        assert_eq!(
            applied.plan(&song),
            Plan {
                status: None,
                display: DisplayChange::Keep,
                timeline: true,
            }
        );
    }

    #[test]
    fn the_old_display_stays_until_the_new_name_is_known() {
        let mut applied = Applied::default();
        let mut song = playing("spotify:track:a");

        applied.plan(&song);

        song.uri = Some("spotify:track:b".into());
        song.title.clear();

        assert_eq!(applied.plan(&song).display, DisplayChange::Keep);

        song.title = "Next".into();

        assert_eq!(applied.plan(&song).display, DisplayChange::Set);
        assert_eq!(applied.plan(&song).display, DisplayChange::Keep);
    }

    #[test]
    fn no_item_closes_and_clears_once() {
        let mut applied = Applied::default();

        applied.plan(&playing("spotify:track:a"));

        assert_eq!(
            applied.plan(&nothing()),
            Plan {
                status: Some(Status::Closed),
                display: DisplayChange::Clear,
                timeline: false,
            }
        );
        assert_eq!(
            applied.plan(&nothing()),
            Plan {
                status: None,
                display: DisplayChange::Keep,
                timeline: false,
            }
        );
    }

    #[test]
    fn startup_with_nothing_playing_only_closes() {
        let mut applied = Applied::default();

        assert_eq!(
            applied.plan(&nothing()),
            Plan {
                status: Some(Status::Closed),
                display: DisplayChange::Keep,
                timeline: false,
            }
        );
    }

    #[test]
    fn a_track_without_a_length_sends_no_timeline() {
        let mut applied = Applied::default();
        let mut song = playing("spotify:track:a");

        song.duration_ms = None;

        assert!(!applied.plan(&song).timeline);
    }
}
