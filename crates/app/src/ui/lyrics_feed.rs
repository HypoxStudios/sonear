use std::time::{Duration, Instant};

use gpui_kit::{Context, Entity, ScrollHandle, Subscription, Task, point, px};
use sonear_core::lyrics::{LyricsError, active_line, next_change_ms};
use sonear_core::playback::now_ms;
use sonear_core::{Core, LyricLine, Lyrics, LyricsQuery};
use sonear_soloist::protocol::{Entity as SoloistEntity, Position};
use tracing::warn;

use crate::store::AppStore;
use crate::theme;

/// How long following the active line stays off after the user scrolls.
const FOLLOW_PAUSE: Duration = Duration::from_secs(5);
/// Added to a line's start so the timer never fires just before it.
const TIMER_SLACK_MS: u64 = 5;

enum Phase {
    Idle,
    Loading,
    Ready(Lyrics),
    Failed,
}

/// What a lyrics view shows.
pub enum Body<'a> {
    NothingPlaying,
    NotATrack,
    Loading,
    Synced(&'a [LyricLine]),
    Plain(&'a str),
    Instrumental,
    NotFound,
    Failed,
}

/// Loads the lyrics of the playing track and works out the active line, once for
/// every view that shows them. It only works while a view is on screen.
pub struct LyricsFeed {
    store: Entity<AppStore>,
    phase: Phase,
    /// The track the phase belongs to.
    uri: Option<String>,
    active: Option<usize>,
    /// Bumped when the track changes or its lyrics arrive, so views restart their scrolling.
    epoch: u64,
    /// What the timer was last set up for, so unrelated store updates leave it alone.
    timing: Option<(bool, Option<Position>)>,
    timer: Option<Task<()>>,
    _subscription: Subscription,
}

impl LyricsFeed {
    /// Does not read the store, so it can be created while the store is being built.
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        Self {
            _subscription: cx.observe(&store, |feed, _, cx| feed.sync(cx)),
            store,
            phase: Phase::Idle,
            uri: None,
            active: None,
            epoch: 0,
            timing: None,
            timer: None,
        }
    }

    pub fn active(&self) -> Option<usize> {
        self.active
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn body(&self) -> Body<'_> {
        let Some(uri) = &self.uri else {
            return Body::NothingPlaying;
        };

        if !is_track(uri) {
            return Body::NotATrack;
        }

        match &self.phase {
            Phase::Idle | Phase::Loading => Body::Loading,
            Phase::Ready(Lyrics::Synced(lines)) => Body::Synced(lines),
            Phase::Ready(Lyrics::Plain(text)) => Body::Plain(text),
            Phase::Ready(Lyrics::Instrumental) => Body::Instrumental,
            Phase::Ready(Lyrics::NotFound) => Body::NotFound,
            Phase::Failed => Body::Failed,
        }
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
        cx.notify();
    }

    /// Reacts to a store update, repainting only when the track or the active line changed.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (wanted, uri, timing) = {
            let state = self.store.read(cx);

            (
                state.lyrics_wanted(),
                state.playback.item.as_ref().map(|item| item.uri.clone()),
                (
                    state.playback.is_playing(),
                    state.playback.position_anchor(),
                ),
            )
        };

        if !wanted {
            self.timer = None;
            self.timing = None;

            return;
        }

        if uri != self.uri {
            self.start_track(uri, cx);
            cx.notify();

            return;
        }

        if self.timing == Some(timing) {
            return;
        }

        self.timing = Some(timing);
        self.refresh(cx);
    }

    fn start_track(&mut self, uri: Option<String>, cx: &mut Context<Self>) {
        self.uri = uri.clone();
        self.phase = Phase::Idle;
        self.active = None;
        self.timing = None;
        self.timer = None;
        self.epoch += 1;

        if uri.is_some_and(|uri| is_track(&uri)) {
            self.load(cx);
        }
    }

    /// Recomputes the active line and sets the one timer to the next line change.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.timer = None;

        let Phase::Ready(Lyrics::Synced(lines)) = &self.phase else {
            return;
        };
        let state = self.store.read(cx);
        let position = state.playback.position_ms_at(now_ms());
        let playing = state.playback.is_playing();
        let active = active_line(lines, position);
        let next = next_change_ms(lines, position);

        if active != self.active {
            self.active = active;
            cx.notify();
        }

        let Some(next) = next.filter(|_| playing) else {
            return;
        };
        let wait = Duration::from_millis(next.saturating_sub(position) + TIMER_SLACK_MS);

        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;

            let _ = this.update(cx, |feed, cx| feed.refresh(cx));
        }));
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let state = self.store.read(cx);
        let Some(item) = state.playback.item.clone() else {
            return;
        };
        let env = state.env();
        let uri = item.uri.clone();

        self.phase = Phase::Loading;

        let work = env.runtime.spawn(async move {
            let Some(query) = query_for(&env.core, &item).await else {
                return Ok(Lyrics::NotFound);
            };

            env.core.lyrics.get(&query).await
        });

        cx.spawn(async move |this, cx| {
            let found = match work.await {
                Ok(Ok(lyrics)) => Some(lyrics),
                Ok(Err(error)) => {
                    log_failure(&error);

                    None
                }
                Err(_) => None,
            };

            let _ = this.update(cx, |feed, cx| feed.finish(&uri, found, cx));
        })
        .detach();
    }

    fn finish(&mut self, uri: &str, found: Option<Lyrics>, cx: &mut Context<Self>) {
        if self.uri.as_deref() != Some(uri) {
            return;
        }

        self.phase = found.map_or(Phase::Failed, Phase::Ready);
        self.active = None;
        self.epoch += 1;
        self.timing = {
            let state = self.store.read(cx);

            Some((
                state.playback.is_playing(),
                state.playback.position_anchor(),
            ))
        };
        self.refresh(cx);
        cx.notify();
    }
}

/// A view's scrolling over the lines: it follows the active line, and stays
/// where the user put it for a while after they scroll.
pub struct Follow {
    scroll: ScrollHandle,
    paused_until: Option<Instant>,
    pending: bool,
    epoch: u64,
    active: Option<usize>,
}

impl Follow {
    pub fn new() -> Self {
        Self {
            scroll: ScrollHandle::new(),
            paused_until: None,
            pending: false,
            epoch: 0,
            active: None,
        }
    }

    pub fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    pub fn active(&self) -> Option<usize> {
        self.active
    }

    pub fn pause(&mut self) {
        self.paused_until = Some(Instant::now() + FOLLOW_PAUSE);
    }

    /// Takes the feed's latest track and active line; call it at the start of a render.
    pub fn sync(&mut self, feed: &LyricsFeed) {
        if feed.epoch() != self.epoch {
            self.epoch = feed.epoch();
            self.active = feed.active();
            self.scroll = ScrollHandle::new();
            self.paused_until = None;
            self.pending = true;

            return;
        }

        if feed.active() == self.active {
            return;
        }

        self.active = feed.active();

        if self
            .paused_until
            .is_none_or(|until| Instant::now() >= until)
        {
            self.paused_until = None;
            self.pending = true;
        }
    }

    /// Moves the list to the active line. Returns true while the line has not been
    /// laid out yet, so the caller repaints.
    pub fn settle(&mut self) -> bool {
        if !self.pending {
            return false;
        }

        if self.scroll_to_active() {
            self.pending = false;

            return false;
        }

        true
    }

    /// Puts the active line at the focus point near the top. Returns false while
    /// the line has not been laid out yet.
    fn scroll_to_active(&self) -> bool {
        let Some(active) = self.active else {
            self.scroll.set_offset(point(px(0.), px(0.)));

            return true;
        };
        let Some(line) = self.scroll.bounds_for_item(active) else {
            return false;
        };
        let viewport = self.scroll.bounds();
        let target = viewport.top() + viewport.size.height * theme::LYRICS_FOCUS;
        let mut offset = self.scroll.offset();

        offset.y = (target - line.top()).min(px(0.));
        self.scroll.set_offset(offset);

        true
    }
}

pub fn is_track(uri: &str) -> bool {
    uri.starts_with("spotify:track:")
}

fn log_failure(error: &LyricsError) {
    warn!(%error, "could not load lyrics");
}

async fn query_for(core: &Core, item: &SoloistEntity) -> Option<LyricsQuery> {
    if !item.name().is_empty() {
        return Some(LyricsQuery {
            uri: item.uri.clone(),
            track: item.name().to_owned(),
            artist: item.first_artist().unwrap_or_default().to_owned(),
            album: item
                .album()
                .map(|album| album.name().to_owned())
                .unwrap_or_default(),
            duration_ms: item.duration_ms(),
        });
    }

    let track = core.history.resolver().resolve(&item.uri).await?;

    Some(LyricsQuery {
        uri: item.uri.clone(),
        artist: track
            .artists
            .first()
            .map(|artist| artist.name.clone())
            .unwrap_or_default(),
        album: track
            .album
            .as_ref()
            .map(|album| album.name.clone())
            .unwrap_or_default(),
        duration_ms: item
            .duration_ms()
            .or(Some(track.duration_ms).filter(|duration| *duration > 0)),
        track: track.name,
    })
}
