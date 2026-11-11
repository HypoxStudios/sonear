use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, AppContext as _, Context, DismissEvent, Div, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription, Window,
    div, px,
};
use rust_i18n::t;
use sonear_core::model::is_track_uri;
use sonear_core::playback::now_ms;
use sonear_soloist::Status;

use super::legal::legal_overlay;
use super::lyrics::LyricsPanel;
use super::pages::artist::ArtistPage;
use super::pages::detail::{DetailPage, Kind};
use super::pages::history::HistoryPage;
use super::pages::home::HomePage;
use super::pages::library::LibraryPage;
use super::pages::search::SearchPage;
use super::pages::settings::SettingsPage;
use super::pages::setup::{SetupView, starting_notice};
use super::pages::show::ShowPage;
use super::palette::PaletteView;
use super::player_bar::PlayerBar;
use super::right_panel::RightPanel;
use super::shortcuts::ShortcutsView;
use super::sidebar::sidebar;
use super::widgets::{accent_button, ghost_button, icon, panel};
use crate::keymap::{
    self, ClearQueue, CloseOverlay, CycleRepeat, GoBack, GoHistory, GoHome, GoLibrary, GoLiked,
    GoSearch, LikeCurrent, NextTrack, OpenPalette, OpenSettings, OpenShortcuts, PreviousTrack,
    SeekBack, SeekForward, StartRadio, ToggleLyrics, ToggleMiniPlayer, TogglePlayPause,
    ToggleQueue, ToggleShuffle, ToggleSidebar, VolumeDown, VolumeUp,
};
use crate::store::{AppStore, Confirm, LibraryTab, Route, SetupStep};
use crate::theme;

/// Days before expiry at which the non-blocking update banner appears.
const EXPIRY_WARNING_DAYS: i64 = 30;
const SEEK_STEP_MS: i64 = 10_000;
const VOLUME_STEP: i16 = 5;

pub struct Shell {
    store: Entity<AppStore>,
    player_bar: Entity<PlayerBar>,
    right_panel: Entity<RightPanel>,
    lyrics_panel: Option<Entity<LyricsPanel>>,
    setup: Entity<SetupView>,
    guided_setup: bool,
    page: Option<(Route, AnyView)>,
    focus: FocusHandle,
    palette: Option<(Entity<PaletteView>, Subscription)>,
    shortcuts: Option<Entity<ShortcutsView>>,
    _subscription: Subscription,
}

impl Shell {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();

        window.focus(&focus, cx);

        Self {
            focus,
            palette: None,
            shortcuts: None,
            player_bar: cx.new(|cx| PlayerBar::new(store.clone(), cx)),
            right_panel: cx.new(|cx| RightPanel::new(store.clone(), cx)),
            lyrics_panel: None,
            setup: cx.new(|cx| SetupView::new(store.clone(), window, cx)),
            guided_setup: false,
            page: None,
            _subscription: cx.observe(&store, |_, _, cx| cx.notify()),
            store,
        }
    }

    fn build_page(&self, route: &Route, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        let store = self.store.clone();

        match route.clone() {
            Route::Home => cx.new(|cx| HomePage::new(store, cx)).into(),
            Route::Search => cx.new(|cx| SearchPage::new(store, window, cx)).into(),
            Route::Library(tab) => cx.new(|cx| LibraryPage::new(store, tab, cx)).into(),
            Route::LikedSongs => cx.new(|cx| DetailPage::new(store, Kind::Liked, cx)).into(),
            Route::History => cx.new(|cx| HistoryPage::new(store, cx)).into(),
            Route::Settings => cx.new(|cx| SettingsPage::new(store, window, cx)).into(),
            Route::Album(id) => cx
                .new(|cx| DetailPage::new(store, Kind::Album(id), cx))
                .into(),
            Route::Playlist(id) => cx
                .new(|cx| DetailPage::new(store, Kind::Playlist(id), cx))
                .into(),
            Route::Artist(id) => cx.new(|cx| ArtistPage::new(store, id, cx)).into(),
            Route::Show(id) => cx.new(|cx| ShowPage::new(store, id, cx)).into(),
        }
    }

    fn main_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let route = state.route.clone();

        if state.needs_setup() {
            return starting_notice();
        }

        if self.page.as_ref().map(|(current, _)| current) != Some(&route) {
            let view = self.build_page(&route, window, cx);

            self.page = Some((route, view));
        }

        match &self.page {
            Some((_, view)) => view
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn expiry_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.store.read(cx);
        let days = state.expiry_days?;

        if state.is_demo()
            || state.is_youtube()
            || days > EXPIRY_WARNING_DAYS
            || matches!(state.soloist, Status::Expired | Status::NotInstalled)
        {
            return None;
        }

        let store = self.store.clone();
        let text = match days {
            ..=0 => t!("shell.banner.expired").into_owned(),
            1 => t!("shell.banner.expires_tomorrow").into_owned(),
            days => t!("shell.banner.expires_in", days = days).into_owned(),
        };

        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_none()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme::hairline())
                .text_size(px(12.))
                .text_color(theme::text_secondary())
                .child(icon(IconName::TriangleAlert, 14., theme::warning()))
                .child(div().flex_1().child(text))
                .child(
                    div()
                        .id("banner-update")
                        .cursor_pointer()
                        .tab_index(0)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::accent())
                        .hover(|style| style.text_color(theme::accent_hover()))
                        .focus_visible(|style| style.text_color(theme::text_primary()))
                        .child(t!("common.download_update"))
                        .on_click(move |_, _, cx| {
                            store.update(cx, |state, cx| state.start_install(cx))
                        }),
                ),
        )
    }

    /// Quiet note in the title bar while Spotify has asked Sonear to wait.
    fn rate_notice(&self, cx: &Context<Self>) -> Option<Div> {
        let rate = self.store.read(cx).rate;
        let text = if rate.quota_exceeded {
            t!("shell.rate.quota")
        } else if rate
            .paused_until
            .is_some_and(|until| until > std::time::Instant::now())
        {
            t!("shell.rate.slow")
        } else {
            return None;
        };

        Some(
            div()
                .absolute()
                .top_0()
                .left(px(16.))
                .h(px(34.))
                .max_w(px(420.))
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .text_color(theme::text_tertiary())
                .child(icon(IconName::TriangleAlert, 12., theme::warning()))
                .child(div().truncate().child(text)),
        )
    }

    fn toast(&self, cx: &Context<Self>) -> Option<Div> {
        let toast = self.store.read(cx).toast.clone()?;
        let store = self.store.clone();
        let action_store = self.store.clone();
        let action = toast.action;

        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(f32::from(theme::PLAYER_HEIGHT)
                    + f32::from(theme::PANEL_GAP) * 2.
                    + 8.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("toast")
                        .flex()
                        .items_center()
                        .gap_3()
                        .max_w(px(560.))
                        .px_4()
                        .py_3()
                        .rounded(theme::CARD_RADIUS)
                        .border_1()
                        .border_color(theme::hairline())
                        .bg(theme::raised())
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(theme::text_primary())
                        .child(div().flex_1().child(toast.text))
                        .when_some(action, |this, action| {
                            this.child(
                                div()
                                    .id("toast-action")
                                    .flex_none()
                                    .cursor_pointer()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::accent())
                                    .hover(|style| style.text_color(theme::text_primary()))
                                    .child(action.label())
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        action_store.update(cx, |state, cx| {
                                            state.run_toast_action(action, cx);
                                        });
                                    }),
                            )
                        })
                        .child(icon(IconName::X, 14., theme::text_tertiary()))
                        .on_click(move |_, _, cx| {
                            store.update(cx, |state, cx| state.dismiss_toast(cx));
                        }),
                ),
        )
    }

    fn avatar_menu(&self, cx: &Context<Self>) -> Option<Stateful<Div>> {
        let state = self.store.read(cx);

        if !state.avatar_menu_open {
            return None;
        }

        let youtube = state.is_youtube();
        let account = state.youtube_account().clone();
        let sign_in = self.store.clone();
        let name = match &account {
            sonear_core::YtAccountState::SignedIn { name, .. } => name.clone(),
            _ => String::new(),
        };
        let name = if name.is_empty() {
            state.profile.value().map_or_else(
                || match state.source() {
                    sonear_core::Source::Spotify => t!("source.name.spotify").into_owned(),
                    sonear_core::Source::Youtube => t!("source.name.youtube").into_owned(),
                },
                |profile| profile.display_name.clone(),
            )
        } else {
            name
        };
        let close = self.store.clone();
        let logout = self.store.clone();
        let shortcuts = self.store.clone();
        let settings = self.store.clone();

        Some(
            div()
                .absolute()
                .inset_0()
                .id("avatar-backdrop")
                .occlude()
                .on_click(move |_, _, cx| {
                    close.update(cx, |state, cx| state.close_avatar_menu(cx));
                })
                .child(
                    div()
                        .absolute()
                        .left(px(f32::from(theme::PANEL_GAP) + 10.))
                        .bottom(px(f32::from(theme::PLAYER_HEIGHT)
                            + f32::from(theme::PANEL_GAP) * 2.
                            + 52.))
                        .w(px(220.))
                        .p_2()
                        .rounded(theme::CARD_RADIUS)
                        .border_1()
                        .border_color(theme::hairline())
                        .bg(theme::raised())
                        .child(
                            div()
                                .px_3()
                                .py_2()
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text_primary())
                                .child(name),
                        )
                        .child(div().h(px(1.)).mx_2().my_1().bg(theme::hairline()))
                        .child(
                            div()
                                .id("menu-settings")
                                .flex()
                                .items_center()
                                .px_3()
                                .h(px(34.))
                                .rounded(theme::CONTROL_RADIUS)
                                .cursor_pointer()
                                .tab_index(0)
                                .text_size(px(13.))
                                .text_color(theme::text_secondary())
                                .hover(|style| {
                                    style.bg(theme::hover()).text_color(theme::text_primary())
                                })
                                .focus_visible(|style| style.bg(theme::hover()))
                                .child(t!("common.settings"))
                                .on_click(move |_, _, cx| {
                                    settings.update(cx, |state, cx| {
                                        state.navigate(Route::Settings, cx)
                                    });
                                }),
                        )
                        .child(
                            div()
                                .id("menu-shortcuts")
                                .flex()
                                .items_center()
                                .px_3()
                                .h(px(34.))
                                .rounded(theme::CONTROL_RADIUS)
                                .cursor_pointer()
                                .tab_index(0)
                                .text_size(px(13.))
                                .text_color(theme::text_secondary())
                                .hover(|style| {
                                    style.bg(theme::hover()).text_color(theme::text_primary())
                                })
                                .focus_visible(|style| style.bg(theme::hover()))
                                .child(t!("common.shortcuts"))
                                .on_click(move |_, _, cx| {
                                    shortcuts.update(cx, |state, cx| state.open_shortcuts(cx));
                                }),
                        )
                        .when(youtube, |this| match &account {
                            sonear_core::YtAccountState::SignedIn { .. } => this.child(
                                div()
                                    .id("menu-yt-signout")
                                    .flex()
                                    .items_center()
                                    .px_3()
                                    .h(px(34.))
                                    .rounded(theme::CONTROL_RADIUS)
                                    .cursor_pointer()
                                    .tab_index(0)
                                    .text_size(px(13.))
                                    .text_color(theme::text_secondary())
                                    .hover(|style| {
                                        style.bg(theme::hover()).text_color(theme::text_primary())
                                    })
                                    .focus_visible(|style| style.bg(theme::hover()))
                                    .child(t!("common.ytm_sign_out"))
                                    .on_click({
                                        let store = self.store.clone();

                                        move |_, _, cx| {
                                            store.update(cx, |state, cx| {
                                                state.ask(Confirm::YtSignOut, cx)
                                            });
                                        }
                                    }),
                            ),
                            sonear_core::YtAccountState::SigningIn => this.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .px_3()
                                    .h(px(34.))
                                    .opacity(0.5)
                                    .text_size(px(13.))
                                    .text_color(theme::text_secondary())
                                    .child(t!("common.ytm_signing_in")),
                            ),
                            other => this
                                .child(
                                    div()
                                        .id("menu-yt-signin")
                                        .flex()
                                        .items_center()
                                        .px_3()
                                        .h(px(34.))
                                        .rounded(theme::CONTROL_RADIUS)
                                        .cursor_pointer()
                                        .tab_index(0)
                                        .text_size(px(13.))
                                        .text_color(theme::text_secondary())
                                        .hover(|style| {
                                            style
                                                .bg(theme::hover())
                                                .text_color(theme::text_primary())
                                        })
                                        .focus_visible(|style| style.bg(theme::hover()))
                                        .child(t!("common.ytm_sign_in"))
                                        .on_click({
                                            let store = sign_in.clone();

                                            move |_, _, cx| {
                                                store.update(cx, |state, cx| {
                                                    state.close_avatar_menu(cx);
                                                    state.sign_in_youtube(cx);
                                                });
                                            }
                                        }),
                                )
                                .when(*other == sonear_core::YtAccountState::Expired, |this| {
                                    this.child(
                                        div()
                                            .px_3()
                                            .pb_1()
                                            .text_size(px(11.))
                                            .text_color(theme::text_tertiary())
                                            .child(t!("common.ytm_session_ended")),
                                    )
                                }),
                        })
                        .when(!youtube, |this| {
                            this.child(
                                div()
                                    .id("menu-logout")
                                    .flex()
                                    .items_center()
                                    .px_3()
                                    .h(px(34.))
                                    .rounded(theme::CONTROL_RADIUS)
                                    .cursor_pointer()
                                    .tab_index(0)
                                    .text_size(px(13.))
                                    .text_color(theme::text_secondary())
                                    .hover(|style| {
                                        style.bg(theme::hover()).text_color(theme::text_primary())
                                    })
                                    .focus_visible(|style| style.bg(theme::hover()))
                                    .child(t!("common.log_out"))
                                    .on_click(move |_, _, cx| {
                                        logout
                                            .update(cx, |state, cx| state.ask(Confirm::Logout, cx));
                                    }),
                            )
                        }),
                ),
        )
    }

    fn confirm_dialog(&self, cx: &Context<Self>) -> Option<Stateful<Div>> {
        let confirm = self.store.read(cx).confirm?;
        let (title, text, action): (SharedString, SharedString, SharedString) = match confirm {
            Confirm::Logout => (
                t!("shell.confirm.logout.title").into(),
                t!("shell.confirm.logout.body").into(),
                t!("shell.confirm.logout.action").into(),
            ),
            Confirm::YtSignOut => (
                t!("shell.confirm.yt_signout.title").into(),
                t!("shell.confirm.yt_signout.body").into(),
                t!("shell.confirm.yt_signout.action").into(),
            ),
            Confirm::ClearHistory => (
                t!("shell.confirm.clear_history.title").into(),
                t!("shell.confirm.clear_history.body").into(),
                t!("shell.confirm.clear_history.action").into(),
            ),
            Confirm::DeleteAll => (
                t!("shell.confirm.delete_all.title").into(),
                if self.store.read(cx).is_youtube() {
                    t!("shell.confirm.delete_all.body_youtube").into()
                } else {
                    t!("shell.confirm.delete_all.body").into()
                },
                t!("shell.confirm.delete_all.action").into(),
            ),
            Confirm::MoveData => {
                let target = self.store.read(cx).pending_data_move.clone()?;

                (
                    t!("shell.confirm.move_data.title").into(),
                    t!(
                        "shell.confirm.move_data.body",
                        path = target.display().to_string()
                    )
                    .into(),
                    t!("shell.confirm.move_data.action").into(),
                )
            }
        };
        let cancel = self.store.clone();
        let accept = self.store.clone();
        let backdrop = self.store.clone();

        Some(
            div()
                .id("confirm-backdrop")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::scrim())
                .on_click(move |_, _, cx| {
                    backdrop.update(cx, |state, cx| state.dismiss_confirm(cx));
                })
                .child(
                    div()
                        .id("confirm-dialog")
                        .flex()
                        .flex_col()
                        .gap_3()
                        .w(px(420.))
                        .p_6()
                        .rounded(theme::PANEL_RADIUS)
                        .border_1()
                        .border_color(theme::hairline())
                        .bg(theme::panel())
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(FontWeight::BOLD)
                                .font_family(theme::FONT_DISPLAY)
                                .text_color(theme::text_primary())
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(theme::text_secondary())
                                .child(text),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .pt_2()
                                .child(
                                    ghost_button("confirm-cancel", t!("common.cancel"), true)
                                        .on_click(move |_, _, cx| {
                                            cancel
                                                .update(cx, |state, cx| state.dismiss_confirm(cx));
                                        }),
                                )
                                .child(
                                    accent_button("confirm-accept", action, None, true).on_click(
                                        move |_, _, cx| {
                                            accept.update(cx, |state, cx| state.accept_confirm(cx));
                                        },
                                    ),
                                ),
                        ),
                ),
        )
    }
}

impl Shell {
    fn full_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let width = f32::from(window.viewport_size().width);
        let (collapsed, right_open, lyrics_showing) = {
            let state = self.store.read(cx);

            (
                state.sidebar_collapsed || width < theme::COLLAPSE_BELOW,
                state.right_panel_open,
                state.lyrics_showing(),
            )
        };
        let gap = f32::from(theme::PANEL_GAP);
        let sidebar_width = f32::from(if collapsed {
            theme::RAIL_WIDTH
        } else {
            theme::SIDEBAR_WIDTH
        });
        let span = width - gap * 4. - sidebar_width;
        let show_right = right_open && span >= theme::RIGHT_PANEL_MIN_SPAN;
        let main_width = if show_right {
            span - f32::from(theme::RIGHT_PANEL_WIDTH)
        } else {
            span
        };

        self.store.read(cx).main_width.set(main_width);

        let banner = self.expiry_banner(cx);
        let content = self.main_content(window, cx);
        let state = self.store.read(cx);
        let sidebar = sidebar(&self.store, state, collapsed);
        let right_style = StyleRefinement::default()
            .flex_none()
            .h_full()
            .w(theme::RIGHT_PANEL_WIDTH);
        let right = show_right.then(|| {
            if lyrics_showing {
                let store = self.store.clone();
                let feed = store.read(cx).lyrics_feed.clone();
                let panel = self
                    .lyrics_panel
                    .get_or_insert_with(|| cx.new(|cx| LyricsPanel::new(store, feed, cx)));

                panel.clone().cached(right_style).into_any_element()
            } else {
                self.right_panel
                    .clone()
                    .cached(right_style)
                    .into_any_element()
            }
        });

        let main = panel()
            .flex_1()
            .min_w_0()
            .children(banner)
            .child(div().flex_1().min_h_0().child(content));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .px(theme::PANEL_GAP)
            .pb(theme::PANEL_GAP)
            .gap(theme::PANEL_GAP)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(theme::PANEL_GAP)
                    .child(sidebar)
                    .child(main)
                    .children(right),
            )
            .child(
                div()
                    .flex_none()
                    .h(theme::PLAYER_HEIGHT)
                    .child(self.player_bar.clone()),
            )
    }

    fn ready(&self, cx: &Context<Self>) -> bool {
        let state = self.store.read(cx);

        state.connected() && !state.needs_setup()
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }

        let store = self.store.clone();
        let palette = cx.new(|cx| PaletteView::new(store, window, cx));
        let subscription = cx.subscribe_in(
            &palette,
            window,
            |shell, _, _: &DismissEvent, window, cx| {
                shell.close_palette(window, cx);
            },
        );

        self.palette = Some((palette, subscription));
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
        } else {
            self.open_palette(window, cx);
        }
    }

    fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.store.read(cx).shortcuts_open;

        self.store.update(cx, |state, cx| {
            if open {
                state.close_shortcuts(cx);
            } else {
                state.open_shortcuts(cx);
            }
        });
        window.focus(&self.focus, cx);
    }

    /// Closes the topmost overlay: the palette, the legal viewer, the shortcut
    /// list, a confirm dialog, then the avatar menu.
    fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);

            return;
        }

        let state = self.store.read(cx);
        let (legal, shortcuts, confirm, menu) = (
            state.legal_open.is_some(),
            state.shortcuts_open,
            state.confirm.is_some(),
            state.avatar_menu_open,
        );

        self.store.update(cx, |state, cx| {
            if legal {
                state.close_legal(cx);
            } else if shortcuts {
                state.close_shortcuts(cx);
            } else if confirm {
                state.dismiss_confirm(cx);
            } else if menu {
                state.toggle_avatar_menu(cx);
            }
        });
        window.focus(&self.focus, cx);
    }

    fn playback(&self, cx: &Context<Self>, act: impl FnOnce(&AppStore)) {
        if self.ready(cx) {
            act(self.store.read(cx));
        }
    }

    fn seek(&self, delta_ms: i64, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }

        self.store.update(cx, |store, cx| {
            let Some(duration) = store.playback.duration_ms() else {
                return;
            };
            let position = store.shown_position_ms(now_ms()) as i64;
            let target = (position + delta_ms).clamp(0, duration as i64);

            store.seek(target as u64, cx);
        });
    }

    fn nudge_volume(&self, delta: i16, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }

        self.store.update(cx, |store, cx| {
            let current = i16::from(store.core.player.volume_target());

            store.set_volume((current + delta).clamp(0, 100) as u8, cx);
        });
    }

    fn like_current(&self, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }

        let uri = self
            .store
            .read(cx)
            .playback
            .item
            .as_ref()
            .map(|item| item.uri.clone())
            .filter(|uri| is_track_uri(uri));

        if let Some(uri) = uri {
            self.store
                .update(cx, |state, cx| state.toggle_like(&uri, cx));
        }
    }

    fn start_radio(&self, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }

        let seed = self
            .store
            .read(cx)
            .playback
            .item
            .as_ref()
            .filter(|item| is_track_uri(&item.uri))
            .map(|item| (item.uri.clone(), item.name().to_owned()));

        self.store.update(cx, |state, cx| match seed {
            Some((uri, name)) => state.start_radio(&uri, &name, cx),
            None => state.show_toast(t!("store.toast.radio_needs_song"), cx),
        });
    }

    fn go(&self, route: Route, cx: &mut Context<Self>) {
        self.store.update(cx, |state, cx| state.navigate(route, cx));
    }

    fn bind_actions(&self, root: Div, cx: &mut Context<Self>) -> Div {
        root.on_action(cx.listener(|shell, _: &OpenPalette, window, cx| {
            shell.toggle_palette(window, cx);
        }))
        .on_action(cx.listener(|shell, _: &OpenShortcuts, window, cx| {
            shell.toggle_shortcuts(window, cx);
        }))
        .on_action(cx.listener(|shell, _: &OpenSettings, _, cx| {
            if shell.ready(cx) {
                shell.go(Route::Settings, cx);
            }
        }))
        .on_action(cx.listener(|shell, _: &CloseOverlay, window, cx| {
            shell.close_overlay(window, cx);
        }))
        .on_action(cx.listener(|shell, _: &TogglePlayPause, _, cx| {
            shell.playback(cx, |state| state.core.player.toggle_play_pause());
        }))
        .on_action(cx.listener(|shell, _: &NextTrack, _, cx| {
            shell.playback(cx, |state| state.core.queue.next());
        }))
        .on_action(cx.listener(|shell, _: &PreviousTrack, _, cx| {
            shell.playback(cx, |state| state.core.player.previous());
        }))
        .on_action(cx.listener(|shell, _: &SeekForward, _, cx| shell.seek(SEEK_STEP_MS, cx)))
        .on_action(cx.listener(|shell, _: &SeekBack, _, cx| shell.seek(-SEEK_STEP_MS, cx)))
        .on_action(cx.listener(|shell, _: &VolumeUp, _, cx| shell.nudge_volume(VOLUME_STEP, cx)))
        .on_action(cx.listener(|shell, _: &VolumeDown, _, cx| {
            shell.nudge_volume(-VOLUME_STEP, cx);
        }))
        .on_action(cx.listener(|shell, _: &ToggleShuffle, _, cx| {
            shell.playback(cx, |state| state.core.player.toggle_shuffle());
        }))
        .on_action(cx.listener(|shell, _: &CycleRepeat, _, cx| {
            shell.playback(cx, |state| state.core.player.cycle_repeat());
        }))
        .on_action(cx.listener(|shell, _: &LikeCurrent, _, cx| shell.like_current(cx)))
        .on_action(cx.listener(|shell, _: &StartRadio, _, cx| shell.start_radio(cx)))
        .on_action(cx.listener(|shell, _: &ToggleQueue, _, cx| {
            shell
                .store
                .update(cx, |state, cx| state.show_queue_panel(cx));
        }))
        .on_action(cx.listener(|shell, _: &ToggleLyrics, _, cx| {
            shell
                .store
                .update(cx, |state, cx| state.show_lyrics_panel(cx));
        }))
        .on_action(cx.listener(|shell, _: &ToggleMiniPlayer, _, cx| {
            shell
                .store
                .update(cx, |state, cx| state.toggle_mini_player(cx));
        }))
        .on_action(cx.listener(|shell, _: &ClearQueue, _, cx| {
            shell.playback(cx, |state| state.core.queue.clear());
        }))
        .on_action(cx.listener(|shell, _: &ToggleSidebar, _, cx| {
            shell.store.update(cx, |state, cx| state.toggle_sidebar(cx));
        }))
        .on_action(cx.listener(|shell, _: &GoHome, _, cx| shell.go(Route::Home, cx)))
        .on_action(cx.listener(|shell, _: &GoSearch, _, cx| shell.go(Route::Search, cx)))
        .on_action(cx.listener(|shell, _: &GoLibrary, _, cx| {
            shell.go(Route::Library(LibraryTab::Playlists), cx);
        }))
        .on_action(cx.listener(|shell, _: &GoLiked, _, cx| shell.go(Route::LikedSongs, cx)))
        .on_action(cx.listener(|shell, _: &GoHistory, _, cx| shell.go(Route::History, cx)))
        .on_action(cx.listener(|shell, _: &GoBack, _, cx| {
            shell.store.update(cx, |state, cx| state.back(cx));
        }))
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }

        let (connected, needs_setup, step) = {
            let state = self.store.read(cx);

            (
                state.connected(),
                state.needs_setup(),
                state.setup_state().step(),
            )
        };

        // A page holds the account's data, so the next login must not reuse it.
        if !connected {
            self.page = None;
        }

        if !needs_setup {
            self.guided_setup = false;
        } else if step != SetupStep::Starting {
            self.guided_setup = true;
        }

        let shortcuts_open = self.store.read(cx).shortcuts_open;

        if !shortcuts_open {
            self.shortcuts = None;
        } else if self.shortcuts.is_none() {
            let store = self.store.clone();

            self.shortcuts = Some(cx.new(|_| ShortcutsView::new(store)));
        }

        let rate = self.rate_notice(cx);
        let toast = self.toast(cx);
        let menu = self.avatar_menu(cx);
        let dialog = self.confirm_dialog(cx);
        let body = if self.guided_setup {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .px(theme::PANEL_GAP)
                .pb(theme::PANEL_GAP)
                .child(self.setup.clone())
        } else {
            self.full_body(window, cx)
        };

        let overlay = || StyleRefinement::default().absolute().inset_0();
        let palette = self
            .palette
            .as_ref()
            .map(|(palette, _)| palette.clone().cached(overlay()));
        let shortcuts = self
            .shortcuts
            .as_ref()
            .map(|shortcuts| shortcuts.clone().cached(overlay()));
        let legal = self
            .store
            .read(cx)
            .legal_open
            .map(|doc| legal_overlay(&self.store, doc));

        self.bind_actions(div(), cx)
            .key_context(keymap::CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::window())
            .text_color(theme::text_primary())
            .font_family(theme::FONT)
            .child(TitleBar::new().bg(theme::window()))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text_secondary())
                    .child("SONEAR"),
            )
            .children(rate)
            .child(body)
            .children(toast)
            .children(menu)
            .children(dialog)
            .children(shortcuts)
            .children(legal)
            .children(palette)
    }
}
