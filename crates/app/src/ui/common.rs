//! Pieces shared by the library pages: cards, rows, skeletons, errors and the breadcrumb.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};
use serde::{Deserialize, Serialize};
use sonear_core::LibraryError;
use sonear_core::model::{Album, Artist, Artwork, ItemKind, Page, Playlist, Show, Track};

use crate::format::clock;
use crate::store::{AppStore, LibraryTab, Route};
use crate::theme;
use crate::ui::widgets::{accent_button, art, heart_button, icon, icon_button};

pub const ROW_HEIGHT: f32 = 52.;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardKind {
    Playlist,
    Album,
    Artist,
    Show,
}

/// Anything shown as a cover card that opens a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardItem {
    pub kind: CardKind,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub art: Artwork,
}

impl CardItem {
    pub fn route(&self) -> Route {
        match self.kind {
            CardKind::Playlist => Route::Playlist(self.id.clone()),
            CardKind::Album => Route::Album(self.id.clone()),
            CardKind::Artist => Route::Artist(self.id.clone()),
            CardKind::Show => Route::Show(self.id.clone()),
        }
    }
}

impl From<&Playlist> for CardItem {
    fn from(playlist: &Playlist) -> Self {
        let count = playlist
            .track_count
            .map(|count| format!("{count} tracks"))
            .unwrap_or_else(|| "Playlist".to_owned());
        let subtitle = match &playlist.owner_name {
            Some(owner) if playlist.track_count.is_some() => format!("{count} \u{00b7} {owner}"),
            _ => count,
        };

        Self {
            kind: CardKind::Playlist,
            id: playlist.id.clone(),
            title: playlist.name.clone(),
            subtitle,
            art: playlist.art.clone(),
        }
    }
}

impl From<&Album> for CardItem {
    fn from(album: &Album) -> Self {
        let year = album.year().map(|year| format!("{year} \u{00b7} "));

        Self {
            kind: CardKind::Album,
            id: album.id.clone(),
            title: album.name.clone(),
            subtitle: format!("{}{}", year.unwrap_or_default(), album.artist_line()),
            art: album.art.clone(),
        }
    }
}

impl From<&Artist> for CardItem {
    fn from(artist: &Artist) -> Self {
        Self {
            kind: CardKind::Artist,
            id: artist.id.clone(),
            title: artist.name.clone(),
            subtitle: "Artist".to_owned(),
            art: artist.art.clone(),
        }
    }
}

impl From<&Show> for CardItem {
    fn from(show: &Show) -> Self {
        Self {
            kind: CardKind::Show,
            id: show.id.clone(),
            title: show.name.clone(),
            subtitle: show.publisher.clone(),
            art: show.art.clone(),
        }
    }
}

pub fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(17.))
        .font_weight(FontWeight::BOLD)
        .font_family(theme::FONT_DISPLAY)
        .text_color(theme::text_primary())
        .child(text.into())
}

pub fn card_text(title: &str, subtitle: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(
            div()
                .truncate()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text_primary())
                .child(title.to_owned()),
        )
        .child(
            div()
                .truncate()
                .text_size(px(12.))
                .text_color(theme::text_secondary())
                .child(subtitle.to_owned()),
        )
}

/// A cover with its name underneath. Opens the item's page when clicked.
pub fn card(index: usize, item: &CardItem, size: f32, store: &Entity<AppStore>) -> AnyElement {
    let store = store.clone();
    let route = item.route();
    let radius = if item.kind == CardKind::Artist {
        px(size / 2.)
    } else {
        theme::CARD_RADIUS
    };

    div()
        .id(ElementId::from((
            SharedString::from(format!("card-{:?}", item.kind)),
            index,
        )))
        .flex()
        .flex_col()
        .gap_2()
        .flex_none()
        .w(px(size))
        .p(px(0.))
        .rounded(theme::CARD_RADIUS)
        .border_1()
        .border_color(theme::transparent())
        .cursor_pointer()
        .tab_index(0)
        .focus_visible(|style| style.border_color(theme::accent()))
        .child(art(&item.art, &item.id, size - 2., radius))
        .child(card_text(&item.title, &item.subtitle))
        .on_click(move |_, _, cx| store.update(cx, |state, cx| state.navigate(route.clone(), cx)))
        .into_any_element()
}

pub fn skeleton_card(size: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .flex_none()
        .w(px(size))
        .child(
            div()
                .size(px(size))
                .rounded(theme::CARD_RADIUS)
                .bg(theme::skeleton()),
        )
        .child(
            div()
                .h(px(11.))
                .w(px(size * 0.7))
                .rounded(px(4.))
                .bg(theme::skeleton()),
        )
        .child(
            div()
                .h(px(10.))
                .w(px(size * 0.45))
                .rounded(px(4.))
                .bg(theme::skeleton()),
        )
}

pub fn skeleton_row(index: usize) -> Div {
    let title = 120. + (index % 4) as f32 * 40.;

    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .h(px(ROW_HEIGHT))
        .px_3()
        .child(div().size(px(36.)).rounded(px(6.)).bg(theme::skeleton()))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .flex_1()
                .child(
                    div()
                        .h(px(11.))
                        .w(px(title))
                        .rounded(px(4.))
                        .bg(theme::skeleton()),
                )
                .child(
                    div()
                        .h(px(10.))
                        .w(px(80.))
                        .rounded(px(4.))
                        .bg(theme::skeleton()),
                ),
        )
        .child(
            div()
                .h(px(10.))
                .w(px(36.))
                .rounded(px(4.))
                .bg(theme::skeleton()),
        )
}

pub fn skeleton_rows(count: usize) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .children((0..count).map(skeleton_row))
}

/// What went wrong in words, plus a way to try again.
pub fn error_panel(
    id: &'static str,
    error: &LibraryError,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let retry = !matches!(error, LibraryError::Forbidden | LibraryError::NotFound);

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .w_full()
        .p_8()
        .child(icon(IconName::TriangleAlert, 26., theme::warning()))
        .child(
            div()
                .max_w(px(440.))
                .text_center()
                .text_size(px(14.))
                .text_color(theme::text_secondary())
                .child(error.to_string()),
        )
        .when(retry, |this| {
            this.child(
                accent_button(
                    id,
                    "Retry",
                    Some(icon(IconName::RefreshCw, 15., theme::on_accent())),
                    true,
                )
                .on_click(on_retry),
            )
        })
        .into_any_element()
}

/// Puts a "couldn't load more" bar under a list when a later page failed.
pub fn with_retry_bar(
    list: AnyElement,
    error: Option<&LibraryError>,
    id: &'static str,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let Some(error) = error else {
        return list;
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(div().flex_1().min_h_0().child(list))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .flex_none()
                .px_6()
                .py_3()
                .border_t_1()
                .border_color(theme::hairline())
                .text_size(px(13.))
                .text_color(theme::text_secondary())
                .child(icon(IconName::TriangleAlert, 14., theme::warning()))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .child(format!("Couldn\u{2019}t load more. {error}")),
                )
                .child(accent_button(id, "Retry", None, true).on_click(on_retry)),
        )
        .into_any_element()
}

/// Back arrow, the page you came from, and the current page.
pub fn breadcrumb(store: &Entity<AppStore>, state: &AppStore, current: &str) -> Div {
    let back = store.clone();
    let parent = store.clone();
    let can_go_back = state.can_go_back();
    let label = state.previous_route().map_or("Your Library", Route::label);

    div()
        .flex()
        .items_center()
        .gap_1()
        .flex_none()
        .px_6()
        .pt_4()
        .h(px(44.))
        .text_size(px(13.))
        .text_color(theme::text_secondary())
        .child(
            icon_button(
                "crumb-back",
                IconName::ChevronLeft,
                16.,
                28.,
                theme::text_secondary(),
                can_go_back,
            )
            .when(can_go_back, |this| {
                this.on_click(move |_, _, cx| back.update(cx, |state, cx| state.back(cx)))
            }),
        )
        .child(
            div()
                .id("crumb-parent")
                .cursor_pointer()
                .hover(|style| style.text_color(theme::text_primary()))
                .child(label)
                .on_click(move |_, _, cx| {
                    parent.update(cx, |state, cx| {
                        if state.can_go_back() {
                            state.back(cx);
                        } else {
                            state.navigate(Route::Library(LibraryTab::Playlists), cx);
                        }
                    })
                }),
        )
        .child(div().px_1().text_color(theme::text_tertiary()).child("/"))
        .child(
            div()
                .truncate()
                .min_w_0()
                .text_color(theme::text_primary())
                .child(current.to_owned()),
        )
}

/// A link that opens a Spotify page in the browser.
pub fn open_in_spotify(id: &'static str, url: Option<String>) -> Option<AnyElement> {
    let url = url?;

    Some(
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_1()
            .cursor_pointer()
            .tab_index(0)
            .text_size(px(12.))
            .text_color(theme::text_secondary())
            .hover(|style| style.text_color(theme::accent()))
            .focus_visible(|style| style.text_color(theme::accent()))
            .child("Open in Spotify")
            .child(icon(IconName::ExternalLink, 12., theme::text_secondary()))
            .on_click(move |_, _, cx: &mut App| cx.open_url(&url))
            .into_any_element(),
    )
}

pub struct TrackRowSpec<'a> {
    pub index: usize,
    pub track: &'a Track,
    pub current: bool,
    /// `None` hides the heart.
    pub liked: Option<bool>,
    pub show_art: bool,
    pub show_album: bool,
    pub show_number: bool,
    /// Replaces the duration, e.g. with when a track was played.
    pub trailing: Option<String>,
    /// `Some` shows an add-to-queue button on hover.
    pub queue_store: Option<&'a Entity<AppStore>>,
}

pub fn track_row(
    id_prefix: &'static str,
    spec: &TrackRowSpec<'_>,
    on_play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_like: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let track = spec.track;
    let playable = track.playable && !track.is_local;
    let color = if spec.current {
        theme::accent()
    } else {
        theme::text_primary()
    };
    let marker = if spec.current {
        icon(IconName::Music, 14., theme::accent()).into_any_element()
    } else {
        div().child((spec.index + 1).to_string()).into_any_element()
    };
    let album_name = track
        .album
        .as_ref()
        .map(|album| album.name.clone())
        .unwrap_or_default();
    let trailing = spec.trailing.clone().unwrap_or_else(|| {
        if track.duration_ms == 0 {
            String::new()
        } else {
            clock(track.duration_ms)
        }
    });
    let heart = spec
        .liked
        .filter(|_| track.kind == ItemKind::Track && !track.is_local);
    let queueable = track.kind == ItemKind::Track && !track.is_local && track.playable;
    let queue = spec
        .queue_store
        .filter(|_| queueable)
        .map(|store| (store.clone(), track.clone()));
    let group = SharedString::from(format!("{id_prefix}-row-{}", spec.index));

    div()
        .w_full()
        .px_5()
        .child(
            div()
                .id(ElementId::from((id_prefix, spec.index)))
                .group(group.clone())
                .flex()
                .items_center()
                .gap_3()
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_3()
                .rounded(theme::CONTROL_RADIUS)
                .border_b_1()
                .border_color(theme::hairline())
                .text_size(px(13.))
                .when(playable, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(theme::hover()))
                        .on_click(on_play)
                })
                .when(!playable, |this| this.opacity(0.5))
                .when(spec.show_number, |this| {
                    this.child(
                        div()
                            .w(px(28.))
                            .flex_none()
                            .text_color(theme::text_tertiary())
                            .child(marker),
                    )
                })
                .when(spec.show_art, |this| {
                    this.child(art(&track.art, &track.uri, 36., px(6.)))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_color(color)
                                .font_weight(if spec.current {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .child(track.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(theme::text_secondary())
                                .child(if track.is_local {
                                    format!("{} \u{00b7} Local file", track.artist_line())
                                } else {
                                    track.artist_line()
                                }),
                        ),
                )
                .when(spec.show_album, |this| {
                    this.child(
                        div()
                            .w(px(180.))
                            .flex_none()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .child(album_name),
                    )
                })
                .when_some(queue, |this, (store, track)| {
                    this.child(
                        icon_button(
                            ElementId::from((
                                SharedString::from(format!("{id_prefix}-queue")),
                                spec.index,
                            )),
                            IconName::ListPlus,
                            16.,
                            28.,
                            theme::text_secondary(),
                            true,
                        )
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            store.update(cx, |state, cx| state.add_to_queue(&track, cx));
                        }),
                    )
                })
                .child(
                    div()
                        .w(px(30.))
                        .flex_none()
                        .when_some(heart, |this, liked| {
                            this.child(
                                heart_button(
                                    ElementId::from((
                                        SharedString::from(format!("{id_prefix}-like")),
                                        spec.index,
                                    )),
                                    liked,
                                    15.,
                                    28.,
                                    theme::text_tertiary(),
                                    true,
                                )
                                .on_click(
                                    move |event, window, cx| {
                                        cx.stop_propagation();
                                        on_like(event, window, cx);
                                    },
                                ),
                            )
                        }),
                )
                .child(
                    div()
                        .w(px(if spec.trailing.is_some() { 120. } else { 52. }))
                        .flex_none()
                        .text_right()
                        .text_size(px(12.))
                        .text_color(theme::text_tertiary())
                        .child(trailing),
                ),
        )
        .into_any_element()
}

/// Converts every item of a page, keeping its paging state.
pub fn map_page<T, U>(page: Page<T>, convert: impl FnMut(T) -> U) -> Page<U> {
    Page {
        items: page.items.into_iter().map(convert).collect(),
        total: page.total,
        offset: page.offset,
        has_more: page.has_more,
        cursor: page.cursor,
    }
}

/// Cards per grid row for a panel `width` pixels wide.
pub fn grid_columns(width: f32) -> usize {
    let usable = (width - 48.).max(theme::GRID_CARD);

    (((usable + theme::GRID_GAP) / (theme::GRID_CARD + theme::GRID_GAP)) as usize).max(1)
}

/// Pixel height of one row of the card grid.
pub const GRID_ROW: f32 = theme::GRID_CARD + 62.;

/// A row of cards for a virtual list.
pub fn card_row(prefix: usize, items: &[CardItem], store: &Entity<AppStore>) -> AnyElement {
    div()
        .flex()
        .gap(px(theme::GRID_GAP))
        .px_6()
        .h(px(GRID_ROW))
        .children(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| card(prefix * 64 + index, item, theme::GRID_CARD, store)),
        )
        .into_any_element()
}
