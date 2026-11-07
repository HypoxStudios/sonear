use std::rc::Rc;

use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, IntoElement, ParentElement as _, Pixels, Render, Size,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px, size,
};
use sonear_core::LibraryError;
use sonear_core::model::{Artist, Track};

use crate::data::{self, Env, Load, Paged, Slot};
use crate::store::AppStore;
use crate::theme;
use crate::ui::common::{
    CardItem, GRID_ROW, ROW_HEIGHT, TrackRowSpec, breadcrumb, card_row, error_panel, grid_columns,
    heading, open_spotify, skeleton_rows, track_row, with_retry_bar,
};
use crate::ui::widgets::{accent_button, art, label_caps, pending_overlay, play_pause_icon};

const HEADING_HEIGHT: f32 = 46.;

enum Row {
    Heading(&'static str),
    Track(usize),
    Cards(usize, usize),
    Loading,
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Self::Heading(_) => HEADING_HEIGHT,
            Self::Track(_) | Self::Loading => ROW_HEIGHT,
            Self::Cards(..) => GRID_ROW,
        }
    }
}

pub struct ArtistPage {
    store: Entity<AppStore>,
    id: String,
    artist: Slot<Artist>,
    /// Only filled when Spotify answered; the section stays hidden otherwise.
    top: Slot<Vec<Track>>,
    albums: Paged<CardItem>,
    scroll: VirtualListScrollHandle,
    tasks: Vec<Task<()>>,
    _subscription: Subscription,
}

impl ArtistPage {
    pub fn new(store: Entity<AppStore>, id: String, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            _subscription: cx.observe(&store, |_, _, cx| cx.notify()),
            store,
            id,
            artist: Slot::default(),
            top: Slot::default(),
            albums: Paged::default(),
            scroll: VirtualListScrollHandle::new(),
            tasks: Vec::new(),
        };

        page.start(cx);

        page
    }

    fn env(&self, cx: &Context<Self>) -> Env {
        self.store.read(cx).env()
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let env = self.env(cx);
        let (artist_id, albums_id, top_id) = (self.id.clone(), self.id.clone(), self.id.clone());

        self.tasks = vec![
            data::load(
                &env,
                Some(format!("artist:{}", self.id)),
                data::fetch(move |core| async move { core.library.artist(&artist_id).await }),
                cx,
                |page: &mut Self, message, _| page.artist.apply(message),
            ),
            data::load(
                &env,
                Some(format!("artist-albums:{}:0", self.id)),
                data::fetch(move |core| async move {
                    let page = core.library.artist_albums(&albums_id, 0).await?;

                    Ok(crate::ui::common::map_page(page, |album| {
                        CardItem::from(&album)
                    }))
                }),
                cx,
                |page: &mut Self, message, _| match message {
                    Load::Loading => {}
                    Load::Ready { value, .. } => page.albums.set_first(value),
                    Load::Failed(error) => page.albums.error = Some(error),
                },
            ),
            data::load(
                &env,
                None,
                data::fetch(
                    move |core| async move { core.library.artist_top_tracks(&top_id).await },
                ),
                cx,
                |page: &mut Self, message, cx| {
                    if let Load::Ready { value, .. } = &message {
                        page.store
                            .read(cx)
                            .ensure_likes(value.iter().map(|track| track.uri.clone()).collect());
                    }

                    page.top.apply(message);
                },
            ),
        ];
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.artist = Slot::default();
        self.top = Slot::default();
        self.albums = Paged::default();
        self.start(cx);
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some((offset, _)) = self.albums.begin_next() else {
            return;
        };
        let env = self.env(cx);
        let id = self.id.clone();

        data::load_page(
            &env,
            move |core| {
                Box::pin(async move {
                    let page = core.library.artist_albums(&id, offset).await?;

                    Ok(crate::ui::common::map_page(page, |album| {
                        CardItem::from(&album)
                    }))
                })
            },
            cx,
            |page: &mut Self, result, _| match result {
                Ok(next) => page.albums.append(next),
                Err(error) => page.albums.fail(error),
            },
        );
    }

    fn top_tracks(&self) -> &[Track] {
        match self.top.value() {
            Some(tracks) => tracks,
            None => &[],
        }
    }

    fn rows(&self, columns: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        let top = self.top_tracks().len();

        if top > 0 {
            rows.push(Row::Heading("Popular"));
            rows.extend((0..top).map(Row::Track));
        }

        let count = self.albums.items.len();

        if count > 0 {
            rows.push(Row::Heading("Albums"));

            for start in (0..count).step_by(columns) {
                rows.push(Row::Cards(start, (start + columns).min(count)));
            }
        }

        if self.albums.loading || !self.albums.first_loaded {
            rows.push(Row::Loading);
        }

        rows
    }

    fn header(&self, cx: &Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let Some(artist) = self.artist.value() else {
            return div()
                .flex()
                .items_center()
                .gap_5()
                .px_8()
                .py_4()
                .child(div().size(px(120.)).rounded_full().bg(theme::skeleton()))
                .into_any_element();
        };
        let uri = artist.uri.clone();
        let playing_here = state.playback.is_playing()
            && state
                .playback
                .context
                .as_ref()
                .is_some_and(|context| context.uri == uri);
        let pending = state.pending_play && !playing_here;
        let store = self.store.clone();
        let play = {
            let mut button = accent_button(
                "artist-play",
                if playing_here { "Pause" } else { "Play" },
                Some(play_pause_icon(playing_here, 14., theme::on_accent())),
                true,
            )
            .relative();

            if pending {
                button = button.child(pending_overlay(theme::accent(), theme::on_accent()));
            }

            button.on_click(move |_, _, cx| {
                if playing_here {
                    store.read(cx).core.player.pause();
                } else {
                    store.update(cx, |state, cx| state.play_context(&uri, cx));
                }
            })
        };

        div()
            .flex()
            .items_center()
            .gap_6()
            .flex_none()
            .px_8()
            .py_4()
            .child(art(&artist.art, &artist.id, 140., px(70.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w_0()
                    .child(label_caps("Artist"))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(40.))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child(artist.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(play)
                            .children(open_spotify("artist-open", artist.open_url.clone())),
                    ),
            )
            .into_any_element()
    }

    fn body(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error) = self
            .albums
            .error
            .clone()
            .filter(|_| self.albums.items.is_empty())
        {
            return self.error_view(&error, cx);
        }

        let width = f32::from(window.viewport_size().width);
        let columns = grid_columns(self.store.read(cx).main_width.get().min(width));
        let rows = Rc::new(self.rows(columns));

        if rows.is_empty() {
            return div()
                .px_8()
                .py_4()
                .text_size(px(13.))
                .text_color(theme::text_secondary())
                .child("No albums to show.")
                .into_any_element();
        }

        let sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
            rows.iter()
                .map(|row| size(px(100.), px(row.height())))
                .collect(),
        );

        let more_error = self
            .albums
            .error
            .clone()
            .filter(|_| !self.albums.items.is_empty());
        let this = cx.entity();
        let list = v_virtual_list(
            cx.entity(),
            "artist-rows",
            sizes,
            move |this, range, _, cx| {
                if this.albums.wants_more(range.end * columns) {
                    this.load_more(cx);
                }

                let store = this.store.clone();
                let state = store.read(cx);
                let likes = state.core.likes.clone();
                let current = state.playback.item.as_ref().map(|item| item.uri.clone());
                let uris: Vec<String> = this.top_tracks().iter().map(|t| t.uri.clone()).collect();

                range
                    .filter_map(|index| {
                        Some(match rows.get(index)? {
                            Row::Heading(title) => div()
                                .flex()
                                .items_end()
                                .px_6()
                                .pb_2()
                                .h(px(HEADING_HEIGHT))
                                .child(heading(*title))
                                .into_any_element(),
                            Row::Track(position) => {
                                let track = this.top_tracks().get(*position)?;
                                let spec = TrackRowSpec {
                                    index: *position,
                                    track,
                                    current: current.as_deref() == Some(track.uri.as_str()),
                                    liked: Some(likes.is_liked(&track.uri).unwrap_or(false)),
                                    show_art: true,
                                    show_album: false,
                                    show_number: true,
                                    trailing: None,
                                    queue_store: Some(&store),
                                };
                                let play_store = store.clone();
                                let like_store = store.clone();
                                let like_uri = track.uri.clone();
                                let uris = uris.clone();
                                let at = *position;

                                track_row(
                                    "artist-track",
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
                            Row::Cards(start, end) => {
                                card_row(index, &this.albums.items[*start..*end], &store)
                            }
                            Row::Loading => div().px_5().child(skeleton_rows(1)).into_any_element(),
                        })
                    })
                    .collect()
            },
        )
        .track_scroll(&self.scroll)
        .into_any_element();

        with_retry_bar(
            list,
            more_error.as_ref(),
            "artist-more-retry",
            move |_, _, cx| {
                this.update(cx, |page, cx| {
                    page.albums.retry();
                    cx.notify();
                });
            },
        )
    }

    fn error_view(&self, error: &LibraryError, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();

        error_panel("artist-retry", error, move |_, _, cx| {
            this.update(cx, |page, cx| page.retry(cx));
        })
    }
}

impl Render for ArtistPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let name = self
            .artist
            .value()
            .map_or_else(|| "\u{2026}".to_owned(), |artist| artist.name.clone());
        let crumb = breadcrumb(&self.store, state, &name);

        if let Some(error) = self.artist.error().cloned() {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(crumb)
                .child(self.error_view(&error, cx))
                .into_any_element();
        }

        let header = self.header(cx);
        let body = self.body(window, cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(crumb)
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
