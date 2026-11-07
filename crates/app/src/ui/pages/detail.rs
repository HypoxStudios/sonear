//! Album, playlist and Liked Songs share one layout: breadcrumb, large art,
//! title, Play and heart, then the track list. The header shrinks on scroll.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, Entity, FontWeight, IntoElement, ParentElement as _, Pixels, Render,
    SharedString, Size, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window,
    div, px, size,
};
use serde::{Deserialize, Serialize};
use sonear_core::LibraryError;
use sonear_core::model::{Album, Artwork, PlayOffset, Playlist, Track, is_liked_context};
use sonear_core::seen::seen_key;

use crate::data::{self, Env, Load, Paged, Slot};
use crate::format::{long_duration, plain_text};
use crate::store::AppStore;
use crate::theme;
use crate::ui::common::{
    ROW_HEIGHT, TrackRowSpec, breadcrumb, error_panel, open_spotify, skeleton_rows, track_row,
    with_retry_bar,
};
use crate::ui::widgets::{
    accent_button, art, heart_button, icon, label_caps, pending_overlay, play_pause_icon,
};

const ART_FULL: f32 = 232.;
const ART_COMPACT: f32 = 64.;
const COLLAPSE_OVER: f32 = 190.;
pub const NOT_EXPOSED: &str = "Spotify doesn't let apps list the tracks in playlists you don't own. Press Play, and what's coming up appears under Up next.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Album(String),
    Playlist(String),
    Liked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub title: String,
    pub byline: String,
    pub description: String,
    pub art: Artwork,
    /// The URI Play starts. `None` for Liked Songs, which has no context.
    pub context_uri: Option<String>,
    pub open_url: Option<String>,
    pub track_count: Option<u32>,
    pub is_playlist: bool,
    pub owner_id: Option<String>,
    pub collaborative: bool,
}

impl Header {
    fn album(album: &Album) -> Self {
        Self {
            title: album.name.clone(),
            byline: [album.year().unwrap_or_default(), &album.artist_line()]
                .iter()
                .filter(|part| !part.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" \u{00b7} "),
            description: String::new(),
            art: album.art.clone(),
            context_uri: Some(album.uri.clone()),
            open_url: album.open_url.clone(),
            track_count: Some(album.total_tracks),
            is_playlist: false,
            owner_id: None,
            collaborative: false,
        }
    }

    fn playlist(playlist: &Playlist) -> Self {
        Self {
            title: playlist.name.clone(),
            byline: playlist
                .owner_name
                .clone()
                .map(|owner| format!("By {owner}"))
                .unwrap_or_default(),
            description: plain_text(&playlist.description),
            art: playlist.art.clone(),
            context_uri: Some(playlist.uri.clone()),
            open_url: playlist.open_url.clone(),
            track_count: playlist.track_count,
            is_playlist: true,
            owner_id: playlist.owner_id.clone(),
            collaborative: playlist.collaborative,
        }
    }

    fn liked() -> Self {
        Self {
            title: "Liked Songs".to_owned(),
            byline: String::new(),
            description: "Everything you've saved.".to_owned(),
            art: Artwork::default(),
            context_uri: None,
            open_url: None,
            track_count: None,
            is_playlist: false,
            owner_id: None,
            collaborative: false,
        }
    }

    /// Hearts are hidden on playlists the user owns. Until the profile is
    /// known, playlists hide them too rather than flicker.
    fn hides_hearts(&self, user: Option<&str>) -> bool {
        self.is_playlist
            && match user {
                Some(user) => self.collaborative || self.owner_id.as_deref() == Some(user),
                None => true,
            }
    }
}

pub struct DetailPage {
    store: Entity<AppStore>,
    kind: Kind,
    header: Slot<Header>,
    tracks: Paged<Track>,
    not_exposed: bool,
    scroll: VirtualListScrollHandle,
    sizes: Rc<Vec<Size<Pixels>>>,
    tasks: Vec<Task<()>>,
    seen_task: Option<Task<()>>,
    /// The store's `seen_version` this page has caught up with.
    seen_version: u64,
    _subscription: Subscription,
}

fn kind_key(kind: &Kind) -> String {
    match kind {
        Kind::Album(id) => format!("album:{id}"),
        Kind::Playlist(id) => format!("playlist:{id}"),
        Kind::Liked => "liked".to_owned(),
    }
}

async fn fetch_tracks(
    core: &sonear_core::Core,
    kind: &Kind,
    offset: u32,
) -> Result<sonear_core::model::Page<Track>, LibraryError> {
    match kind {
        Kind::Album(id) => core.library.album_tracks(id, offset).await,
        Kind::Playlist(id) => core.library.playlist_tracks(id, offset).await,
        Kind::Liked => core.library.saved_tracks(offset).await,
    }
}

impl DetailPage {
    pub fn new(store: Entity<AppStore>, kind: Kind, cx: &mut Context<Self>) -> Self {
        let seen_version = store.read(cx).seen_version;
        let mut page = Self {
            _subscription: cx.observe(&store, |page, _, cx| {
                page.on_store(cx);
                cx.notify();
            }),
            store,
            kind,
            header: Slot::default(),
            tracks: Paged::default(),
            not_exposed: false,
            scroll: VirtualListScrollHandle::new(),
            sizes: Rc::new(Vec::new()),
            tasks: Vec::new(),
            seen_task: None,
            seen_version,
        };

        page.start(cx);

        page
    }

    fn env(&self, cx: &Context<Self>) -> Env {
        self.store.read(cx).env()
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let env = self.env(cx);
        let key = kind_key(&self.kind);
        let mut tasks = Vec::new();

        match self.kind.clone() {
            Kind::Liked => self.header.apply(Load::Ready {
                value: Header::liked(),
            }),
            Kind::Album(id) => tasks.push(data::load(
                &env,
                Some(format!("header:{key}")),
                data::fetch(move |core| async move {
                    core.library
                        .album(&id)
                        .await
                        .map(|album| Header::album(&album))
                }),
                cx,
                |page: &mut Self, message, cx| page.on_header(message, cx),
            )),
            Kind::Playlist(id) => tasks.push(data::load(
                &env,
                Some(format!("header:{key}")),
                data::fetch(move |core| async move {
                    core.library
                        .playlist(&id)
                        .await
                        .map(|playlist| Header::playlist(&playlist))
                }),
                cx,
                |page: &mut Self, message, cx| page.on_header(message, cx),
            )),
        }

        let kind = self.kind.clone();
        let cache_key = (self.kind != Kind::Liked).then(|| format!("tracks:{key}:0"));

        tasks.push(data::load(
            &env,
            cache_key,
            data::fetch(move |core| async move { fetch_tracks(&core, &kind, 0).await }),
            cx,
            |page: &mut Self, message, cx| page.on_tracks(message, cx),
        ));
        self.tasks = tasks;
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let retained = if self.not_exposed {
            std::mem::take(&mut self.tracks.items)
        } else {
            Vec::new()
        };

        self.header = Slot::default();
        self.tracks = Paged::default();
        self.tracks.items = retained;
        self.not_exposed = false;
        self.seen_task = None;
        self.start(cx);
    }

    fn on_header(&mut self, message: Load<Header>, cx: &mut Context<Self>) {
        if let (Load::Ready { value, .. }, Kind::Album(_)) = (&message, &self.kind)
            && let Some(uri) = &value.context_uri
        {
            self.store.read(cx).ensure_likes(vec![uri.clone()]);
        }

        self.header.apply(message);
    }

    fn on_tracks(
        &mut self,
        message: Load<sonear_core::model::Page<Track>>,
        cx: &mut Context<Self>,
    ) {
        match message {
            Load::Loading => {}
            Load::Ready { value, .. } => {
                self.track_hearts(&value.items, cx);
                self.tracks.set_first(value);
            }
            Load::Failed(error) => self.on_track_error(error, cx),
        }
    }

    fn on_track_error(&mut self, error: LibraryError, cx: &mut Context<Self>) {
        let withheld = matches!(self.kind, Kind::Playlist(_))
            && matches!(error, LibraryError::Forbidden | LibraryError::NotFound);

        if withheld {
            let first_time = !self.not_exposed;

            self.not_exposed = true;
            self.tracks.loading = false;
            self.tracks.first_loaded = true;

            if first_time {
                self.start_seen(cx);
            }
        } else if self.tracks.first_loaded {
            self.tracks.fail(error);
        } else {
            self.tracks.error = Some(error);
        }
    }

    fn start_seen(&mut self, cx: &mut Context<Self>) {
        let Kind::Playlist(id) = &self.kind else {
            return;
        };
        let env = self.env(cx);

        self.seen_task = Some(data::load(
            &env,
            Some(seen_key(id)),
            data::fetch(|_core| async { Err::<Vec<Track>, _>(LibraryError::NotFound) }),
            cx,
            |page: &mut Self, message, cx| page.on_seen(message, cx),
        ));
    }

    fn on_seen(&mut self, message: Load<Vec<Track>>, cx: &mut Context<Self>) {
        if !self.not_exposed {
            return;
        }

        let Load::Ready { value } = message else {
            return;
        };
        let fresh: Vec<Track> = value
            .iter()
            .filter(|track| !self.tracks.items.iter().any(|shown| shown.uri == track.uri))
            .cloned()
            .collect();

        self.track_hearts(&fresh, cx);
        self.tracks.items = value;
    }

    /// Reloads the collected tracks when the collector saved new ones for this playlist.
    fn on_store(&mut self, cx: &mut Context<Self>) {
        let state = self.store.read(cx);

        if state.seen_version == self.seen_version {
            return;
        }

        self.seen_version = state.seen_version;

        let Kind::Playlist(id) = &self.kind else {
            return;
        };

        if self.not_exposed && state.seen_changed.as_deref() == Some(id.as_str()) {
            self.start_seen(cx);
        }
    }

    fn track_hearts(&self, tracks: &[Track], cx: &Context<Self>) {
        let uris: Vec<String> = tracks.iter().map(|track| track.uri.clone()).collect();
        let store = self.store.read(cx);

        if self.kind == Kind::Liked {
            store.core.likes.mark(&uris, true);
        } else {
            store.ensure_likes(uris);
        }
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some((offset, _)) = self.tracks.begin_next() else {
            return;
        };
        let env = self.env(cx);
        let kind = self.kind.clone();

        data::load_page(
            &env,
            move |core| Box::pin(async move { fetch_tracks(&core, &kind, offset).await }),
            cx,
            |page: &mut Self, result, cx| match result {
                Ok(next) => {
                    page.track_hearts(&next.items, cx);
                    page.tracks.append(next);
                }
                Err(error) => page.tracks.fail(error),
            },
        );
    }

    fn sizes_for(&mut self, count: usize) -> Rc<Vec<Size<Pixels>>> {
        if self.sizes.len() != count {
            self.sizes = Rc::new(vec![size(px(100.), px(ROW_HEIGHT)); count]);
        }

        self.sizes.clone()
    }

    fn uris(&self) -> Vec<String> {
        self.tracks
            .items
            .iter()
            .map(|track| track.uri.clone())
            .collect()
    }

    /// What Play starts: the context, or the loaded tracks when there is none.
    fn play_target(&self) -> (Option<String>, Vec<String>) {
        (
            self.header
                .value()
                .and_then(|header| header.context_uri.clone()),
            self.uris(),
        )
    }

    fn header_view(&self, cx: &Context<Self>) -> Div {
        let state = self.store.read(cx);
        let model = &state.playback;
        let header = self.header.value();
        let context = header.and_then(|header| header.context_uri.clone());
        let liked = self.kind == Kind::Liked;
        let in_liked_context = liked
            && model
                .context
                .as_ref()
                .is_some_and(|context| is_liked_context(&context.uri));
        let playing_here = model.is_playing()
            && match &context {
                Some(uri) => model.context.as_ref().is_some_and(|c| &c.uri == uri),
                None => liked && (in_liked_context || self.is_current_in_list(model)),
            };
        let paused_in_list = context.is_none()
            && !model.is_playing()
            && (in_liked_context || self.is_current_in_list(model));
        let pending = state.pending_play;
        let scrolled = -f32::from(self.scroll.offset().y);
        let t = (scrolled / COLLAPSE_OVER).clamp(0., 1.);
        let art_size = ART_FULL + (ART_COMPACT - ART_FULL) * t;
        let title_size = 44. + (24. - 44.) * t;
        let expanded = t < 0.45;
        let title = header.map_or(String::new(), |header| header.title.clone());
        let label = match self.kind {
            Kind::Album(_) => "Album",
            Kind::Playlist(_) => "Playlist",
            Kind::Liked => "Playlist",
        };
        let count_text = self.count_text(header);
        let store = self.store.clone();
        let this = cx.entity();
        let play = {
            let mut button = accent_button(
                "detail-play",
                if playing_here { "Pause" } else { "Play" },
                Some(play_pause_icon(playing_here, 14., theme::on_accent())),
                header.is_some(),
            )
            .relative();

            if pending && !playing_here {
                button = button.child(pending_overlay(theme::accent(), theme::on_accent()));
            }

            button.on_click(move |_, _, cx| {
                if playing_here {
                    store.read(cx).core.player.pause();
                } else if paused_in_list {
                    store.read(cx).core.player.play(None);
                } else {
                    let (context, uris) = this.read(cx).play_target();

                    store.update(cx, |state, cx| match context {
                        Some(uri) => state.play_context(&uri, cx),
                        None if liked => state.play_liked(None, uris, 0, cx),
                        None => state.play_tracks(uris, 0, cx),
                    });
                }
            })
        };
        let heart = match (&self.kind, &context) {
            (Kind::Album(_), Some(uri)) => {
                let liked = state.core.likes.is_liked(uri).unwrap_or(false);
                let uri = uri.clone();
                let store = self.store.clone();

                Some(
                    heart_button(
                        "detail-like",
                        liked,
                        16.,
                        36.,
                        theme::text_secondary(),
                        true,
                    )
                    .border_color(theme::hairline())
                    .on_click(move |_, _, cx| {
                        store.update(cx, |state, cx| state.toggle_like(&uri, cx));
                    }),
                )
            }
            _ => None,
        };

        div()
            .flex()
            .items_center()
            .gap_6()
            .flex_none()
            .px_8()
            .py_4()
            .child(match header {
                Some(header) => art(&header.art, &title, art_size, theme::ART_RADIUS),
                None => div()
                    .size(px(art_size))
                    .rounded(theme::ART_RADIUS)
                    .bg(theme::skeleton())
                    .into_any_element(),
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w_0()
                    .when(expanded, |this| this.child(label_caps(label)))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(title_size))
                            .font_weight(FontWeight::BOLD)
                            .font_family(theme::FONT_DISPLAY)
                            .text_color(theme::text_primary())
                            .child(title.clone()),
                    )
                    .when_some(
                        header.filter(|_| expanded).map(|h| h.byline.clone()),
                        |this, byline| {
                            this.when(!byline.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(theme::text_secondary())
                                        .child(byline),
                                )
                            })
                        },
                    )
                    .when_some(
                        header
                            .filter(|_| expanded)
                            .map(|h| h.description.clone())
                            .filter(|text| !text.is_empty()),
                        |this, description| {
                            this.child(
                                div()
                                    .max_w(px(420.))
                                    .line_clamp(2)
                                    .text_size(px(13.))
                                    .text_color(theme::text_secondary())
                                    .child(description),
                            )
                        },
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(play)
                            .children(heart)
                            .children(
                                open_spotify(
                                    "detail-open",
                                    header.and_then(|header| header.open_url.clone()),
                                )
                                .filter(|_| expanded),
                            ),
                    )
                    .when(expanded && !count_text.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::text_tertiary())
                                .child(count_text),
                        )
                    }),
            )
    }

    fn is_current_in_list(&self, model: &sonear_core::PlaybackModel) -> bool {
        model
            .item
            .as_ref()
            .is_some_and(|item| self.tracks.items.iter().any(|track| track.uri == item.uri))
    }

    fn count_text(&self, header: Option<&Header>) -> String {
        let count = self
            .tracks
            .total
            .or_else(|| header.and_then(|header| header.track_count));
        let Some(count) = count else {
            return String::new();
        };
        let noun = if count == 1 { "track" } else { "tracks" };
        let all_loaded = self.tracks.first_loaded && !self.tracks.has_more && !self.not_exposed;

        if all_loaded {
            let total: u64 = self
                .tracks
                .items
                .iter()
                .map(|track| track.duration_ms)
                .sum();

            format!("{count} {noun}  \u{00b7}  {}", long_duration(total))
        } else {
            format!("{count} {noun}")
        }
    }

    fn seen_notice(&self, header: Option<&Header>) -> String {
        let seen = self.tracks.items.len();
        let total = self
            .tracks
            .total
            .or_else(|| header.and_then(|header| header.track_count));
        let shown = match total {
            Some(total) => format!("the {seen} of {total} tracks"),
            None => format!("the {seen} tracks"),
        };

        format!(
            "Spotify doesn't let apps list the tracks in playlists you don't own, so this shows {shown} seen while playing. More appear as it plays."
        )
    }

    fn body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.not_exposed && self.tracks.items.is_empty() {
            return notice_panel(NOT_EXPOSED);
        }

        if let Some(error) = &self.tracks.error
            && self.tracks.items.is_empty()
        {
            let this = cx.entity();

            return error_panel("detail-retry", error, move |_, _, cx| {
                this.update(cx, |page, cx| page.retry(cx));
            });
        }

        if !self.tracks.first_loaded {
            return div().px_5().child(skeleton_rows(9)).into_any_element();
        }

        if self.tracks.items.is_empty() {
            return div()
                .p_8()
                .text_size(px(13.))
                .text_color(theme::text_secondary())
                .child("Nothing here yet.")
                .into_any_element();
        }

        let count = self.tracks.items.len();
        let sizes = self.sizes_for(count);
        let not_exposed = self.not_exposed;
        let notice = not_exposed.then(|| self.seen_notice(self.header.value()));
        let kind = self.kind.clone();
        let is_liked = kind == Kind::Liked;
        let user = self.store.read(cx).user_id().map(str::to_owned);
        let editable = self
            .header
            .value()
            .is_some_and(|header| header.hides_hearts(user.as_deref()));
        let context = self
            .header
            .value()
            .and_then(|header| header.context_uri.clone());
        let current = self
            .store
            .read(cx)
            .playback
            .item
            .as_ref()
            .map(|item| item.uri.clone());

        let more_error = self.tracks.error.clone();
        let this = cx.entity();
        let list = v_virtual_list(
            cx.entity(),
            "detail-tracks",
            sizes,
            move |this, range, _, cx| {
                if this.tracks.wants_more(range.end) {
                    this.load_more(cx);
                }

                let state = this.store.read(cx);
                let likes = state.core.likes.clone();
                let store = this.store.clone();
                let page = cx.entity();
                let show_art = !matches!(kind, Kind::Album(_));
                let show_album = !matches!(kind, Kind::Album(_));

                range
                    .filter_map(|index| {
                        let track = this.tracks.items.get(index)?;
                        let liked =
                            (!editable).then(|| likes.is_liked(&track.uri).unwrap_or(false));
                        let spec = TrackRowSpec {
                            index,
                            track,
                            current: current.as_deref() == Some(track.uri.as_str()),
                            liked,
                            show_art,
                            show_album,
                            show_number: !not_exposed,
                            trailing: None,
                            queue_store: Some(&store),
                        };
                        let play_store = store.clone();
                        let like_store = store.clone();
                        let play_page = page.clone();
                        let play_context = context.clone();
                        let track_uri = track.uri.clone();
                        let like_uri = track.uri.clone();
                        let is_album = matches!(kind, Kind::Album(_));

                        Some(track_row(
                            "detail-row",
                            &spec,
                            move |_, _, cx| {
                                let uris = play_page.read(cx).uris();

                                play_store.update(cx, |state, cx| match &play_context {
                                    Some(context) if is_album => state.play_in_context(
                                        context,
                                        PlayOffset::Position(index as u32),
                                        cx,
                                    ),
                                    Some(context) => state.play_in_context(
                                        context,
                                        PlayOffset::Uri(track_uri.clone()),
                                        cx,
                                    ),
                                    None if is_liked => {
                                        state.play_liked(Some(track_uri.clone()), uris, index, cx)
                                    }
                                    None => state.play_tracks(uris, index, cx),
                                });
                            },
                            move |_, _, cx| {
                                like_store.update(cx, |state, cx| state.toggle_like(&like_uri, cx));
                            },
                        ))
                    })
                    .collect()
            },
        )
        .track_scroll(&self.scroll)
        .into_any_element();

        let list = with_retry_bar(
            list,
            more_error.as_ref(),
            "detail-more-retry",
            move |_, _, cx| {
                this.update(cx, |page, cx| {
                    page.tracks.retry();
                    cx.notify();
                });
            },
        );

        match notice {
            Some(notice) => div()
                .flex()
                .flex_col()
                .size_full()
                .child(div().flex_none().child(notice_panel(notice)))
                .child(div().flex_1().min_h_0().child(list))
                .into_any_element(),
            None => list,
        }
    }
}

fn notice_panel(text: impl Into<SharedString>) -> AnyElement {
    div()
        .px_8()
        .py_4()
        .child(
            div()
                .flex()
                .items_start()
                .gap_3()
                .w_full()
                .max_w(px(560.))
                .p_4()
                .rounded(theme::CARD_RADIUS)
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::raised())
                .child(icon(IconName::ListMusic, 18., theme::accent()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(theme::text_secondary())
                        .child(text.into()),
                ),
        )
        .into_any_element()
}

impl Render for DetailPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let name = self
            .header
            .value()
            .map_or_else(|| "\u{2026}".to_owned(), |header| header.title.clone());
        let crumb = breadcrumb(&self.store, state, &name);
        let error = self.header.error().cloned();
        let header = self.header_view(cx);
        let body = self.body(cx);

        let mut column = div().flex().flex_col().size_full().child(crumb);

        if let Some(error) = error
            && self.header.value().is_none()
        {
            let this = cx.entity();

            return column
                .child(error_panel(
                    "detail-header-retry",
                    &error,
                    move |_, _, cx| {
                        this.update(cx, |page, cx| page.retry(cx));
                    },
                ))
                .into_any_element();
        }

        column = column
            .child(header)
            .child(div().flex_1().min_h_0().child(body));

        column.into_any_element()
    }
}
