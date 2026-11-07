use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    Pixels, Render, Size, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window,
    div, px, size,
};
use sonear_core::model::{Album, Artist, Playlist, SEARCH_KINDS, SearchKind, Show, Track};
use sonear_core::{LibraryError, model::Page};

use super::empty::empty_state;
use crate::data::{self, Env, Load, Paged};
use crate::store::AppStore;
use crate::theme;
use crate::ui::common::{
    CardItem, GRID_ROW, ROW_HEIGHT, TrackRowSpec, card_row, error_panel, grid_columns, heading,
    skeleton_rows, track_row,
};
use crate::ui::widgets::{accent_button, icon};

const DEBOUNCE: Duration = Duration::from_millis(300);
const HEADER_HEIGHT: f32 = 46.;
const MORE_HEIGHT: f32 = 52.;

enum Row {
    Heading(&'static str),
    Track(usize),
    Cards(SearchKind, usize, usize),
    More(SearchKind),
    Loading,
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Self::Heading(_) => HEADER_HEIGHT,
            Self::Track(_) | Self::Loading => ROW_HEIGHT,
            Self::Cards(..) => GRID_ROW,
            Self::More(_) => MORE_HEIGHT,
        }
    }
}

#[derive(Default)]
struct Results {
    tracks: Paged<Track>,
    artists: Paged<Artist>,
    albums: Paged<Album>,
    playlists: Paged<Playlist>,
    shows: Paged<Show>,
}

pub struct SearchPage {
    store: Entity<AppStore>,
    input: Entity<InputState>,
    query: String,
    searched: String,
    generation: u64,
    loading: bool,
    error: Option<LibraryError>,
    results: Results,
    scroll: VirtualListScrollHandle,
    debounce: Option<Task<()>>,
    tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn kind_name(kind: SearchKind) -> &'static str {
    match kind {
        SearchKind::Track => "Songs",
        SearchKind::Artist => "Artists",
        SearchKind::Album => "Albums",
        SearchKind::Playlist => "Playlists",
        SearchKind::Show => "Podcasts",
    }
}

impl SearchPage {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("What do you want to listen to?"));
        let handle = input.read(cx).focus_handle(cx);

        window.focus(&handle, cx);

        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&input, |page, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();

                    page.on_query(text, cx);
                }
            }),
        ];

        #[allow(unused_mut)]
        let mut page = Self {
            store,
            input,
            query: String::new(),
            searched: String::new(),
            generation: 0,
            loading: false,
            error: None,
            results: Results::default(),
            scroll: VirtualListScrollHandle::new(),
            debounce: None,
            tasks: Vec::new(),
            _subscriptions: subscriptions,
        };

        #[cfg(feature = "demo")]
        if let Ok(query) = std::env::var("SONEAR_DEMO_QUERY") {
            let input = page.input.clone();

            input.update(cx, |input, cx| input.set_value(query.clone(), window, cx));
            page.on_query(query, cx);
        }

        page
    }

    fn env(&self, cx: &Context<Self>) -> Env {
        self.store.read(cx).env()
    }

    fn on_query(&mut self, text: String, cx: &mut Context<Self>) {
        let text = text.trim().to_owned();

        if text == self.query {
            return;
        }

        self.query = text.clone();
        self.generation += 1;

        let generation = self.generation;

        if text.is_empty() {
            self.debounce = None;
            self.searched.clear();
            self.loading = false;
            self.error = None;
            self.results = Results::default();
            cx.notify();

            return;
        }

        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;

            let _ = this.update(cx, |page, cx| {
                if page.generation == generation {
                    page.run(cx);
                }
            });
        }));
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        let env = self.env(cx);
        let query = self.query.clone();
        let generation = self.generation;

        self.searched.clone_from(&query);
        self.loading = true;
        self.error = None;
        self.results = Results::default();
        self.scroll = VirtualListScrollHandle::new();
        self.tasks = vec![data::load(
            &env,
            None,
            data::fetch(
                move |core| async move { core.library.search(&query, &SEARCH_KINDS, 0).await },
            ),
            cx,
            move |page: &mut Self, message, cx| {
                if page.generation != generation {
                    return;
                }

                match message {
                    Load::Loading => {}
                    Load::Ready { value, .. } => {
                        page.loading = false;
                        page.store.read(cx).ensure_likes(
                            value.tracks.items.iter().map(|t| t.uri.clone()).collect(),
                        );
                        page.results.tracks.set_first(value.tracks);
                        page.results.artists.set_first(value.artists);
                        page.results.albums.set_first(value.albums);
                        page.results.playlists.set_first(value.playlists);
                        page.results.shows.set_first(value.shows);
                    }
                    Load::Failed(error) => {
                        page.loading = false;
                        page.error = Some(error);
                    }
                }
            },
        )];
    }

    fn show_more(&mut self, kind: SearchKind, cx: &mut Context<Self>) {
        let begun = match kind {
            SearchKind::Track => self.results.tracks.begin_next(),
            SearchKind::Artist => self.results.artists.begin_next(),
            SearchKind::Album => self.results.albums.begin_next(),
            SearchKind::Playlist => self.results.playlists.begin_next(),
            SearchKind::Show => self.results.shows.begin_next(),
        };
        let Some((offset, _)) = begun else {
            return;
        };
        let env = self.env(cx);
        let query = self.searched.clone();
        let generation = self.generation;

        data::load_page(
            &env,
            move |core| Box::pin(async move { core.library.search(&query, &[kind], offset).await }),
            cx,
            move |page: &mut Self, result, cx| {
                if page.generation != generation {
                    return;
                }

                page.append(kind, result, cx);
            },
        );
    }

    fn append(
        &mut self,
        kind: SearchKind,
        result: Result<sonear_core::model::SearchResults, LibraryError>,
        cx: &mut Context<Self>,
    ) {
        fn take<T>(list: &mut Paged<T>, page: Page<T>, error: Option<LibraryError>) {
            match error {
                Some(error) => list.fail(error),
                None => list.append(page),
            }
        }

        match result {
            Ok(found) => match kind {
                SearchKind::Track => {
                    self.store
                        .read(cx)
                        .ensure_likes(found.tracks.items.iter().map(|t| t.uri.clone()).collect());
                    take(&mut self.results.tracks, found.tracks, None);
                }
                SearchKind::Artist => take(&mut self.results.artists, found.artists, None),
                SearchKind::Album => take(&mut self.results.albums, found.albums, None),
                SearchKind::Playlist => take(&mut self.results.playlists, found.playlists, None),
                SearchKind::Show => take(&mut self.results.shows, found.shows, None),
            },
            Err(error) => match kind {
                SearchKind::Track => self.results.tracks.fail(error),
                SearchKind::Artist => self.results.artists.fail(error),
                SearchKind::Album => self.results.albums.fail(error),
                SearchKind::Playlist => self.results.playlists.fail(error),
                SearchKind::Show => self.results.shows.fail(error),
            },
        }
    }

    fn card_items(&self, kind: SearchKind) -> Vec<CardItem> {
        match kind {
            SearchKind::Artist => self
                .results
                .artists
                .items
                .iter()
                .map(CardItem::from)
                .collect(),
            SearchKind::Album => self
                .results
                .albums
                .items
                .iter()
                .map(CardItem::from)
                .collect(),
            SearchKind::Playlist => self
                .results
                .playlists
                .items
                .iter()
                .map(CardItem::from)
                .collect(),
            SearchKind::Show => self
                .results
                .shows
                .items
                .iter()
                .map(CardItem::from)
                .collect(),
            SearchKind::Track => Vec::new(),
        }
    }

    fn rows(&self, columns: usize) -> Vec<Row> {
        let mut rows = Vec::new();

        for kind in SEARCH_KINDS {
            let (count, has_more, loading) = match kind {
                SearchKind::Track => (
                    self.results.tracks.items.len(),
                    self.results.tracks.has_more,
                    self.results.tracks.loading,
                ),
                SearchKind::Artist => (
                    self.results.artists.items.len(),
                    self.results.artists.has_more,
                    self.results.artists.loading,
                ),
                SearchKind::Album => (
                    self.results.albums.items.len(),
                    self.results.albums.has_more,
                    self.results.albums.loading,
                ),
                SearchKind::Playlist => (
                    self.results.playlists.items.len(),
                    self.results.playlists.has_more,
                    self.results.playlists.loading,
                ),
                SearchKind::Show => (
                    self.results.shows.items.len(),
                    self.results.shows.has_more,
                    self.results.shows.loading,
                ),
            };

            if count == 0 {
                continue;
            }

            rows.push(Row::Heading(kind_name(kind)));

            if kind == SearchKind::Track {
                rows.extend((0..count).map(Row::Track));
            } else {
                for start in (0..count).step_by(columns) {
                    rows.push(Row::Cards(kind, start, (start + columns).min(count)));
                }
            }

            if loading {
                rows.push(Row::Loading);
            } else if has_more {
                rows.push(Row::More(kind));
            }
        }

        rows
    }

    fn nothing_found(&self) -> bool {
        self.results.tracks.items.is_empty()
            && self.results.artists.items.is_empty()
            && self.results.albums.items.is_empty()
            && self.results.playlists.items.is_empty()
            && self.results.shows.items.is_empty()
    }

    fn body(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.query.is_empty() {
            return empty_state(
                IconName::Search,
                "Search Spotify",
                "Find songs, artists, albums, playlists and podcasts. Results appear as you type.",
            );
        }

        if let Some(error) = &self.error {
            let this = cx.entity();

            return error_panel("search-retry", error, move |_, _, cx| {
                this.update(cx, |page, cx| page.run(cx));
            });
        }

        if self.loading || self.searched != self.query {
            return div().px_5().child(skeleton_rows(9)).into_any_element();
        }

        if self.nothing_found() {
            return empty_state(
                IconName::Search,
                format!("No results for \u{201c}{}\u{201d}", self.query),
                "Check the spelling, or try a shorter search or the name of an artist.",
            );
        }

        let width = f32::from(window.viewport_size().width);
        let columns = grid_columns(self.store.read(cx).main_width.get().min(width));
        let rows = Rc::new(self.rows(columns));
        let sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
            rows.iter()
                .map(|row| size(px(100.), px(row.height())))
                .collect(),
        );
        let items = rows.clone();

        v_virtual_list(
            cx.entity(),
            "search-results",
            sizes,
            move |this, range, _, cx| {
                let store = this.store.clone();
                let state = store.read(cx);
                let likes = state.core.likes.clone();
                let current = state.playback.item.as_ref().map(|item| item.uri.clone());
                let track_uris: Vec<String> = this
                    .results
                    .tracks
                    .items
                    .iter()
                    .map(|track| track.uri.clone())
                    .collect();

                range
                    .filter_map(|index| {
                        let row = items.get(index)?;

                        Some(match row {
                            Row::Heading(title) => div()
                                .flex()
                                .items_end()
                                .px_6()
                                .pb_2()
                                .h(px(HEADER_HEIGHT))
                                .child(heading(*title))
                                .into_any_element(),
                            Row::Track(position) => {
                                let track = this.results.tracks.items.get(*position)?;
                                let spec = TrackRowSpec {
                                    index: *position,
                                    track,
                                    current: current.as_deref() == Some(track.uri.as_str()),
                                    liked: Some(likes.is_liked(&track.uri).unwrap_or(false)),
                                    show_art: true,
                                    show_album: true,
                                    show_number: false,
                                    trailing: None,
                                };
                                let play_store = store.clone();
                                let like_store = store.clone();
                                let uris = track_uris.clone();
                                let like_uri = track.uri.clone();
                                let at = *position;

                                track_row(
                                    "search-track",
                                    &spec,
                                    move |_, _, cx| {
                                        play_store.update(cx, |state, cx| {
                                            state.play_tracks(uris.clone(), at, cx);
                                        });
                                    },
                                    move |_, _, cx| {
                                        like_store.update(cx, |state, cx| {
                                            state.toggle_like(&like_uri, cx)
                                        });
                                    },
                                )
                            }
                            Row::Cards(kind, start, end) => {
                                let cards = this.card_items(*kind);

                                card_row(index, &cards[*start..*end], &store)
                            }
                            Row::More(kind) => {
                                let kind = *kind;
                                let page = cx.entity();

                                div()
                                    .flex()
                                    .items_center()
                                    .px_6()
                                    .h(px(MORE_HEIGHT))
                                    .child(
                                        accent_button(
                                            gpui_kit::ElementId::from((
                                                "search-more",
                                                kind as usize,
                                            )),
                                            "Show more",
                                            None,
                                            true,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                page.update(cx, |page, cx| {
                                                    page.show_more(kind, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .into_any_element()
                            }
                            Row::Loading => div().px_5().child(skeleton_rows(1)).into_any_element(),
                        })
                    })
                    .collect()
            },
        )
        .track_scroll(&self.scroll)
        .into_any_element()
    }
}

impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(window, cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_none()
                    .px_6()
                    .pt_6()
                    .pb_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .w_full()
                            .max_w(px(520.))
                            .px_3()
                            .rounded(theme::CONTROL_RADIUS)
                            .bg(theme::active_nav())
                            .child(icon(IconName::Search, 16., theme::text_tertiary()))
                            .child(
                                div().flex_1().child(
                                    Input::new(&self.input)
                                        .appearance(false)
                                        .bordered(false)
                                        .cleanable(true),
                                ),
                            ),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}
