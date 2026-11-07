use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyElement, AnyView, AppContext as _, Context, DismissEvent, Div, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, Stateful,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription, Window, div, px,
};
use sonear_core::playback::now_ms;
use sonear_soloist::Status;

use super::pages::artist::ArtistPage;
use super::pages::detail::{DetailPage, Kind};
use super::pages::history::HistoryPage;
use super::pages::home::HomePage;
use super::pages::library::LibraryPage;
use super::pages::search::SearchPage;
use super::pages::setup::SetupView;
use super::pages::show::ShowPage;
use super::palette::PaletteView;
use super::player_bar::PlayerBar;
use super::right_panel::RightPanel;
use super::shortcuts::ShortcutsView;
use super::sidebar::sidebar;
use super::widgets::{accent_button, ghost_button, icon, panel};
use crate::keymap::{
    self, ClearQueue, CloseOverlay, CycleRepeat, GoBack, GoHistory, GoHome, GoLibrary, GoLiked,
    GoSearch, LikeCurrent, NextTrack, OpenPalette, OpenShortcuts, PreviousTrack, SeekBack,
    SeekForward, TogglePlayPause, ToggleQueue, ToggleShuffle, ToggleSidebar, VolumeDown, VolumeUp,
};
use crate::store::{AppStore, Confirm, LibraryTab, Route};
use crate::theme;

/// Days before expiry at which the non-blocking update banner appears.
const EXPIRY_WARNING_DAYS: i64 = 30;
const SEEK_STEP_MS: i64 = 10_000;
const VOLUME_STEP: i16 = 5;

/// Whether the setup screen replaces the pages.
fn needs_setup(state: &AppStore) -> bool {
    (!state.is_demo() && (!matches!(state.soloist, Status::Ready { .. }) || !state.connected()))
        || (state.is_demo() && std::env::var("SONEAR_DEMO_SETUP").is_ok())
}

pub struct Shell {
    store: Entity<AppStore>,
    player_bar: Entity<PlayerBar>,
    right_panel: Entity<RightPanel>,
    setup: Entity<SetupView>,
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
            setup: cx.new(|cx| SetupView::new(store.clone(), window, cx)),
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
        let connected = state.connected();
        let needs_setup = needs_setup(state);

        // A page holds the account's data, so the next login must not reuse it.
        if !connected {
            self.page = None;
        }

        if needs_setup {
            return self.setup.clone().into_any_element();
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
            || days > EXPIRY_WARNING_DAYS
            || matches!(state.soloist, Status::Expired | Status::NotInstalled)
        {
            return None;
        }

        let store = self.store.clone();
        let text = match days {
            ..=0 => "This Soloist build has expired.".to_owned(),
            1 => "This Soloist build expires tomorrow.".to_owned(),
            days => format!("This Soloist build expires in {days} days."),
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
                        .child("Download update")
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
            "Spotify's request quota is used up. It resets later and is shared by your whole developer account."
        } else if rate
            .paused_until
            .is_some_and(|until| until > std::time::Instant::now())
        {
            "Spotify asked Sonear to slow down"
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

        let name = state.profile.value().map_or_else(
            || "Spotify".to_owned(),
            |profile| profile.display_name.clone(),
        );
        let close = self.store.clone();
        let logout = self.store.clone();
        let shortcuts = self.store.clone();

        Some(
            div()
                .absolute()
                .inset_0()
                .id("avatar-backdrop")
                .on_click(move |_, _, cx| {
                    close.update(cx, |state, cx| state.toggle_avatar_menu(cx));
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
                                .child("Keyboard shortcuts")
                                .on_click(move |_, _, cx| {
                                    shortcuts.update(cx, |state, cx| state.open_shortcuts(cx));
                                }),
                        )
                        .child(
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
                                .child("Log out")
                                .on_click(move |_, _, cx| {
                                    logout.update(cx, |state, cx| state.ask(Confirm::Logout, cx));
                                }),
                        ),
                ),
        )
    }

    fn confirm_dialog(&self, cx: &Context<Self>) -> Option<Stateful<Div>> {
        let confirm = self.store.read(cx).confirm?;
        let (title, text, action) = match confirm {
            Confirm::Logout => (
                "Log out of Spotify?",
                "This removes your Spotify login, cached data, cover art, listening history and queue from this computer. Your Soloist key, install and pairing stay.",
                "Log out",
            ),
            Confirm::ClearHistory => (
                "Clear history?",
                "This deletes the list of plays Sonear keeps on this computer. Nothing changes in your Spotify account.",
                "Clear history",
            ),
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
                                .child(ghost_button("confirm-cancel", "Cancel", true).on_click(
                                    move |_, _, cx| {
                                        cancel.update(cx, |state, cx| state.dismiss_confirm(cx));
                                    },
                                ))
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
    fn ready(&self, cx: &Context<Self>) -> bool {
        let state = self.store.read(cx);

        state.connected() && !needs_setup(state)
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

    /// Closes the topmost overlay: the palette, the shortcut list, a confirm
    /// dialog, then the avatar menu.
    fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);

            return;
        }

        let state = self.store.read(cx);
        let (shortcuts, confirm, menu) = (
            state.shortcuts_open,
            state.confirm.is_some(),
            state.avatar_menu_open,
        );

        self.store.update(cx, |state, cx| {
            if shortcuts {
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

    fn seek(&self, delta_ms: i64, cx: &Context<Self>) {
        self.playback(cx, |state| {
            let Some(duration) = state.playback.duration_ms() else {
                return;
            };
            let position = state.playback.position_ms_at(now_ms()) as i64;
            let target = (position + delta_ms).clamp(0, duration as i64);

            state.core.player.seek(target as u64);
        });
    }

    fn nudge_volume(&self, delta: i16, cx: &Context<Self>) {
        self.playback(cx, |state| {
            let volume = (i16::from(state.playback.volume) + delta).clamp(0, 100);

            state.core.player.set_volume(volume as u8);
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
            .filter(|uri| uri.starts_with("spotify:track:"));

        if let Some(uri) = uri {
            self.store
                .update(cx, |state, cx| state.toggle_like(&uri, cx));
        }
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
        .on_action(cx.listener(|shell, _: &ToggleQueue, _, cx| {
            shell
                .store
                .update(cx, |state, cx| state.toggle_right_panel(cx));
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

        let shortcuts_open = self.store.read(cx).shortcuts_open;

        if !shortcuts_open {
            self.shortcuts = None;
        } else if self.shortcuts.is_none() {
            let store = self.store.clone();

            self.shortcuts = Some(cx.new(|_| ShortcutsView::new(store)));
        }

        let width = f32::from(window.viewport_size().width);
        let (collapsed, right_open) = {
            let state = self.store.read(cx);

            (
                state.sidebar_collapsed || width < theme::COLLAPSE_BELOW,
                state.right_panel_open,
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
        let rate = self.rate_notice(cx);
        let toast = self.toast(cx);
        let menu = self.avatar_menu(cx);
        let dialog = self.confirm_dialog(cx);
        let state = self.store.read(cx);
        let sidebar = sidebar(&self.store, state, collapsed);
        let right = show_right.then(|| {
            self.right_panel.clone().cached(
                StyleRefinement::default()
                    .flex_none()
                    .h_full()
                    .w(theme::RIGHT_PANEL_WIDTH),
            )
        });

        let main = panel()
            .flex_1()
            .min_w_0()
            .children(banner)
            .child(div().flex_1().min_h_0().child(content));

        let overlay = || StyleRefinement::default().absolute().inset_0();
        let palette = self
            .palette
            .as_ref()
            .map(|(palette, _)| palette.clone().cached(overlay()));
        let shortcuts = self
            .shortcuts
            .as_ref()
            .map(|shortcuts| shortcuts.clone().cached(overlay()));

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
            .child(
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
                    ),
            )
            .children(toast)
            .children(menu)
            .children(dialog)
            .children(shortcuts)
            .children(palette)
    }
}
