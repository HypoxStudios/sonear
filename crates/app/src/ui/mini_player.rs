use rust_i18n::t;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, ElementId, Entity,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollEvent,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowKind, WindowOptions, div,
    point, px, relative, size,
};
use sonear_core::playback::now_ms;
use sonear_core::{Config, Core};
use sonear_soloist::protocol::PlayStatus;
use windows::Win32::Foundation::HWND;

use super::lyrics_feed::{Body, Follow, LyricsFeed};
use super::widgets::{
    cover_for, ghost_button, icon, icon_button, round_play_button, set_if_changed, text_tooltip,
};
use crate::platform_window;
use crate::store::AppStore;
use crate::theme;

/// The progress line is a couple of pixels thick, so a slower tick still reads as smooth.
const REPAINT_EVERY: Duration = Duration::from_millis(500);
/// A resize is saved once the window has been still this long.
const SAVE_SIZE_AFTER: Duration = Duration::from_millis(400);
/// Sizes this close count as the same, since pixels round at some scales.
const SIZE_TOLERANCE: f32 = 1.5;
const SKELETON_WIDTHS: [f32; 4] = [0.9, 0.7, 0.8, 0.55];
const NOTE: &str = "\u{266a}";

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Area {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Area {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

impl From<Bounds<gpui_kit::Pixels>> for Area {
    fn from(bounds: Bounds<gpui_kit::Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        }
    }
}

/// A connected display: the whole screen and the part the taskbar leaves free.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Display {
    bounds: Area,
    work: Area,
}

/// The smallest the window can be resized to.
fn min_size(lyrics: bool) -> (f32, f32) {
    if lyrics {
        (theme::MINI_MIN_WIDTH, theme::MINI_LYRICS_MIN_HEIGHT)
    } else {
        (theme::MINI_MIN_WIDTH, theme::MINI_MIN_HEIGHT)
    }
}

/// The window's size in logical pixels: the one the user left it at, else the default.
fn size_for(config: &Config, lyrics: bool) -> (f32, f32) {
    let (saved, default) = if lyrics {
        (
            config.mini_player_lyrics_size,
            (theme::MINI_WIDTH, theme::MINI_LYRICS_HEIGHT),
        )
    } else {
        (
            config.mini_player_size,
            (theme::MINI_WIDTH, theme::MINI_HEIGHT),
        )
    };
    let (width, height) = saved.map_or(default, |[width, height]| (width, height));
    let (min_width, min_height) = min_size(lyrics);

    (width.max(min_width), height.max(min_height))
}

/// Moves a window of `size` at `corner` fully inside `work`.
fn clamp_into(corner: (f32, f32), size: (f32, f32), work: Area) -> (f32, f32) {
    (
        corner.0.min(work.x + work.width - size.0).max(work.x),
        corner.1.min(work.y + work.height - size.1).max(work.y),
    )
}

/// Where the window's top-left corner goes: the remembered spot, pulled fully
/// into the usable area, while its centre is still on a connected display, else
/// the bottom-right of the primary display's usable area.
fn origin(
    saved: Option<[f32; 2]>,
    window: (f32, f32),
    displays: &[Display],
    primary: Area,
) -> (f32, f32) {
    if let Some([x, y]) = saved {
        let (cx, cy) = (x + window.0 / 2., y + window.1 / 2.);

        if let Some(display) = displays
            .iter()
            .find(|display| display.bounds.contains(cx, cy))
        {
            return clamp_into((x, y), window, display.work);
        }
    }

    clamp_into(
        (
            primary.x + primary.width - window.0 - theme::MINI_MARGIN,
            primary.y + primary.height - window.1 - theme::MINI_MARGIN,
        ),
        window,
        primary,
    )
}

/// Stores the window's position for the next time it opens.
pub fn remember_position(window: &Window, core: &Core) {
    let corner = window.bounds().origin;

    core.config.update(|config| {
        config.mini_player_position = Some([f32::from(corner.x), f32::from(corner.y)]);
    });
}

/// Opens the mini player. `saved` is the remembered position, if any.
pub fn open(
    store: Entity<AppStore>,
    saved: Option<[f32; 2]>,
    lyrics: bool,
    core: std::sync::Arc<Core>,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let displays: Vec<Display> = cx
        .displays()
        .iter()
        .map(|display| Display {
            bounds: display.bounds().into(),
            work: display.visible_bounds().into(),
        })
        .collect();
    let primary = cx
        .primary_display()
        .map_or_else(
            || displays.first().map(|display| display.work),
            |display| Some(display.visible_bounds().into()),
        )
        .unwrap_or(Area {
            x: 0.,
            y: 0.,
            width: theme::MINI_WIDTH,
            height: theme::MINI_HEIGHT,
        });
    let window_size = size_for(&core.config.get(), lyrics);
    let (x, y) = origin(saved, window_size, &displays, primary);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(x), px(y)),
            size(px(window_size.0), px(window_size.1)),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        window_background: WindowBackgroundAppearance::Transparent,
        focus: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        let hwnd = platform_window::hwnd(window);

        if let Some(hwnd) = hwnd {
            let (min_width, min_height) = min_size(lyrics);

            platform_window::make_borderless(hwnd);
            platform_window::hide_outline(hwnd);
            platform_window::never_activate(hwnd);
            platform_window::set_min_size(min_width, min_height);
        }

        window.on_window_should_close(cx, move |window, _| {
            remember_position(window, &core);

            true
        });

        cx.new(|cx| MiniPlayer::new(store, hwnd, lyrics, window, cx))
    })?;

    // The kit's root paints the theme background under every window, which
    // would keep the mini player solid at any opacity.
    handle.update(cx, |_, window, cx| {
        gpui_kit::base::Root::update(window, cx, |root, _, cx| {
            root.style().background = Some(gpui_kit::transparent_black().into());
            cx.notify();
        });
    })?;

    Ok(handle)
}

pub struct MiniPlayer {
    store: Entity<AppStore>,
    feed: Entity<LyricsFeed>,
    hwnd: Option<HWND>,
    /// Whether the window is laid out for the lyrics.
    lyrics_mode: bool,
    follow: Follow,
    volume: Entity<SliderState>,
    dragging_volume: bool,
    ticker: Option<Task<()>>,
    size_save: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MiniPlayer {
    fn new(
        store: Entity<AppStore>,
        hwnd: Option<HWND>,
        lyrics: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = store.downgrade();
        let feed = store.read(cx).lyrics_feed.clone();
        let volume = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.));
        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| {
                this.sync_mode(cx);
                this.sync_ticker(cx);
                cx.notify();
            }),
            cx.observe(&feed, |_, _, cx| cx.notify()),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.bounds_changed(window, cx);
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
            cx.on_release(move |_, cx| {
                let _ = weak.update(cx, |store, cx| store.mini_player_closed(cx));
            }),
        ];

        Self {
            store,
            feed,
            hwnd,
            lyrics_mode: lyrics,
            follow: Follow::new(),
            volume,
            dragging_volume: false,
            ticker: None,
            size_save: None,
            _subscriptions: subscriptions,
        }
    }

    /// Resizes the window when the store switched between compact and lyrics mode,
    /// which also happens when lyrics are turned off in Settings.
    fn sync_mode(&mut self, cx: &mut Context<Self>) {
        let state = self.store.read(cx);
        let lyrics = state.mini_lyrics_on();

        if lyrics == self.lyrics_mode {
            return;
        }

        let (width, height) = size_for(&state.core.config.get(), lyrics);

        self.lyrics_mode = lyrics;
        self.follow = Follow::new();

        let Some(hwnd) = self.hwnd else {
            return;
        };
        let (min_width, min_height) = min_size(lyrics);

        platform_window::set_min_size(min_width, min_height);

        // The window's resize message calls back into GPUI, which fails while
        // this update is still running, so the window ends up out of step.
        cx.spawn(async move |_, _| platform_window::resize_anchored(hwnd, width, height))
            .detach();
    }

    /// Remembers a size the user resized the window to, once the resizing has stopped.
    fn bounds_changed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let bounds = window.bounds().size;
        let current = (f32::from(bounds.width), f32::from(bounds.height));
        let lyrics = self.lyrics_mode;
        let core = self.store.read(cx).core.clone();
        let known = size_for(&core.config.get(), lyrics);

        if (current.0 - known.0).abs() < SIZE_TOLERANCE
            && (current.1 - known.1).abs() < SIZE_TOLERANCE
        {
            self.size_save = None;

            return;
        }

        self.size_save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_SIZE_AFTER).await;

            core.config.update(|config| {
                let size = Some([current.0, current.1]);

                if lyrics {
                    config.mini_player_lyrics_size = size;
                } else {
                    config.mini_player_size = size;
                }
            });
        }));
    }

    /// Repaints on a timer only while a song plays.
    fn sync_ticker(&mut self, cx: &mut Context<Self>) {
        if !self.store.read(cx).playback.is_playing() {
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
                    .update(cx, |mini, cx| {
                        if !mini.store.read(cx).playback.is_playing() {
                            mini.ticker = None;

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

    fn lyrics_body(&self, body: Body<'_>, can_seek: bool, cx: &Context<Self>) -> AnyElement {
        match body {
            Body::NothingPlaying => message(t!("lyrics.play_something")),
            Body::NotATrack => message(t!("lyrics.songs_only")),
            Body::Loading => skeleton(),
            Body::Synced(lines) => self.synced_list(lines, can_seek, cx),
            Body::Plain(text) => plain_list(text),
            Body::Instrumental => message(t!("lyrics.instrumental")),
            Body::NotFound => message(t!("lyrics.not_found")),
            Body::Failed => {
                let feed = self.feed.clone();

                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_2()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme::text_secondary())
                            .child(t!("lyrics.load_failed")),
                    )
                    .child(
                        ghost_button("mini-lyrics-retry", t!("common.try_again"), true)
                            .occlude()
                            .on_click(move |_, _, cx| feed.update(cx, |feed, cx| feed.retry(cx))),
                    )
                    .into_any_element()
            }
        }
    }

    fn synced_list(
        &self,
        lines: &[sonear_core::LyricLine],
        can_seek: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let store = self.store.clone();
        let active = self.follow.active();

        div()
            .id("mini-lyrics-lines")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(theme::MINI_LYRICS_GAP))
            .pb(px(theme::MINI_LYRICS_END_PADDING))
            .overflow_y_scroll()
            .track_scroll(self.follow.scroll())
            .on_scroll(cx.listener(|this, _: &ScrollEvent, _, _| this.follow.pause()))
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
                    .id(ElementId::from(("mini-lyric-line", index)))
                    .block_mouse_except_scroll()
                    .w_full()
                    .text_size(px(theme::MINI_LYRICS_SIZE))
                    .font_weight(FontWeight::BOLD)
                    .font_family(theme::FONT_DISPLAY)
                    .text_color(color)
                    .child(text.to_owned())
                    .when(can_seek, |this| {
                        this.cursor_pointer().on_click(move |_, _, cx| {
                            store.update(cx, |store, cx| store.seek(at_ms, cx));
                        })
                    })
            }))
            .into_any_element()
    }
}

fn message(text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
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
        .gap(px(theme::MINI_LYRICS_GAP))
        .children(SKELETON_WIDTHS.iter().map(|width| {
            div()
                .h(px(theme::MINI_LYRICS_SIZE))
                .w(relative(*width))
                .rounded(theme::ART_RADIUS_SMALL)
                .bg(theme::skeleton())
        }))
        .into_any_element()
}

fn plain_list(text: &str) -> AnyElement {
    div()
        .id("mini-lyrics-plain")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap_2()
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

impl Render for MiniPlayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_ticker(cx);

        let viewport = window.viewport_size();
        let height = f32::from(viewport.height);
        let state = self.store.read(cx);
        let shown_volume = state.shown_volume(now_ms());
        let opacity = f32::from(state.mini_player_opacity) / 100.;
        let lyrics_available = state.lyrics_enabled;
        let model = state.playback.clone();
        let player = state.core.player.clone();
        let queue = state.core.queue.clone();
        let has_queued = model.is_active
            && model.item.is_some()
            && (state.queue.locked.is_some() || !state.queue.entries.is_empty());
        let item = model.item.clone();
        let playing = model.is_playing();
        let play_enabled = if playing {
            model.can("pause")
        } else {
            item.is_some()
        };
        let fraction = match model.duration_ms() {
            Some(duration) if duration > 0 => {
                (state.shown_position_ms(now_ms()) as f32 / duration as f32).clamp(0., 1.)
            }
            _ => 0.,
        };
        let open_url = item
            .as_ref()
            .and_then(|item| super::player_bar::spotify_url(&item.uri));
        let title = item.as_ref().map_or_else(
            || t!("player.nothing_playing").into_owned(),
            |item| item.name().to_owned(),
        );
        let subtitle = item
            .as_ref()
            .map(|item| item.artist_line())
            .unwrap_or_default();
        let pending = model.status == PlayStatus::Buffering;
        let lyrics_mode = self.lyrics_mode;
        let can_seek = model.can("seek");

        if !self.dragging_volume {
            set_if_changed(&self.volume, f32::from(shown_volume), 0.5, window, cx);
        }

        let volume_icon = match shown_volume {
            0 => IconName::VolumeX,
            1..=49 => IconName::Volume1,
            _ => IconName::Volume2,
        };
        let lyrics_store = self.store.clone();
        let show_store = self.store.clone();
        let close_store = self.store.clone();

        let title_tip: SharedString = if open_url.is_some() {
            t!("common.open_spotify").into_owned().into()
        } else {
            title.clone().into()
        };
        let titles = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id("mini-title")
                    .self_start()
                    .max_w_full()
                    .truncate()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if item.is_some() {
                        theme::text_primary()
                    } else {
                        theme::text_tertiary()
                    })
                    .tooltip(text_tooltip(title_tip))
                    .when_some(open_url, |this, url| {
                        this.cursor_pointer()
                            .occlude()
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    })
                    .child(title),
            )
            .when(item.is_some(), |this| {
                this.child(
                    div()
                        .id("mini-artists")
                        .truncate()
                        .text_size(px(12.))
                        .text_color(theme::text_secondary())
                        .tooltip(text_tooltip(subtitle.clone()))
                        .child(subtitle),
                )
            });

        let lyrics_tip = if lyrics_mode {
            format!("{}\n{}", t!("mini.lyrics.hide"), t!("lyrics.credit"))
        } else {
            t!("mini.lyrics.show").into_owned()
        };
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .mr(px(-theme::MINI_CLOSE_INSET))
            .when(lyrics_available, |this| {
                this.child(
                    icon_button(
                        "mini-lyrics",
                        IconName::MicVocal,
                        theme::MINI_HEADER_GLYPH,
                        theme::MINI_HEADER_BOX,
                        if lyrics_mode {
                            theme::accent()
                        } else {
                            theme::text_secondary()
                        },
                        true,
                    )
                    .occlude()
                    .tooltip(text_tooltip(lyrics_tip))
                    .on_click(move |_, _, cx| {
                        lyrics_store.update(cx, |state, cx| state.toggle_mini_player_lyrics(cx));
                    }),
                )
            })
            .child(
                icon_button(
                    "mini-show",
                    IconName::Maximize2,
                    theme::MINI_HEADER_GLYPH,
                    theme::MINI_HEADER_BOX,
                    theme::text_secondary(),
                    true,
                )
                .occlude()
                .tooltip(text_tooltip(t!("mini.show")))
                .on_click(move |_, _, cx| {
                    show_store.update(cx, |state, cx| state.show_main_window(cx));
                }),
            )
            .child(
                icon_button(
                    "mini-close",
                    IconName::X,
                    theme::MINI_HEADER_GLYPH,
                    theme::MINI_HEADER_BOX,
                    theme::text_secondary(),
                    true,
                )
                .occlude()
                .tooltip(text_tooltip(t!("common.close")))
                .on_click(move |_, _, cx| {
                    close_store.update(cx, |state, cx| state.toggle_mini_player(cx));
                }),
            );

        let transport = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child({
                let player = player.clone();

                icon_button(
                    "mini-previous",
                    IconName::SkipBack,
                    theme::MINI_TRANSPORT_GLYPH,
                    theme::MINI_TRANSPORT_BOX,
                    theme::text_primary(),
                    model.can("skip_prev"),
                )
                .ml(px(-theme::MINI_PREVIOUS_INSET))
                .occlude()
                .on_click(move |_, _, _| player.previous())
            })
            .child({
                let player = player.clone();
                let mut button = round_play_button(
                    "mini-play-pause",
                    playing,
                    theme::MINI_PLAY_SIZE,
                    theme::text_primary(),
                    play_enabled,
                )
                .occlude();

                if pending {
                    button = button.child(super::widgets::pending_overlay(
                        theme::text_primary(),
                        theme::on_accent(),
                    ));
                }

                button.on_click(move |_, _, _| player.toggle_play_pause())
            })
            .child(
                icon_button(
                    "mini-next",
                    IconName::SkipForward,
                    theme::MINI_TRANSPORT_GLYPH,
                    theme::MINI_TRANSPORT_BOX,
                    theme::text_primary(),
                    model.can("skip_next") || has_queued,
                )
                .occlude()
                .on_click(move |_, _, _| queue.next()),
            );

        let volume = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_end()
            .gap_1p5()
            .ml_3()
            .child(icon(
                volume_icon,
                theme::MINI_TRANSPORT_GLYPH,
                theme::text_secondary(),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(theme::MINI_VOLUME_WIDTH))
                    .max_w(px(theme::MINI_VOLUME_MAX_WIDTH))
                    .occlude()
                    .child(Slider::new(&self.volume)),
            );

        let bottom_row = div()
            .flex()
            .flex_none()
            .items_center()
            .child(transport)
            .child(volume);

        let content = if lyrics_mode {
            let feed = self.feed.clone();
            let feed = feed.read(cx);

            self.follow.sync(feed);

            let repaint = self.follow.settle();
            let body = self.lyrics_body(feed.body(), can_seek, cx);

            if repaint {
                cx.notify();
            }

            div()
                .flex()
                .flex_col()
                .gap_2()
                .size_full()
                .p(px(theme::MINI_PADDING))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_start()
                        .gap_3()
                        .child(cover_for(
                            item.as_ref(),
                            theme::MINI_COVER_SMALL,
                            theme::ART_RADIUS_SMALL,
                        ))
                        .child(titles)
                        .child(header),
                )
                .child(div().flex().flex_col().flex_1().min_h_0().child(body))
                .when(item.is_some(), |this| this.child(bottom_row))
        } else {
            let cover = (height - 2. * theme::MINI_PADDING).clamp(1., theme::MINI_COVER_MAX);

            div()
                .flex()
                .items_center()
                .gap_3()
                .size_full()
                .p(px(theme::MINI_PADDING))
                .child(cover_for(item.as_ref(), cover, theme::ART_RADIUS_SMALL))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_between()
                        .flex_1()
                        .min_w_0()
                        .h(px(cover))
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap_3()
                                .child(titles)
                                .child(header),
                        )
                        .when(item.is_some(), |this| this.child(bottom_row)),
                )
        };

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .opacity(opacity)
            .bg(theme::player())
            .font_family(theme::FONT)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(content)
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .h(px(theme::MINI_PROGRESS_HEIGHT))
                    .w(relative(fraction))
                    .bg(theme::accent()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Area = Area {
        x: 0.,
        y: 0.,
        width: 1920.,
        height: 1040.,
    };
    const DISPLAY: Display = Display {
        bounds: SCREEN,
        work: SCREEN,
    };
    const COMPACT: (f32, f32) = (theme::MINI_WIDTH, theme::MINI_HEIGHT);

    fn corner() -> (f32, f32) {
        (
            SCREEN.width - COMPACT.0 - theme::MINI_MARGIN,
            SCREEN.height - COMPACT.1 - theme::MINI_MARGIN,
        )
    }

    #[test]
    fn no_saved_position_goes_to_the_bottom_right() {
        assert_eq!(origin(None, COMPACT, &[DISPLAY], SCREEN), corner());
    }

    #[test]
    fn a_position_on_a_connected_display_is_kept() {
        assert_eq!(
            origin(Some([300., 200.]), COMPACT, &[DISPLAY], SCREEN),
            (300., 200.)
        );
    }

    #[test]
    fn a_position_on_a_missing_display_falls_back() {
        assert_eq!(
            origin(Some([2600., 200.]), COMPACT, &[DISPLAY], SCREEN),
            corner()
        );
        assert_eq!(
            origin(Some([-3000., 0.]), COMPACT, &[DISPLAY], SCREEN),
            corner()
        );
    }

    #[test]
    fn a_second_display_counts() {
        let second = Display {
            bounds: Area { x: 1920., ..SCREEN },
            work: Area { x: 1920., ..SCREEN },
        };

        assert_eq!(
            origin(Some([2600., 200.]), COMPACT, &[DISPLAY, second], SCREEN),
            (2600., 200.)
        );
    }

    #[test]
    fn the_fallback_uses_the_primary_work_area() {
        let work = Area {
            height: 1000.,
            ..SCREEN
        };

        assert_eq!(
            origin(None, COMPACT, &[DISPLAY], work).1,
            1000. - COMPACT.1 - theme::MINI_MARGIN
        );
    }

    #[test]
    fn a_saved_position_is_pulled_fully_on_screen() {
        let lyrics = (theme::MINI_WIDTH, theme::MINI_LYRICS_HEIGHT);
        let work = Area {
            height: 1000.,
            ..SCREEN
        };
        let display = Display {
            bounds: SCREEN,
            work,
        };

        assert_eq!(
            origin(Some([1700., 800.]), lyrics, &[display], work),
            (1920. - lyrics.0, 1000. - lyrics.1)
        );
        assert_eq!(
            origin(Some([-100., -30.]), COMPACT, &[display], work),
            (0., 0.)
        );
    }

    #[test]
    fn the_size_is_the_saved_one_or_the_default_but_never_below_the_minimum() {
        let mut config = Config::default();

        assert_eq!(size_for(&config, false), COMPACT);
        assert_eq!(
            size_for(&config, true),
            (theme::MINI_WIDTH, theme::MINI_LYRICS_HEIGHT)
        );

        config.mini_player_size = Some([420., 120.]);
        config.mini_player_lyrics_size = Some([100., 100.]);

        assert_eq!(size_for(&config, false), (420., 120.));
        assert_eq!(
            size_for(&config, true),
            (theme::MINI_MIN_WIDTH, theme::MINI_LYRICS_MIN_HEIGHT)
        );
    }
}
