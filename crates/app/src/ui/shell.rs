use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyElement, AnyView, AppContext as _, Context, Div, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use sonear_soloist::Status;

use super::pages::artist::ArtistPage;
use super::pages::detail::{DetailPage, Kind};
use super::pages::history::HistoryPage;
use super::pages::home::HomePage;
use super::pages::library::LibraryPage;
use super::pages::search::SearchPage;
use super::pages::setup::SetupView;
use super::pages::show::ShowPage;
use super::player_bar::PlayerBar;
use super::right_panel::right_panel;
use super::sidebar::sidebar;
use super::widgets::{accent_button, ghost_button, icon, panel};
use crate::store::{AppStore, Confirm, Route};
use crate::theme;

/// Days before expiry at which the non-blocking update banner appears.
const EXPIRY_WARNING_DAYS: i64 = 30;

pub struct Shell {
    store: Entity<AppStore>,
    player_bar: Entity<PlayerBar>,
    setup: Entity<SetupView>,
    page: Option<(Route, AnyView)>,
    _subscription: Subscription,
}

impl Shell {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            player_bar: cx.new(|cx| PlayerBar::new(store.clone(), cx)),
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
        let needs_setup = (!state.is_demo()
            && (!matches!(state.soloist, Status::Ready { .. }) || !connected))
            || (state.is_demo() && std::env::var("SONEAR_DEMO_SETUP").is_ok());

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
            Some((_, view)) => view.clone().into_any_element(),
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
                "This removes your Spotify login, cached data, cover art and listening history from this computer. Your Soloist key, install and pairing stay.",
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

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let right = show_right.then(|| right_panel(&self.store, state));

        let main = panel()
            .flex_1()
            .min_w_0()
            .children(banner)
            .child(div().flex_1().min_h_0().child(content));

        div()
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
    }
}
