use rust_i18n::t;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, WindowBounds,
    WindowControlArea, WindowKind, WindowOptions, div, point, px, relative, size,
};
use sonear_core::Core;
use sonear_core::playback::now_ms;
use sonear_soloist::protocol::PlayStatus;

use super::widgets::{cover_for, icon_button, round_play_button, spotify_icon_bare, text_tooltip};
use crate::store::AppStore;
use crate::theme;

/// The progress line is a couple of pixels thick, so a slower tick still reads as smooth.
const REPAINT_EVERY: Duration = Duration::from_millis(500);

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

/// Where the window's top-left corner goes: the remembered spot while its
/// centre is still on a connected display, else the bottom-right of the
/// primary display's usable area.
fn origin(saved: Option<[f32; 2]>, displays: &[Area], primary: Area) -> (f32, f32) {
    let centre = |[x, y]: [f32; 2]| (x + theme::MINI_WIDTH / 2., y + theme::MINI_HEIGHT / 2.);

    if let Some(saved) = saved {
        let (cx, cy) = centre(saved);

        if displays.iter().any(|display| display.contains(cx, cy)) {
            return (saved[0], saved[1]);
        }
    }

    (
        primary.x + primary.width - theme::MINI_WIDTH - theme::MINI_MARGIN,
        primary.y + primary.height - theme::MINI_HEIGHT - theme::MINI_MARGIN,
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
    core: std::sync::Arc<Core>,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let displays: Vec<Area> = cx
        .displays()
        .iter()
        .map(|display| display.bounds().into())
        .collect();
    let primary = cx
        .primary_display()
        .map_or_else(
            || displays.first().copied(),
            |display| Some(display.visible_bounds().into()),
        )
        .unwrap_or(Area {
            x: 0.,
            y: 0.,
            width: theme::MINI_WIDTH,
            height: theme::MINI_HEIGHT,
        });
    let (x, y) = origin(saved, &displays, primary);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(x), px(y)),
            size(px(theme::MINI_WIDTH), px(theme::MINI_HEIGHT)),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        if let Some(hwnd) = crate::platform_window::hwnd(window) {
            crate::platform_window::make_borderless(hwnd);
        }

        window.on_window_should_close(cx, move |window, _| {
            remember_position(window, &core);

            true
        });

        cx.new(|cx| MiniPlayer::new(store, cx))
    })?;

    Ok(handle)
}

pub struct MiniPlayer {
    store: Entity<AppStore>,
    ticker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MiniPlayer {
    fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let weak = store.downgrade();
        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| {
                this.sync_ticker(cx);
                cx.notify();
            }),
            cx.on_release(move |_, cx| {
                let _ = weak.update(cx, |store, cx| store.mini_player_closed(cx));
            }),
        ];

        Self {
            store,
            ticker: None,
            _subscriptions: subscriptions,
        }
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
}

impl Render for MiniPlayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_ticker(cx);

        let state = self.store.read(cx);
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
        let show_store = self.store.clone();
        let close_store = self.store.clone();

        let titles = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id("mini-title")
                    .truncate()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if item.is_some() {
                        theme::text_primary()
                    } else {
                        theme::text_tertiary()
                    })
                    .tooltip(text_tooltip(title.clone()))
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

        let top = div()
            .flex()
            .items_start()
            .gap_3()
            .child(titles)
            .when(item.is_some(), |this| {
                this.child(
                    div()
                        .id("mini-spotify")
                        .tooltip(text_tooltip(t!("common.open_spotify")))
                        .child(spotify_icon_bare())
                        .when_some(open_url, |this, url| {
                            this.cursor_pointer()
                                .occlude()
                                .on_click(move |_, _, cx| cx.open_url(&url))
                        }),
                )
            });

        let transport = div()
            .flex()
            .items_center()
            .gap_1()
            .child({
                let player = player.clone();

                icon_button(
                    "mini-previous",
                    IconName::SkipBack,
                    16.,
                    28.,
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
                    30.,
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
                    16.,
                    28.,
                    theme::text_primary(),
                    model.can("skip_next") || has_queued,
                )
                .occlude()
                .on_click(move |_, _, _| queue.next()),
            );

        let window_buttons = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                icon_button(
                    "mini-show",
                    IconName::Maximize2,
                    12.,
                    22.,
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
                    12.,
                    22.,
                    theme::text_secondary(),
                    true,
                )
                .mr(px(-theme::MINI_CLOSE_INSET))
                .occlude()
                .tooltip(text_tooltip(t!("common.close")))
                .on_click(move |_, _, cx| {
                    close_store.update(cx, |state, cx| state.toggle_mini_player(cx));
                }),
            );

        let content = div()
            .flex()
            .flex_col()
            .justify_between()
            .flex_1()
            .min_w_0()
            .h(px(theme::MINI_COVER))
            .child(top)
            .when(item.is_some(), |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(transport)
                        .child(window_buttons),
                )
            });

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme::player())
            .font_family(theme::FONT)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .size_full()
                    .p(px(theme::MINI_PADDING))
                    .child(cover_for(
                        item.as_ref(),
                        theme::MINI_COVER,
                        theme::ART_RADIUS_SMALL,
                    ))
                    .child(content),
            )
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

    fn corner() -> (f32, f32) {
        (
            SCREEN.width - theme::MINI_WIDTH - theme::MINI_MARGIN,
            SCREEN.height - theme::MINI_HEIGHT - theme::MINI_MARGIN,
        )
    }

    #[test]
    fn no_saved_position_goes_to_the_bottom_right() {
        assert_eq!(origin(None, &[SCREEN], SCREEN), corner());
    }

    #[test]
    fn a_position_on_a_connected_display_is_kept() {
        assert_eq!(origin(Some([300., 200.]), &[SCREEN], SCREEN), (300., 200.));
    }

    #[test]
    fn a_position_on_a_missing_display_falls_back() {
        assert_eq!(origin(Some([2600., 200.]), &[SCREEN], SCREEN), corner());
        assert_eq!(origin(Some([-3000., 0.]), &[SCREEN], SCREEN), corner());
    }

    #[test]
    fn a_second_display_counts() {
        let second = Area { x: 1920., ..SCREEN };

        assert_eq!(
            origin(Some([2600., 200.]), &[SCREEN, second], SCREEN),
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
            origin(None, &[SCREEN], work).1,
            1000. - theme::MINI_HEIGHT - theme::MINI_MARGIN
        );
    }
}
