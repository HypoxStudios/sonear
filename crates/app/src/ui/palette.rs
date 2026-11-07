use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{Disableable as _, IndexPath};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DismissEvent, Div, Entity, EventEmitter, FontWeight,
    InteractiveElement as _, IntoElement, Keystroke, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, px,
};
use sonear_core::SpotifyLink;
use sonear_core::model::{Artwork, Page, Playlist, SearchKind, SearchResults, Track};
use sonear_soloist::protocol::RepeatMode;

use crate::data::{self, Load};
use crate::keymap;
use crate::store::{AppStore, Route};
use crate::theme;
use crate::ui::widgets::{art, icon};

const DEBOUNCE: Duration = Duration::from_millis(300);
const MIN_SEARCH_CHARS: usize = 2;
const PLAYLIST_MATCHES: usize = 5;
const TRACKS: usize = 4;
const OTHERS: usize = 3;
const KINDS: [SearchKind; 4] = [
    SearchKind::Track,
    SearchKind::Artist,
    SearchKind::Album,
    SearchKind::Playlist,
];

struct Entry {
    id: &'static str,
    label: Option<&'static str>,
    icon: IconName,
}

const fn entry(id: &'static str, icon: IconName) -> Entry {
    Entry {
        id,
        label: None,
        icon,
    }
}

const fn named(id: &'static str, label: &'static str, icon: IconName) -> Entry {
    Entry {
        id,
        label: Some(label),
        icon,
    }
}

const COMMANDS: [(&str, &[Entry]); 4] = [
    (
        "Playback",
        &[
            entry("play_pause", IconName::Play),
            entry("next", IconName::SkipForward),
            entry("previous", IconName::SkipBack),
            entry("shuffle", IconName::Shuffle),
            entry("repeat", IconName::Repeat),
            entry("like", IconName::Heart),
        ],
    ),
    (
        "Queue",
        &[
            entry("toggle_queue", IconName::ListMusic),
            entry("clear_queue", IconName::X),
        ],
    ),
    (
        "Go to",
        &[
            named("go_home", "Home", IconName::House),
            named("go_search", "Search", IconName::Search),
            named("go_library", "Your Library", IconName::Library),
            named("go_liked", "Liked Songs", IconName::Heart),
            named("go_history", "History", IconName::Clock),
        ],
    ),
    (
        "General",
        &[
            entry("settings", IconName::Settings),
            entry("shortcuts", IconName::KeyRound),
        ],
    ),
];

#[derive(Clone)]
enum Target {
    /// The item dispatches its own action.
    Command,
    Route(Route),
    Track(String),
    Link(SpotifyLink),
    Search,
    None,
}

#[derive(Default)]
struct Groups {
    groups: Vec<CommandGroup>,
    targets: Vec<Vec<Target>>,
}

impl Groups {
    fn add(&mut self, label: Option<&str>, rows: Vec<(CommandItem, Target)>) {
        if rows.is_empty() {
            return;
        }

        let (items, targets): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
        let mut group = CommandGroup::new().items(items);

        if let Some(label) = label {
            group = group.label(label.to_owned());
        }

        self.groups.push(group);
        self.targets.push(targets);
    }
}

pub struct PaletteView {
    store: Entity<AppStore>,
    state: Entity<CommandState>,
    playlists: Vec<Playlist>,
    query: String,
    results: Option<SearchResults>,
    error: Option<String>,
    loading: bool,
    generation: u64,
    targets: Vec<Vec<Target>>,
    debounce: Option<Task<()>>,
    search: Option<Task<()>>,
}

impl EventEmitter<DismissEvent> for PaletteView {}

impl PaletteView {
    pub fn new(store: Entity<AppStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let playlists = {
            let state = store.read(cx);

            state
                .core
                .meta
                .get::<Page<Playlist>>("sidebar-playlists")
                .map_or_else(
                    || state.sidebar_playlists.clone(),
                    |cached| cached.value.items,
                )
        };

        state.update(cx, |state, cx| state.focus(window, cx));

        Self {
            store,
            state,
            playlists,
            query: String::new(),
            results: None,
            error: None,
            loading: false,
            generation: 0,
            targets: Vec::new(),
            debounce: None,
            search: None,
        }
    }

    fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        let query = query.trim().to_owned();

        if query == self.query {
            return;
        }

        self.query.clone_from(&query);
        self.generation += 1;
        self.debounce = None;

        if query.chars().count() < MIN_SEARCH_CHARS || SpotifyLink::parse(&query).is_some() {
            self.search = None;
            self.results = None;
            self.error = None;
            self.loading = false;
            cx.notify();

            return;
        }

        let generation = self.generation;

        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;

            let _ = this.update(cx, |palette, cx| {
                if palette.generation == generation {
                    palette.run_search(cx);
                }
            });
        }));
        cx.notify();
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let env = self.store.read(cx).env();
        let query = self.query.clone();
        let generation = self.generation;

        self.loading = true;
        self.error = None;
        self.search = Some(data::load(
            &env,
            None,
            data::fetch(move |core| async move { core.library.search(&query, &KINDS, 0).await }),
            cx,
            move |palette: &mut Self, message, _| {
                if palette.generation != generation {
                    return;
                }

                match message {
                    Load::Loading => {}
                    Load::Ready { value } => {
                        palette.loading = false;
                        palette.results = Some(value);
                    }
                    Load::Failed(error) => {
                        palette.loading = false;
                        palette.error = Some(error.to_string());
                    }
                }
            },
        ));
        cx.notify();
    }

    fn confirm(&mut self, index: IndexPath, cx: &mut Context<Self>) {
        let target = self
            .targets
            .get(index.section)
            .and_then(|group| group.get(index.row))
            .cloned()
            .unwrap_or(Target::None);
        let query = self.query.clone();

        self.store.update(cx, |store, cx| match target {
            Target::Route(route) => store.navigate(route, cx),
            Target::Track(uri) => store.play_tracks(vec![uri], 0, cx),
            Target::Link(link) => store.open_link(link, cx),
            Target::Search => store.search_for(&query, cx),
            Target::Command | Target::None => {}
        });
        cx.emit(DismissEvent);
    }

    fn commands(&self, cx: &App) -> Vec<(&'static str, Vec<(CommandItem, Target)>)> {
        let query = self.query.to_lowercase();
        let state = self.store.read(cx);
        let uri = state
            .playback
            .item
            .as_ref()
            .map(|item| item.uri.as_str())
            .unwrap_or_default();
        let checked = |id: &str| match id {
            "shuffle" => state.playback.options.shuffle,
            "repeat" => !matches!(
                state.playback.repeat(),
                RepeatMode::Off | RepeatMode::Unknown
            ),
            "like" => state.core.likes.is_liked(uri).unwrap_or(false),
            _ => false,
        };

        let bindings = keymap::effective(&state.core.config.get().shortcuts);

        COMMANDS
            .iter()
            .map(|(heading, entries)| {
                let rows = entries
                    .iter()
                    .filter_map(|entry| {
                        let shortcut = keymap::find(entry.id)?;
                        let label = entry.label.unwrap_or(shortcut.label);
                        let matches = query.is_empty()
                            || label.to_lowercase().contains(&query)
                            || shortcut.label.to_lowercase().contains(&query);

                        let keystroke = bindings
                            .iter()
                            .find(|(other, _)| other.id == entry.id)
                            .and_then(|(_, keystroke)| keymap::parse(keystroke.as_deref()?));
                        let (glyph, on) = (entry.icon, checked(entry.id));

                        matches.then(|| {
                            (
                                CommandItem::new()
                                    .label(label)
                                    .action(shortcut.action())
                                    .child(move |_, _| command_row(glyph, label, on, &keystroke)),
                                Target::Command,
                            )
                        })
                    })
                    .collect();

                (*heading, rows)
            })
            .collect()
    }

    fn model(&mut self, cx: &App) -> Vec<CommandGroup> {
        let mut groups = Groups::default();

        if self.query.is_empty() {
            for (heading, rows) in self.commands(cx) {
                groups.add(Some(heading), rows);
            }

            self.targets = groups.targets;

            return groups.groups;
        }

        self.typed_model(&mut groups, cx);
        self.targets = groups.targets;

        groups.groups
    }

    fn typed_model(&self, groups: &mut Groups, cx: &App) {
        if let Some(link) = SpotifyLink::parse(&self.query) {
            groups.add(Some("Spotify link"), vec![link_row(link)]);

            return;
        }

        let query = self.query.to_lowercase();

        groups.add(
            Some("Commands"),
            self.commands(cx)
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .collect(),
        );

        let playlists = self
            .playlists
            .iter()
            .filter(|playlist| playlist.name.to_lowercase().contains(&query))
            .take(PLAYLIST_MATCHES)
            .map(playlist_row)
            .collect();

        groups.add(Some("Your playlists"), playlists);

        if self.query.chars().count() >= MIN_SEARCH_CHARS {
            if let Some(results) = &self.results {
                let tracks = results
                    .tracks
                    .items
                    .iter()
                    .filter(|track| track.playable && !track.is_local)
                    .take(TRACKS)
                    .map(track_row)
                    .collect();
                let artists = results
                    .artists
                    .items
                    .iter()
                    .take(OTHERS)
                    .map(|artist| {
                        row(
                            artist.art.clone(),
                            artist.uri.clone(),
                            true,
                            artist.name.clone(),
                            "Artist".to_owned(),
                            Target::Route(Route::Artist(artist.id.clone())),
                        )
                    })
                    .collect();
                let albums = results
                    .albums
                    .items
                    .iter()
                    .take(OTHERS)
                    .map(|album| {
                        row(
                            album.art.clone(),
                            album.uri.clone(),
                            false,
                            album.name.clone(),
                            format!("Album \u{00b7} {}", album.artist_line()),
                            Target::Route(Route::Album(album.id.clone())),
                        )
                    })
                    .collect();
                let found_playlists = results
                    .playlists
                    .items
                    .iter()
                    .take(OTHERS)
                    .map(playlist_row)
                    .collect();

                groups.add(Some("Songs"), tracks);
                groups.add(Some("Artists"), artists);
                groups.add(Some("Albums"), albums);
                groups.add(Some("Playlists"), found_playlists);
            }

            if let Some(error) = &self.error {
                let message = error.clone();

                groups.add(
                    Some("Spotify"),
                    vec![(
                        CommandItem::new()
                            .label(message.clone())
                            .disabled(true)
                            .child(move |_, _| {
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::text_tertiary())
                                    .child(message.clone())
                            }),
                        Target::None,
                    )],
                );
            }
        }

        groups.add(
            None,
            vec![(
                CommandItem::new()
                    .label(format!("Search for \u{201c}{}\u{201d}", self.query))
                    .icon(icon(IconName::Search, 16., theme::text_secondary())),
                Target::Search,
            )],
        );
    }
}

fn command_row(glyph: IconName, label: &str, on: bool, keystroke: &Option<Keystroke>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .child(icon(glyph, 16., theme::text_secondary()))
        .child(div().flex_1().truncate().child(label.to_owned()))
        .when(on, |row| {
            row.child(icon(IconName::CircleCheck, 14., theme::accent()))
        })
        .children(keystroke.clone().map(Kbd::new))
}

fn link_row(link: SpotifyLink) -> (CommandItem, Target) {
    let (verb, glyph) = match link {
        SpotifyLink::Track(_) | SpotifyLink::Episode(_) => ("Play", IconName::Play),
        _ => ("Open", IconName::ExternalLink),
    };
    let label = format!("{verb} {}", link.kind_label().to_lowercase());
    let id = link.id().to_owned();
    let shown = label.clone();
    let item = CommandItem::new().label(label).child(move |_, _| {
        div()
            .flex()
            .items_center()
            .gap_3()
            .w_full()
            .child(icon(glyph, 16., theme::text_secondary()))
            .child(div().flex_1().truncate().child(shown.clone()))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::text_tertiary())
                    .child(id.clone()),
            )
    });

    (item, Target::Link(link))
}

fn playlist_row(playlist: &Playlist) -> (CommandItem, Target) {
    let subtitle = match &playlist.owner_name {
        Some(owner) => format!("Playlist \u{00b7} {owner}"),
        None => "Playlist".to_owned(),
    };

    row(
        playlist.art.clone(),
        playlist.uri.clone(),
        false,
        playlist.name.clone(),
        subtitle,
        Target::Route(Route::Playlist(playlist.id.clone())),
    )
}

fn track_row(track: &Track) -> (CommandItem, Target) {
    let subtitle = match &track.album {
        Some(album) => format!("{} \u{00b7} {}", track.artist_line(), album.name),
        None => track.artist_line(),
    };

    row(
        track.art.clone(),
        track.uri.clone(),
        false,
        track.name.clone(),
        subtitle,
        Target::Track(track.uri.clone()),
    )
}

fn row(
    artwork: Artwork,
    seed: String,
    round: bool,
    title: String,
    subtitle: String,
    target: Target,
) -> (CommandItem, Target) {
    let item = CommandItem::new()
        .label(title.clone())
        .child(move |_, _| cover_row(&artwork, &seed, round, &title, &subtitle));

    (item, target)
}

fn cover_row(
    artwork: &Artwork,
    seed: &str,
    round: bool,
    title: &str,
    subtitle: &str,
) -> AnyElement {
    let radius = if round {
        px(theme::PALETTE_COVER / 2.)
    } else {
        theme::ART_RADIUS_SMALL
    };

    div()
        .flex()
        .items_center()
        .gap_3()
        .min_w_0()
        .child(art(artwork, seed, theme::PALETTE_COVER, radius))
        .child(
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
                        .text_size(px(11.))
                        .text_color(theme::text_secondary())
                        .child(subtitle.to_owned()),
                ),
        )
        .into_any_element()
}

impl Render for PaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.read(cx).is_loading() != self.loading {
            let loading = self.loading;

            self.state
                .update(cx, |state, cx| state.set_loading(loading, window, cx));
        }

        let groups = self.model(cx);
        let queried = cx.entity().downgrade();
        let confirmed = queried.clone();
        let top = window.viewport_size().height * theme::PALETTE_TOP;
        let mut command = Command::new(&self.state)
            .filterable(false)
            .bordered(false)
            .placeholder("Type a command or search Spotify")
            .max_h(theme::PALETTE_MAX_HEIGHT)
            .empty(|_, _, _| {
                div()
                    .py_6()
                    .flex()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(theme::text_tertiary())
                    .child("Nothing found")
            })
            .on_query(move |query, _, cx| {
                let _ = queried.update(cx, |palette, cx| palette.set_query(query, cx));
            })
            .on_confirm(move |index, _, cx| {
                let _ = confirmed.update(cx, |palette, cx| palette.confirm(index, cx));
            })
            .bg(theme::panel());

        for group in groups {
            command = command.group(group);
        }

        div()
            .id("palette-backdrop")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .pt(top)
            .bg(theme::scrim())
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
            .child(
                div()
                    .id("palette")
                    .w(theme::OVERLAY_WIDTH)
                    .overflow_hidden()
                    .rounded(theme::PANEL_RADIUS)
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::panel())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(command),
            )
    }
}
