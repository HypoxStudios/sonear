use rust_i18n::t;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, px, relative,
};
use sonear_core::PlaybackModel;
use sonear_core::playback::now_ms;
use sonear_soloist::Status;
use sonear_soloist::protocol::{PlayStatus, RepeatMode};

use super::widgets::{
    cover_for, heart_button, icon, icon_button, round_play_button, set_if_changed, spotify_icon,
    text_tooltip,
};
use crate::format::clock;
use crate::store::AppStore;
use crate::theme;

const REPAINT_EVERY: Duration = Duration::from_millis(250);
const PROGRESS_STEPS: f32 = 1000.;
const SIDE_WIDTH: f32 = 320.;
/// Extra space before the volume icon, which has no button box, so it sits as
/// far from the mini player icon as the toggle icons sit from each other.
const VOLUME_GAP: f32 = 8.;
const CENTER_SHARE: f32 = 0.4;
const CENTER_MAX_WIDTH: f32 = 720.;

pub struct PlayerBar {
    store: Entity<AppStore>,
    progress: Entity<SliderState>,
    volume: Entity<SliderState>,
    dragging_progress: bool,
    dragging_volume: bool,
    ticker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl PlayerBar {
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let progress = cx.new(|_| SliderState::new().min(0.).max(PROGRESS_STEPS).step(1.));
        let volume = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.));
        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| {
                this.sync_ticker(cx);
                cx.notify();
            }),
            cx.subscribe(&progress, |this, _, event: &SliderEvent, cx| match event {
                SliderEvent::Change(_) => this.dragging_progress = true,
                SliderEvent::Release(value) => {
                    this.dragging_progress = false;
                    this.seek_to(value.end(), cx);
                }
            }),
            cx.subscribe(&volume, |this, _, event: &SliderEvent, cx| match event {
                SliderEvent::Change(value) => {
                    this.dragging_volume = true;
                    this.store.update(cx, |store, cx| {
                        store.set_volume(value.end().round() as u8, cx);
                    });
                }
                SliderEvent::Release(_) => this.dragging_volume = false,
            }),
        ];

        let mut bar = Self {
            store,
            progress,
            volume,
            dragging_progress: false,
            dragging_volume: false,
            ticker: None,
            _subscriptions: subscriptions,
        };

        bar.sync_ticker(cx);

        bar
    }

    /// Only this view repaints each tick, and only while playing in a window
    /// that is on screen, so an idle or hidden app runs no repaint timer and
    /// pages stay untouched.
    fn sync_ticker(&mut self, cx: &mut Context<Self>) {
        if !self.store.read(cx).ticking() {
            self.ticker = None;

            return;
        }

        if self.ticker.is_some() {
            return;
        }

        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REPAINT_EVERY).await;

                let keep_going = this
                    .update(cx, |bar, cx| {
                        if !bar.store.read(cx).ticking() {
                            bar.ticker = None;

                            return false;
                        }

                        cx.notify();

                        true
                    })
                    .unwrap_or(false);

                if !keep_going {
                    break;
                }
            }
        }));
    }

    fn seek_to(&mut self, step: f32, cx: &mut Context<Self>) {
        let Some(duration) = self.store.read(cx).playback.duration_ms() else {
            return;
        };
        let target = (f64::from(step) / f64::from(PROGRESS_STEPS) * duration as f64) as u64;

        self.store
            .update(cx, |store, cx| store.seek(target.min(duration), cx));
    }

    fn sync_sliders(
        &mut self,
        model: &PlaybackModel,
        shown_position: u64,
        shown_volume: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging_progress {
            let target = match model.duration_ms() {
                Some(duration) if duration > 0 => {
                    shown_position as f32 / duration as f32 * PROGRESS_STEPS
                }
                _ => 0.,
            };

            set_if_changed(&self.progress, target, 0.5, window, cx);
        }

        if !self.dragging_volume {
            set_if_changed(&self.volume, f32::from(shown_volume), 0.5, window, cx);
        }
    }
}

/// `spotify:track:abc` as `https://open.spotify.com/track/abc`.
pub(super) fn spotify_url(uri: &str) -> Option<String> {
    let (kind, id) = sonear_core::model::split_uri(uri)?;

    matches!(kind, "track" | "episode").then(|| format!("https://open.spotify.com/{kind}/{id}"))
}

fn soloist_label(status: &Status, demo: bool) -> (String, bool) {
    if demo {
        return (t!("player.status.demo").into_owned(), true);
    }

    let (label, ready) = match status {
        Status::Ready { .. } => (t!("player.status.ready"), true),
        Status::Starting => (t!("player.status.starting"), false),
        Status::NeedsPairing { .. } => (t!("player.status.pair"), false),
        Status::NotInstalled => (t!("player.status.not_installed"), false),
        Status::NeedsKey => (t!("player.status.key_needed"), false),
        Status::Expired => (t!("player.status.update_needed"), false),
        Status::Failed { .. } => (t!("player.status.offline"), false),
        Status::Stopped => (t!("player.status.stopped"), false),
    };

    (label.into_owned(), ready)
}

impl Render for PlayerBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let model = state.playback.clone();
        let demo = state.is_demo();
        let (status_label, status_ready) = soloist_label(&state.soloist, demo);
        let queue_open = state.queue_showing();
        let lyrics_open = state.lyrics_showing();
        let lyrics_enabled = state.lyrics_enabled;
        let mini_open = state.mini_player_open();
        let player = state.core.player.clone();
        let queue = state.core.queue.clone();
        let has_queued = model.is_active
            && model.item.is_some()
            && (state.queue.locked.is_some() || !state.queue.entries.is_empty());
        let pending_play = state.pending_play;
        let likes = state.core.likes.clone();
        let now = now_ms();
        let shown_position = state.shown_position_ms(now);
        let shown_volume = state.shown_volume(now);
        let store = self.store.clone();
        let lyrics_store = self.store.clone();
        let mini_store = self.store.clone();

        self.sync_sliders(&model, shown_position, shown_volume, window, cx);

        let item = model.item.clone();
        let playing = model.is_playing();
        let buffering = model.status == PlayStatus::Buffering;
        let play_enabled = if playing {
            model.can("pause")
        } else {
            item.is_some()
        };
        let duration = model.duration_ms();
        let position = if self.dragging_progress {
            let step = self.progress.read(cx).value().end();

            (f64::from(step) / f64::from(PROGRESS_STEPS) * duration.unwrap_or(0) as f64) as u64
        } else {
            shown_position
        };
        let track_uri = item
            .as_ref()
            .map(|item| item.uri.clone())
            .filter(|uri| uri.starts_with("spotify:track:"));
        let liked = track_uri
            .as_ref()
            .and_then(|uri| likes.is_liked(uri))
            .unwrap_or(false);
        let can_like = track_uri.is_some();
        let open_url = item.as_ref().and_then(|item| spotify_url(&item.uri));
        let repeat = model.repeat();
        let shuffle_on = model.options.shuffle;
        let accent_if = |on: bool| {
            if on {
                theme::accent()
            } else {
                theme::text_secondary()
            }
        };

        let title = item.as_ref().map_or_else(
            || t!("player.nothing_playing").into_owned(),
            |item| item.name().to_owned(),
        );
        let subtitle = match &item {
            Some(item) => item.artist_line(),
            None if demo => String::new(),
            None => t!("player.choose_sonear").into_owned(),
        };

        let left = div()
            .flex()
            .items_center()
            .gap_3()
            .w(px(SIDE_WIDTH))
            .flex_none()
            .min_w_0()
            .child(cover_for(item.as_ref(), 52., theme::ART_RADIUS_SMALL))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .id("player-title")
                            .truncate()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if item.is_some() {
                                theme::text_primary()
                            } else {
                                theme::text_tertiary()
                            })
                            .tooltip(text_tooltip(title.clone()))
                            .when_some(open_url.clone(), |this, url| {
                                this.cursor_pointer()
                                    .hover(|style| style.text_color(theme::accent()))
                                    .on_click(move |_, _, cx| cx.open_url(&url))
                            })
                            .child(title),
                    )
                    .child(
                        div()
                            .id("player-artists")
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .tooltip(text_tooltip(subtitle.clone()))
                            .child(subtitle),
                    ),
            )
            .when(item.is_some(), |this| {
                this.child(
                    div()
                        .id("player-spotify")
                        .flex_none()
                        .tooltip(text_tooltip(t!("common.open_spotify")))
                        .when_some(open_url, |this, url| {
                            this.cursor_pointer()
                                .on_click(move |_, _, cx| cx.open_url(&url))
                        })
                        .child(spotify_icon()),
                )
            })
            .child({
                let store = self.store.clone();

                heart_button("like", liked, 16., 30., theme::text_secondary(), can_like).on_click(
                    move |_, _, cx| {
                        if let Some(uri) = &track_uri {
                            store.update(cx, |state, cx| state.toggle_like(uri, cx));
                        }
                    },
                )
            });

        let play_button = {
            let player = player.clone();
            let mut button = round_play_button(
                "play-pause",
                playing,
                40.,
                theme::text_primary(),
                play_enabled,
            );

            if buffering || (pending_play && !playing) {
                button = button.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme::text_primary())
                        .child(Spinner::new().small().color(theme::on_accent())),
                );
            }

            button.on_click(move |_, _, _| player.toggle_play_pause())
        };

        let controls = div()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .child({
                let player = player.clone();

                icon_button(
                    "shuffle",
                    IconName::Shuffle,
                    17.,
                    34.,
                    accent_if(shuffle_on),
                    model.can("shuffle"),
                )
                .on_click(move |_, _, _| player.toggle_shuffle())
            })
            .child({
                let player = player.clone();

                icon_button(
                    "previous",
                    IconName::SkipBack,
                    18.,
                    34.,
                    theme::text_primary(),
                    model.can("skip_prev"),
                )
                .on_click(move |_, _, _| player.previous())
            })
            .child(play_button)
            .child({
                let queue = queue.clone();

                icon_button(
                    "next",
                    IconName::SkipForward,
                    18.,
                    34.,
                    theme::text_primary(),
                    model.can("skip_next") || has_queued,
                )
                .on_click(move |_, _, _| queue.next())
            })
            .child({
                let player = player.clone();

                icon_button(
                    "repeat",
                    if repeat == RepeatMode::Track {
                        IconName::Repeat1
                    } else {
                        IconName::Repeat
                    },
                    17.,
                    34.,
                    accent_if(matches!(repeat, RepeatMode::Context | RepeatMode::Track)),
                    model.can("set_repeat"),
                )
                .on_click(move |_, _, _| player.cycle_repeat())
            });

        let seek_enabled = duration.is_some() && model.can("seek");
        let progress = div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(time_label(clock(position)).text_right())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Slider::new(&self.progress).disabled(!seek_enabled)),
            )
            .child(time_label(clock(duration.unwrap_or(0))));

        let center = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .w(relative(CENTER_SHARE))
            .max_w(px(CENTER_MAX_WIDTH))
            .min_w_0()
            .child(controls)
            .child(progress);

        let volume_icon = match shown_volume {
            0 => IconName::VolumeX,
            1..=49 => IconName::Volume1,
            _ => IconName::Volume2,
        };
        let right = div()
            .flex()
            .items_center()
            .justify_end()
            .gap_1()
            .w(px(SIDE_WIDTH))
            .flex_none()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pr_2()
                    .text_size(px(12.))
                    .text_color(theme::text_secondary())
                    .child(div().size(px(6.)).rounded_full().bg(if status_ready {
                        theme::accent()
                    } else {
                        theme::text_secondary()
                    }))
                    .child(status_label),
            )
            .when(lyrics_enabled, |this| {
                this.child(
                    icon_button(
                        "lyrics",
                        IconName::MicVocal,
                        16.,
                        32.,
                        accent_if(lyrics_open),
                        true,
                    )
                    .on_click(move |_, _, cx| {
                        lyrics_store.update(cx, |state, cx| state.show_lyrics_panel(cx));
                    }),
                )
            })
            .child(
                icon_button(
                    "queue",
                    IconName::ListMusic,
                    16.,
                    32.,
                    accent_if(queue_open),
                    true,
                )
                .on_click(move |_, _, cx| {
                    store.update(cx, |state, cx| state.show_queue_panel(cx));
                }),
            )
            .child(
                icon_button(
                    "mini-player",
                    IconName::PictureInPicture2,
                    16.,
                    32.,
                    accent_if(mini_open),
                    true,
                )
                .tooltip(text_tooltip(t!("player.mini_player")))
                .on_click(move |_, _, cx| {
                    mini_store.update(cx, |state, cx| state.toggle_mini_player(cx));
                }),
            )
            .child(div().flex_none().ml(px(VOLUME_GAP)).child(icon(
                volume_icon,
                16.,
                theme::text_secondary(),
            )))
            .child(
                div()
                    .w(px(88.))
                    .flex_none()
                    .child(Slider::new(&self.volume)),
            );

        div()
            .flex()
            .items_center()
            .gap_4()
            .size_full()
            .px_4()
            .rounded(theme::PANEL_RADIUS)
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::player())
            .child(div().flex_1().flex().child(left))
            .child(center)
            .child(div().flex_1().flex().justify_end().child(right))
    }
}

fn time_label(text: String) -> Div {
    div()
        .w(px(40.))
        .flex_none()
        .text_size(px(11.))
        .text_color(theme::text_tertiary())
        .child(text)
}
