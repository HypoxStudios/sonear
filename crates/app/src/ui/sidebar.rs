use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Div, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
    px,
};

use super::widgets::{art, icon, icon_button, logo_mark, spotify_icon};
use crate::store::{AppStore, LibraryTab, Route};
use crate::theme;

struct NavItem {
    id: &'static str,
    label: &'static str,
    icon: IconName,
    route: Route,
}

const LOGO_SIZE: f32 = 34.;

fn nav_items() -> [NavItem; 3] {
    [
        NavItem {
            id: "nav-home",
            label: "Home",
            icon: IconName::House,
            route: Route::Home,
        },
        NavItem {
            id: "nav-search",
            label: "Search",
            icon: IconName::Search,
            route: Route::Search,
        },
        NavItem {
            id: "nav-library",
            label: "Your Library",
            icon: IconName::Library,
            route: Route::Library(LibraryTab::Playlists),
        },
    ]
}

fn row(
    id: impl Into<ElementId>,
    leading: AnyElement,
    label: impl Into<SharedString>,
    active: bool,
    collapsed: bool,
) -> Stateful<Div> {
    let color = if active {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };

    div()
        .id(id)
        .flex()
        .items_center()
        .flex_none()
        .h(px(40.))
        .rounded(theme::CONTROL_RADIUS)
        .border_1()
        .border_color(theme::transparent())
        .cursor_pointer()
        .tab_index(0)
        .text_color(color)
        .text_size(px(14.))
        .when(active, |this| this.bg(theme::active_nav()))
        .when(!active, |this| this.hover(|style| style.bg(theme::hover())))
        .focus_visible(|style| style.border_color(theme::accent()))
        .map(|this| {
            if collapsed {
                this.justify_center().w(px(40.)).child(leading)
            } else {
                this.gap_3()
                    .px_3()
                    .w_full()
                    .child(leading)
                    .child(div().truncate().min_w_0().child(label.into()))
            }
        })
}

fn glyph(name: IconName, collapsed: bool, active: bool) -> AnyElement {
    let color = if active {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };

    icon(name, if collapsed { 18. } else { 17. }, color).into_any_element()
}

pub fn sidebar(store: &Entity<AppStore>, state: &AppStore, collapsed: bool) -> AnyElement {
    let route = state.route.clone();
    let mut column = div()
        .flex()
        .flex_col()
        .flex_none()
        .h_full()
        .w(if collapsed {
            theme::RAIL_WIDTH
        } else {
            theme::SIDEBAR_WIDTH
        })
        .min_h_0()
        .p(px(if collapsed { 11. } else { 14. }))
        .gap_1()
        .rounded(theme::PANEL_RADIUS)
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::panel());

    let header = if collapsed {
        div()
            .flex()
            .justify_center()
            .pb_2()
            .child(logo_mark(LOGO_SIZE))
    } else {
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .pb_3()
            .child(logo_mark(LOGO_SIZE))
            .child(
                div()
                    .flex_1()
                    .text_size(px(19.))
                    .font_weight(FontWeight::BOLD)
                    .font_family(theme::FONT_DISPLAY)
                    .text_color(theme::text_primary())
                    .child("sonear"),
            )
            .child(toggle_button(store, collapsed))
    };

    column = column.child(header);

    for item in nav_items() {
        let store = store.clone();
        let target = item.route.clone();
        let active = route.is_in_section(&item.route);

        column = column.child(
            row(
                item.id,
                glyph(item.icon, collapsed, active),
                item.label,
                active,
                collapsed,
            )
            .on_click(move |_, _, cx| {
                store.update(cx, |state, cx| state.navigate(target.clone(), cx))
            }),
        );
    }

    column = column.child(if collapsed {
        div().h(px(1.)).mx_2().my_2().bg(theme::hairline())
    } else {
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .pt_4()
            .pb_1()
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::text_tertiary())
            .child("YOUR MUSIC")
            .child(spotify_icon())
    });

    let liked_active = route == Route::LikedSongs;
    let mut list = div()
        .id("sidebar-list")
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child({
            let store = store.clone();

            row(
                "nav-liked",
                glyph(IconName::Heart, collapsed, liked_active),
                "Liked Songs",
                liked_active,
                collapsed,
            )
            .on_click(move |_, _, cx| {
                store.update(cx, |state, cx| state.navigate(Route::LikedSongs, cx))
            })
        });

    for playlist in &state.sidebar_playlists {
        let store = store.clone();
        let target = Route::Playlist(playlist.id.clone());
        let active = route == target;
        let leading = art(&playlist.art, &playlist.id, 24., theme::ART_RADIUS_SMALL);

        list = list.child(
            row(
                ElementId::from(SharedString::from(format!("nav-playlist-{}", playlist.id))),
                leading,
                playlist.name.clone(),
                active,
                collapsed,
            )
            .on_click(move |_, _, cx| {
                store.update(cx, |state, cx| state.navigate(target.clone(), cx))
            }),
        );
    }

    column
        .child(list)
        .child(avatar(store, state, collapsed))
        .when(collapsed, |this| {
            this.child(toggle_button(store, collapsed))
        })
        .into_any_element()
}

fn toggle_button(store: &Entity<AppStore>, collapsed: bool) -> Stateful<Div> {
    let store = store.clone();
    let (name, id): (IconName, SharedString) = if collapsed {
        (IconName::PanelLeftOpen, "sidebar-expand".into())
    } else {
        (IconName::PanelLeftClose, "sidebar-collapse".into())
    };

    icon_button(id, name, 17., 32., theme::text_tertiary(), true).on_click(move |_, _, cx| {
        store.update(cx, |state, cx| state.toggle_sidebar(cx));
    })
}

fn avatar(store: &Entity<AppStore>, state: &AppStore, collapsed: bool) -> Div {
    let profile = state.profile.value();
    let initial = profile
        .and_then(|profile| profile.display_name.chars().next())
        .map(|letter| letter.to_uppercase().to_string());
    let tile = div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(px(32.))
        .rounded(px(16.))
        .overflow_hidden()
        .bg(theme::active_nav())
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_secondary());
    let tile = match (profile, initial) {
        (Some(profile), _) if !profile.art.is_empty() => {
            tile.child(art(&profile.art, &profile.id, 32., px(16.)))
        }
        (_, Some(letter)) => tile.child(letter),
        _ => tile.child(icon(IconName::User, 15., theme::text_secondary())),
    };
    let name = profile.map(|profile| profile.display_name.clone());
    let store = store.clone();
    let button = div()
        .id("avatar")
        .flex()
        .items_center()
        .gap_2()
        .rounded(theme::CONTROL_RADIUS)
        .border_1()
        .border_color(theme::transparent())
        .cursor_pointer()
        .tab_index(0)
        .hover(|style| style.bg(theme::hover()))
        .focus_visible(|style| style.border_color(theme::accent()))
        .child(tile)
        .when(!collapsed, |this| {
            this.pr_2().child(
                div()
                    .truncate()
                    .min_w_0()
                    .text_size(px(13.))
                    .text_color(theme::text_secondary())
                    .child(name.unwrap_or_default()),
            )
        })
        .on_click(move |_, _, cx| store.update(cx, |state, cx| state.toggle_avatar_menu(cx)));

    div()
        .flex()
        .items_center()
        .when(collapsed, |this| this.justify_center())
        .pt_2()
        .child(button)
}
