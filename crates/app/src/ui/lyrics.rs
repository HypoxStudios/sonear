use rust_i18n::t;
use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollEvent, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, point, px,
    relative,
};
use sonear_core::lyrics::{LyricsError, active_line, next_change_ms};
use sonear_core::playback::now_ms;
use sonear_core::{Core, Lyrics, LyricsQuery};
use sonear_soloist::protocol::{Entity as SoloistEntity, Position};
use tracing::warn;

use super::widgets::ghost_button;
use crate::store::AppStore;
use crate::theme;

const LRCLIB_URL: &str = "https://lrclib.net";
const FOLLOW_PAUSE: Duration = Duration::from_secs(5);
/// Added to a line's start so the timer never fires just before it.
const TIMER_SLACK_MS: u64 = 5;
const SKELETON_WIDTHS: [f32; 7] = [0.9, 0.7, 0.8, 0.55, 0.85, 0.65, 0.75];
const NOTE: &str = "\u{266a}";

enum Phase {
    Idle,
    Loading,
    Ready(Lyrics),
    Failed,
}

pub struct LyricsPanel {
    store: Entity<AppStore>,
    scroll: ScrollHandle,
    phase: Phase,
    /// The track the phase belongs to.
    uri: Option<String>,
    active: Option<usize>,
    /// What the timer was last set up for, so unrelated store updates leave it alone.
    timing: Option<(bool, Option<Position>)>,
    timer: Option<Task<()>>,
    follow_paused_until: Option<Instant>,
    scroll_pending: bool,
    _subscription: Subscription,
}

impl LyricsPanel {
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        Self {
            _subscription: cx.observe(&store, |panel, _, cx| panel.sync(cx)),
            store,
            scroll: ScrollHandle::new(),
            phase: Phase::Idle,
            uri: None,
            active: None,
            timing: None,
            timer: None,
            follow_paused_until: None,
            scroll_pending: false,
        }
    }

    /// Reacts to a store update, repainting only when the track or the active line changed.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (showing, same_track, timing) = {
            let state = self.store.read(cx);
            let uri = state.playback.item.as_ref().map(|item| item.uri.as_str());

            (
                state.lyrics_visible(),
                uri == self.uri.as_deref(),
                (
                    state.playback.is_playing(),
                    state.playback.position_anchor(),
                ),
            )
        };

        if !showing {
            self.timer = None;
            self.timing = None;

            return;
        }

        if !same_track {
            cx.notify();

            return;
        }

        if self.timing == Some(timing) {
            return;
        }

        self.timing = Some(timing);
        self.refresh(cx);
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

            if self
                .follow_paused_until
                .is_none_or(|until| Instant::now() >= until)
            {
                self.follow_paused_until = None;
                self.scroll_pending = true;
            }

            cx.notify();
        }

        let Some(next) = next.filter(|_| playing) else {
            return;
        };
        let wait = Duration::from_millis(next.saturating_sub(position) + TIMER_SLACK_MS);

        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;

            let _ = this.update(cx, |panel, cx| panel.refresh(cx));
        }));
    }

    /// Starts loading when the playing track is not the one the panel holds.
    fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        let current = self
            .store
            .read(cx)
            .playback
            .item
            .as_ref()
            .map(|item| item.uri.clone());

        if current == self.uri {
            return;
        }

        self.uri = current.clone();
        self.phase = Phase::Idle;
        self.active = None;
        self.timing = None;
        self.timer = None;
        self.follow_paused_until = None;
        self.scroll_pending = true;
        self.scroll = ScrollHandle::new();

        if current.is_some_and(|uri| is_track(&uri)) {
            self.load(cx);
        }
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

            let _ = this.update(cx, |panel, cx| panel.finish(&uri, found, cx));
        })
        .detach();
    }

    fn finish(&mut self, uri: &str, found: Option<Lyrics>, cx: &mut Context<Self>) {
        if self.uri.as_deref() != Some(uri) {
            return;
        }

        self.phase = found.map_or(Phase::Failed, Phase::Ready);
        self.active = None;
        self.scroll_pending = true;
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

    /// Moves the list so the active line sits near the top third. Returns false
    /// while the line has not been laid out yet.
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

    fn synced_list(&self, lines: &[sonear_core::LyricLine], cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let can_seek = state.playback.can("seek");
        let store = self.store.clone();
        let active = self.active;

        div()
            .id("lyrics-lines")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(theme::LYRICS_GAP))
            .px_2()
            .pt_2()
            .pb(px(theme::LYRICS_END_PADDING))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll(cx.listener(|panel, _: &ScrollEvent, _, _| {
                panel.follow_paused_until = Some(Instant::now() + FOLLOW_PAUSE);
            }))
            .children(lines.iter().enumerate().map(|(index, line)| {
                let color = match active {
                    Some(active) if index == active => theme::text_primary(),
                    Some(active) if index < active => theme::text_secondary(),
                    _ => theme::text_tertiary(),
                };
                let text = if line.text.is_empty() {
                    NOTE
                } else {
                    line.text.as_str()
                };
                let at_ms = line.at_ms;
                let store = store.clone();

                div()
                    .id(ElementId::from(("lyric-line", index)))
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded(theme::CONTROL_RADIUS)
                    .text_size(px(theme::LYRICS_SIZE))
                    .font_weight(FontWeight::BOLD)
                    .font_family(theme::FONT_DISPLAY)
                    .text_color(color)
                    .child(text.to_owned())
                    .when(can_seek, |this| {
                        this.cursor_pointer()
                            .tab_index(0)
                            .hover(|style| style.bg(theme::hover()))
                            .focus_visible(|style| style.bg(theme::hover()))
                            .on_click(move |_, _, cx| {
                                store.update(cx, |store, cx| store.seek(at_ms, cx));
                            })
                    })
            }))
            .into_any_element()
    }

    fn plain_list(text: &str) -> AnyElement {
        div()
            .id("lyrics-plain")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .px_4()
            .pt_2()
            .pb_4()
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::text_tertiary())
                    .child(t!("lyrics.not_synced")),
            )
            .child(
                div()
                    .text_size(px(theme::LYRICS_PLAIN_SIZE))
                    .text_color(theme::text_primary())
                    .child(text.to_owned()),
            )
            .into_any_element()
    }
}

fn log_failure(error: &LyricsError) {
    warn!(%error, "could not load lyrics");
}

fn is_track(uri: &str) -> bool {
    uri.starts_with("spotify:track:")
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

fn message(text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .px_4()
        .pt_2()
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child(text.into())
        .into_any_element()
}

fn skeleton() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(px(theme::LYRICS_GAP))
        .px_4()
        .pt_3()
        .children(SKELETON_WIDTHS.iter().map(|width| {
            div()
                .h(px(theme::LYRICS_SKELETON_HEIGHT))
                .w(relative(*width))
                .rounded(theme::ART_RADIUS_SMALL)
                .bg(theme::skeleton())
        }))
        .into_any_element()
}

impl Render for LyricsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_loaded(cx);

        if self.scroll_pending && !self.scroll_to_active() {
            cx.notify();
        } else {
            self.scroll_pending = false;
        }

        let playing = self
            .store
            .read(cx)
            .playback
            .item
            .as_ref()
            .map(|item| is_track(&item.uri));
        let body = match (playing, &self.phase) {
            (None, _) => message(t!("lyrics.play_something")),
            (Some(false), _) => message(t!("lyrics.songs_only")),
            (Some(true), Phase::Idle | Phase::Loading) => skeleton(),
            (Some(true), Phase::Ready(Lyrics::Synced(lines))) => self.synced_list(lines, cx),
            (Some(true), Phase::Ready(Lyrics::Plain(text))) => Self::plain_list(text),
            (Some(true), Phase::Ready(Lyrics::Instrumental)) => message(t!("lyrics.instrumental")),
            (Some(true), Phase::Ready(Lyrics::NotFound)) => message(t!("lyrics.not_found")),
            (Some(true), Phase::Failed) => div()
                .flex()
                .flex_col()
                .items_start()
                .gap_3()
                .flex_1()
                .min_h_0()
                .px_4()
                .pt_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::text_secondary())
                        .child(t!("lyrics.load_failed")),
                )
                .child(
                    ghost_button("lyrics-retry", t!("common.try_again"), true).on_click(
                        cx.listener(|panel, _, _, cx| {
                            panel.load(cx);
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .rounded(theme::PANEL_RADIUS)
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::panel())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_none()
                    .pl_4()
                    .pr_2()
                    .pt_4()
                    .pb_2()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child(t!("lyrics.title")),
                    ),
            )
            .child(body)
            .child(
                div()
                    .id("lyrics-credit")
                    .flex()
                    .items_center()
                    .flex_none()
                    .mx_4()
                    .mb_3()
                    .pt_3()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .cursor_pointer()
                    .tab_index(0)
                    .text_size(px(12.))
                    .text_color(theme::text_tertiary())
                    .hover(|style| style.text_color(theme::text_secondary()))
                    .focus_visible(|style| style.text_color(theme::accent()))
                    .child(t!("lyrics.credit"))
                    .on_click(|_, _, cx| cx.open_url(LRCLIB_URL)),
            )
    }
}
