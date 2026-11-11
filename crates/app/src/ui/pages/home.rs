use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, Context, Div, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, div, px,
};
use rust_i18n::t;
use sonear_core::LibraryError;
use sonear_core::history::{Hero, HeroKind};
use sonear_core::model::{Page, Playlist, Shelf, ShelfItems, Track};

use crate::data::{self, Load, Slot};
use crate::format::greeting;
use crate::i18n::plural;
use crate::store::{AppStore, Route};
use crate::theme;
use crate::ui::common::{CardItem, card, error_panel, heading, skeleton_card};
use crate::ui::source_switch::source_switch;
use crate::ui::widgets::{art, icon, pending_overlay, round_play_button, text_tooltip};

const TILE: f32 = 132.;
const SHELF_LEN: usize = 10;
const RECENT_LOOKUPS: usize = 40;

pub struct HomePage {
    store: Entity<AppStore>,
    hero: Slot<Option<Hero>>,
    on_repeat: Slot<Vec<Track>>,
    recent: Slot<Vec<Track>>,
    playlists: Slot<Page<Playlist>>,
    /// Home shelves the source suggests. Empty on Spotify.
    discover: Slot<Vec<Shelf>>,
    seen_version: u64,
    tasks: Vec<Task<()>>,
    history_tasks: Vec<Task<()>>,
    _subscription: Subscription,
}

impl HomePage {
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let seen_version = store.read(cx).history_version;
        let mut page = Self {
            _subscription: cx.observe(&store, |page, store, cx| {
                let version = store.read(cx).history_version;

                if version != page.seen_version {
                    page.seen_version = version;
                    page.load_history(false, cx);
                }

                cx.notify();
            }),
            store,
            hero: Slot::default(),
            on_repeat: Slot::default(),
            recent: Slot::default(),
            playlists: Slot::default(),
            discover: Slot::default(),
            seen_version,
            tasks: Vec::new(),
            history_tasks: Vec::new(),
        };

        page.start(cx);

        page
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let env = self.store.read(cx).env();

        self.tasks = vec![
            data::load(
                &env,
                Some("home:on-repeat".to_owned()),
                data::fetch(|core| async move { core.library.top_tracks(SHELF_LEN as u32).await }),
                cx,
                |page: &mut Self, message, cx| {
                    if let Load::Ready { value, .. } = &message {
                        page.store
                            .read(cx)
                            .ensure_likes(value.iter().map(|track| track.uri.clone()).collect());
                    }

                    page.on_repeat.apply(message);
                },
            ),
            data::load(
                &env,
                Some("home:playlists".to_owned()),
                data::fetch(|core| async move { core.library.my_playlists(0).await }),
                cx,
                |page: &mut Self, message, _| page.playlists.apply(message),
            ),
            data::load(
                &env,
                Some("home:discover".to_owned()),
                data::fetch(|core| async move { core.library.discover().await }),
                cx,
                |page: &mut Self, message, cx| {
                    if let Load::Ready { value, .. } = &message {
                        let uris = value
                            .iter()
                            .filter_map(|shelf| match &shelf.items {
                                ShelfItems::Tracks(tracks) => Some(tracks),
                                ShelfItems::Albums(_) => None,
                            })
                            .flatten()
                            .map(|track| track.uri.clone())
                            .collect();

                        page.store.read(cx).ensure_likes(uris);
                    }

                    page.discover.apply(message);
                },
            ),
        ];
        self.load_history(true, cx);
    }

    /// The hero and the recently played shelf both come from the history.
    fn load_history(&mut self, use_cache: bool, cx: &mut Context<Self>) {
        let env = self.store.read(cx).env();

        self.history_tasks = vec![
            data::load(
                &env,
                use_cache.then(|| "home:hero".to_owned()),
                data::fetch(|core| async move { core.history.hero().await }),
                cx,
                |page: &mut Self, message, _| page.hero.apply(message),
            ),
            data::load(
                &env,
                None,
                data::fetch(|core| async move {
                    let rows = core.history.recent(RECENT_LOOKUPS as u32, 0).await;
                    let mut seen = Vec::new();
                    let mut tracks = Vec::new();

                    for row in rows {
                        if seen.contains(&row.track_uri) {
                            continue;
                        }

                        seen.push(row.track_uri.clone());

                        if let Some(track) = core.history.resolver().resolve(&row.track_uri).await {
                            tracks.push(track);
                        }

                        if tracks.len() >= SHELF_LEN {
                            break;
                        }
                    }

                    Ok::<_, LibraryError>(tracks)
                }),
                cx,
                |page: &mut Self, message, cx| {
                    if let Load::Ready { value, .. } = &message {
                        page.store
                            .read(cx)
                            .ensure_likes(value.iter().map(|track| track.uri.clone()).collect());
                    }

                    page.recent.apply(message);
                },
            ),
        ];
    }

    fn search_pill(&self, cx: &Context<Self>) -> Div {
        let store = self.store.clone();
        let state = self.store.read(cx);
        let switch = source_switch(&self.store, state.source(), state.switching_to());

        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .id("home-search")
                    .flex()
                    .items_center()
                    .gap_2()
                    .w(px(260.))
                    .h(px(38.))
                    .px_3()
                    .rounded(theme::CONTROL_RADIUS)
                    .bg(theme::active_nav())
                    .cursor_pointer()
                    .tab_index(0)
                    .border_1()
                    .border_color(theme::transparent())
                    .focus_visible(|style| style.border_color(theme::accent()))
                    .text_size(px(13.))
                    .text_color(theme::text_tertiary())
                    .child(icon(IconName::Search, 15., theme::text_tertiary()))
                    .child(t!("home.search_placeholder"))
                    .on_click(move |_, _, cx| {
                        store.update(cx, |state, cx| state.navigate(Route::Search, cx))
                    }),
            )
            .child(switch)
    }

    fn hero_card(&self, cx: &Context<Self>) -> AnyElement {
        let state = self.store.read(cx);

        let hero = match &self.hero.state {
            Load::Ready {
                value: Some(hero), ..
            } => hero.clone(),
            Load::Ready { value: None, .. } => {
                return hint_card(t!("home.nothing_yet"));
            }
            Load::Loading => return hero_skeleton(),
            Load::Failed(error) => {
                let this = cx.entity();

                return error_panel("hero-retry", error, move |_, _, cx| {
                    this.update(cx, |page, cx| page.load_history(false, cx));
                });
            }
        };
        let (label, route) = match hero.kind {
            HeroKind::Playlist => (t!("home.hero.playlist"), Route::Playlist(hero.id.clone())),
            HeroKind::Album => (t!("route.album"), Route::Album(hero.id.clone())),
            HeroKind::Artist => (t!("route.artist"), Route::Artist(hero.id.clone())),
            HeroKind::LikedSongs => (t!("home.hero.library"), Route::LikedSongs),
        };
        let label = label.to_uppercase();
        let model = &state.playback;
        let playing_here = model.is_playing()
            && model
                .context
                .as_ref()
                .is_some_and(|context| context.uri == hero.uri);
        let pending = state.pending_play && !playing_here;
        let count = hero
            .track_count
            .map(|count| plural(u64::from(count), "common.tracks_one", "common.tracks_other"));
        let store = self.store.clone();
        let open = self.store.clone();
        let target = route.clone();
        let uri = hero.uri.clone();
        let liked = hero.kind == HeroKind::LikedSongs;
        let play = {
            let mut button =
                round_play_button("hero-play", playing_here, 38., theme::accent(), true).relative();

            if pending {
                button = button.child(pending_overlay(theme::accent(), theme::on_accent()));
            }

            button.on_click(move |_, _, cx| {
                cx.stop_propagation();

                if playing_here {
                    store.read(cx).core.player.pause();
                } else if liked {
                    store.update(cx, |state, cx| state.navigate(target.clone(), cx));
                } else {
                    store.update(cx, |state, cx| state.play_context(&uri, cx));
                }
            })
        };

        div()
            .id("hero")
            .flex()
            .items_center()
            .gap_5()
            .w_full()
            .p_5()
            .mt_6()
            .relative()
            .rounded(theme::CARD_RADIUS)
            .bg(theme::raised())
            .cursor_pointer()
            .tab_index(0)
            .border_1()
            .border_color(theme::transparent())
            .focus_visible(|style| style.border_color(theme::accent()))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(theme::CARD_RADIUS)
                    .bg(theme::hero_tint()),
            )
            .child(art(&hero.art, &hero.uri, 128., theme::ART_RADIUS))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text_tertiary())
                            .child(label),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child(hero.name.clone()),
                    )
                    .children(count.map(|count| {
                        div()
                            .text_size(px(12.))
                            .text_color(theme::text_secondary())
                            .child(count)
                    }))
                    .child(div().pt_2().child(play)),
            )
            .on_click(move |_, _, cx| {
                open.update(cx, |state, cx| state.navigate(route.clone(), cx))
            })
            .into_any_element()
    }

    fn track_shelf(
        &self,
        id: &'static str,
        title: impl Into<SharedString>,
        slot: &Slot<Vec<Track>>,
    ) -> Option<Div> {
        match &slot.state {
            Load::Loading => Some(shelf(title, skeleton_tiles())),
            Load::Failed(_) => None,
            Load::Ready { value, .. } if value.is_empty() => None,
            Load::Ready { value, .. } => Some(shelf(title, self.track_tiles(id.into(), value))),
        }
    }

    /// The shelves the source suggests, as it named them.
    fn discover_shelves(&self) -> Vec<Div> {
        match &self.discover.state {
            Load::Loading => Vec::new(),
            Load::Failed(_) => Vec::new(),
            Load::Ready { value, .. } => value
                .iter()
                .filter_map(|source| match &source.items {
                    ShelfItems::Tracks(tracks) if !tracks.is_empty() => Some(shelf(
                        source.title.clone(),
                        self.track_tiles(SharedString::from(source.id.clone()), tracks),
                    )),
                    ShelfItems::Albums(albums) if !albums.is_empty() => {
                        let tiles = albums
                            .iter()
                            .take(SHELF_LEN)
                            .enumerate()
                            .map(|(index, album)| {
                                card(index, &CardItem::from(album), TILE, &self.store)
                            })
                            .collect();

                        Some(shelf(source.title.clone(), tiles))
                    }
                    _ => None,
                })
                .collect(),
        }
    }

    fn track_tiles(&self, id: SharedString, value: &[Track]) -> Vec<AnyElement> {
        let uris: Vec<String> = value.iter().map(|track| track.uri.clone()).collect();

        value
            .iter()
            .take(SHELF_LEN)
            .enumerate()
            .map(|(index, track)| {
                let store = self.store.clone();
                let uris = uris.clone();

                div()
                    .id(ElementId::from((id.clone(), index)))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_none()
                    .w(px(TILE))
                    .cursor_pointer()
                    .tab_index(0)
                    .rounded(theme::CARD_RADIUS)
                    .border_1()
                    .border_color(theme::transparent())
                    .focus_visible(|style| style.border_color(theme::accent()))
                    .tooltip(text_tooltip(track.name.clone()))
                    .child(art(&track.art, &track.uri, TILE - 2., theme::ART_RADIUS))
                    .child(crate::ui::common::card_text(
                        &track.name,
                        &track.artist_line(),
                    ))
                    .on_click(move |_, _, cx| {
                        store.update(cx, |state, cx| state.play_tracks(uris.clone(), index, cx));
                    })
                    .into_any_element()
            })
            .collect()
    }

    fn playlist_shelf(&self) -> Option<Div> {
        match &self.playlists.state {
            Load::Loading => Some(shelf(t!("home.your_playlists"), skeleton_tiles())),
            Load::Failed(_) => None,
            Load::Ready { value, .. } => {
                if value.items.is_empty() {
                    return None;
                }

                let tiles =
                    value
                        .items
                        .iter()
                        .take(SHELF_LEN)
                        .enumerate()
                        .map(|(index, playlist)| {
                            card(index, &CardItem::from(playlist), TILE, &self.store)
                        });

                Some(shelf(t!("home.your_playlists"), tiles.collect()))
            }
        }
    }
}

fn shelf(title: impl Into<SharedString>, tiles: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_6()
        .child(heading(title))
        .child(div().flex().gap(px(14.)).overflow_hidden().children(tiles))
}

fn skeleton_tiles() -> Vec<AnyElement> {
    (0..6)
        .map(|_| skeleton_card(TILE).into_any_element())
        .collect()
}

fn hero_skeleton() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_5()
        .w_full()
        .p_5()
        .mt_6()
        .rounded(theme::CARD_RADIUS)
        .bg(theme::raised())
        .child(
            div()
                .size(px(128.))
                .rounded(theme::CARD_RADIUS)
                .bg(theme::skeleton()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .h(px(10.))
                        .w(px(90.))
                        .rounded(px(4.))
                        .bg(theme::skeleton()),
                )
                .child(
                    div()
                        .h(px(26.))
                        .w(px(240.))
                        .rounded(px(6.))
                        .bg(theme::skeleton()),
                )
                .child(
                    div()
                        .h(px(10.))
                        .w(px(110.))
                        .rounded(px(4.))
                        .bg(theme::skeleton()),
                ),
        )
        .into_any_element()
}

fn hint_card(text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .p_5()
        .mt_6()
        .rounded(theme::CARD_RADIUS)
        .bg(theme::raised())
        .child(icon(IconName::Music, 22., theme::text_tertiary()))
        .child(
            div()
                .text_size(px(14.))
                .text_color(theme::text_secondary())
                .child(text.into()),
        )
        .into_any_element()
}

impl Render for HomePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hour = chrono::Timelike::hour(&chrono::Local::now());
        let hero = self.hero_card(cx);
        let on_repeat = self.track_shelf("on-repeat", t!("home.on_repeat"), &self.on_repeat);
        let recent = self.track_shelf("recent", t!("home.recent"), &self.recent);
        let playlists = self.playlist_shelf();
        let discover = self.discover_shelves();

        div()
            .id("home")
            .flex()
            .flex_col()
            .size_full()
            .overflow_y_scroll()
            .p_6()
            .child(self.search_pill(cx))
            .child(
                div()
                    .pt_6()
                    .text_size(px(36.))
                    .font_weight(FontWeight::BOLD)
                    .font_family(theme::FONT_DISPLAY)
                    .text_color(theme::text_primary())
                    .child(greeting(hour)),
            )
            .child(
                div()
                    .pt_1()
                    .text_size(px(14.))
                    .text_color(theme::text_secondary())
                    .child(t!("home.pick_up")),
            )
            .child(hero)
            .children(on_repeat)
            .children(recent)
            .children(discover)
            .children(playlists)
    }
}
