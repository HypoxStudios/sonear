//! Pure parsers over InnerTube JSON. The responses are untrusted and change
//! shape without notice, so a row that does not look right is skipped and
//! only a response with none of the expected structure fails.

use std::collections::HashSet;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use serde_json::Value;

use crate::error::YoutubeError;
use crate::models::{
    AccountInfo, AlbumCard, AlbumPage, AlbumRef, ArtistCard, ArtistPage, ArtistRef, BrowseRef,
    HomeItem, HomeShelf, PlaylistCard, PlaylistPage, SearchResults, ShelfKind, Song, Thumb,
    TrackPage,
};

const COUNT_WORDS: [&str; 8] = [
    "song", "track", "canci", "pista", "titel", "morceau", "brani", "músicas",
];
const LABEL_WORDS: [&str; 5] = ["song", "video", "canción", "cancion", "vídeo"];
const EPISODE_WORDS: [&str; 4] = ["episode", "episodio", "épisode", "folge"];

type Runs<'a> = Vec<&'a Value>;
type Groups<'a> = Vec<Runs<'a>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Link {
    Artist,
    Channel,
    Album,
    Playlist,
    Other,
}

enum Target<'a> {
    Video(&'a str),
    Browse(Link, &'a str),
    Playlist(&'a str),
}

struct Parts<'a> {
    title: String,
    target: Target<'a>,
    groups: Groups<'a>,
    runs: Runs<'a>,
    thumbs: Vec<Thumb>,
    duration_ms: Option<u64>,
    explicit: bool,
}

fn decode(detail: &str) -> YoutubeError {
    YoutubeError::Decode(detail.to_owned())
}

fn string_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn runs_of(text: Option<&Value>) -> Runs<'_> {
    text.and_then(|text| text.get("runs"))
        .and_then(Value::as_array)
        .map(|runs| runs.iter().collect())
        .unwrap_or_default()
}

fn run_text(run: &Value) -> &str {
    run.get("text").and_then(Value::as_str).unwrap_or_default()
}

fn join_text(runs: &[&Value]) -> String {
    runs.iter().map(|run| run_text(run)).collect()
}

fn find_key<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    match node {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }

            map.values().find_map(|child| find_key(child, key))
        }
        Value::Array(items) => items.iter().find_map(|child| find_key(child, key)),
        _ => None,
    }
}

fn link_of(endpoint: &Value) -> (Link, &str) {
    let id = endpoint
        .get("browseId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let page = string_at(
        endpoint,
        "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
    );
    let link = match page {
        Some("MUSIC_PAGE_TYPE_ARTIST") => Link::Artist,
        Some("MUSIC_PAGE_TYPE_USER_CHANNEL") => Link::Channel,
        Some("MUSIC_PAGE_TYPE_ALBUM") => Link::Album,
        Some("MUSIC_PAGE_TYPE_PLAYLIST") => Link::Playlist,
        Some(_) => Link::Other,
        None if id.starts_with("UC") => Link::Artist,
        None if id.starts_with("MPRE") => Link::Album,
        None if id.starts_with("VL") => Link::Playlist,
        None => Link::Other,
    };

    (link, id)
}

fn run_link(run: &Value) -> Option<(Link, &str)> {
    let endpoint = run.pointer("/navigationEndpoint/browseEndpoint")?;
    let (link, id) = link_of(endpoint);

    (!id.is_empty()).then_some((link, id))
}

fn is_separator(run: &Value) -> bool {
    matches!(run_text(run).trim(), "•" | "·")
}

fn split_groups<'a>(runs: &[&'a Value]) -> Groups<'a> {
    let mut groups: Groups = Vec::new();
    let mut current: Runs = Vec::new();

    for run in runs {
        if is_separator(run) {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }

            continue;
        }

        current.push(run);
    }

    if !current.is_empty() {
        groups.push(current);
    }

    groups
}

pub fn parse_duration(text: &str) -> Option<u64> {
    let mut seconds = 0u64;
    let mut parts = 0;

    for part in text.trim().split(':') {
        if part.is_empty() || part.len() > 2 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }

        seconds = seconds * 60 + part.parse::<u64>().ok()?;
        parts += 1;
    }

    (2..=3).contains(&parts).then_some(seconds * 1000)
}

fn is_year(text: &str) -> bool {
    text.len() == 4 && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn track_count_of(text: &str) -> Option<u32> {
    let text = text.trim();
    let (number, rest) = text.split_once(char::is_whitespace)?;
    let digits: String = number
        .chars()
        .filter(|c| !matches!(c, ',' | '.' | '\u{a0}'))
        .collect();
    let rest = rest.to_lowercase();

    if digits.is_empty() || !COUNT_WORDS.iter().any(|word| rest.contains(word)) {
        return None;
    }

    digits.parse().ok()
}

fn count_in(runs: &[&Value]) -> Option<u32> {
    runs.iter().find_map(|run| track_count_of(run_text(run)))
}

fn names(text: &str) -> Vec<ArtistRef> {
    text.replace(" & ", ", ")
        .split(", ")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| ArtistRef {
            id: None,
            name: name.to_owned(),
        })
        .collect()
}

fn linked_artists(runs: &[&Value]) -> Vec<ArtistRef> {
    runs.iter()
        .filter_map(|run| match run_link(run)? {
            (Link::Artist | Link::Channel, id) => Some(ArtistRef {
                id: Some(id.to_owned()),
                name: run_text(run).to_owned(),
            }),
            _ => None,
        })
        .collect()
}

fn linked_album(runs: &[&Value]) -> Option<AlbumRef> {
    runs.iter().find_map(|run| match run_link(run)? {
        (Link::Album, id) => Some(AlbumRef {
            id: id.to_owned(),
            name: run_text(run).to_owned(),
        }),
        _ => None,
    })
}

fn is_duration_group(group: &[&Value]) -> bool {
    parse_duration(&join_text(group)).is_some()
}

fn group_duration(groups: &[Vec<&Value>]) -> Option<u64> {
    groups
        .iter()
        .find_map(|group| parse_duration(&join_text(group)))
}

fn unlinked_song_artists(groups: &[Vec<&Value>]) -> Vec<ArtistRef> {
    let candidates: Vec<&Runs> = groups
        .iter()
        .filter(|group| !is_duration_group(group))
        .collect();
    let group = match candidates.as_slice() {
        [] => return Vec::new(),
        [only] => {
            let text = join_text(only);

            if LABEL_WORDS.contains(&text.to_lowercase().as_str()) {
                return Vec::new();
            }

            only
        }
        [_, second, ..] => second,
    };

    names(&join_text(group))
}

struct AlbumMeta {
    kind: String,
    year: Option<String>,
    artists: Vec<ArtistRef>,
}

fn album_meta(groups: &[Vec<&Value>], runs: &[&Value]) -> AlbumMeta {
    let year = groups
        .iter()
        .map(|group| join_text(group))
        .find(|text| is_year(text));
    let kind_group = groups.first().filter(|group| {
        groups.len() >= 2
            && group.iter().all(|run| run_link(run).is_none())
            && !is_year(&join_text(group))
    });
    let kind = kind_group
        .map(|group| join_text(group))
        .unwrap_or_else(|| "Album".to_owned());
    let mut artists = linked_artists(runs);

    if artists.is_empty() {
        let skip = usize::from(kind_group.is_some());

        artists = groups
            .iter()
            .skip(skip)
            .map(|group| join_text(group))
            .find(|text| !is_year(text))
            .map(|text| names(&text))
            .unwrap_or_default();
    }

    AlbumMeta {
        kind,
        year,
        artists,
    }
}

fn playlist_author(groups: &[Vec<&Value>]) -> Option<String> {
    let labelled = groups.len() == 2
        && matches!(
            join_text(&groups[0]).to_lowercase().as_str(),
            "playlist" | "lista de reproducción"
        );
    let group = match groups.len() {
        0 => return None,
        2 if labelled => &groups[1],
        1 | 2 => &groups[0],
        _ => &groups[1],
    };
    let text = join_text(group);

    if track_count_of(&text).is_some() || LABEL_WORDS.contains(&text.to_lowercase().as_str()) {
        return None;
    }

    Some(text)
}

fn thumbs_of(node: Option<&Value>) -> Vec<Thumb> {
    let Some(node) = node else {
        return Vec::new();
    };
    let list = [
        "/musicThumbnailRenderer/thumbnail/thumbnails",
        "/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/thumbnails",
        "/thumbnails",
    ]
    .iter()
    .find_map(|pointer| node.pointer(pointer).and_then(Value::as_array));

    list.map(|list| {
        list.iter()
            .filter_map(|thumb| {
                let url = thumb.get("url")?.as_str()?;

                Some(Thumb {
                    url: url.to_owned(),
                    width: thumb
                        .get("width")
                        .and_then(Value::as_u64)
                        .and_then(|width| u32::try_from(width).ok()),
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

fn is_explicit(badges: Option<&Value>) -> bool {
    badges.and_then(Value::as_array).is_some_and(|badges| {
        badges.iter().any(|badge| {
            string_at(badge, "/musicInlineBadgeRenderer/icon/iconType")
                == Some("MUSIC_EXPLICIT_BADGE")
        })
    })
}

fn strip_vl(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

fn target_of(holder: &Value) -> Option<Target<'_>> {
    let nav = holder.get("navigationEndpoint")?;

    if let Some(watch) = nav.get("watchEndpoint") {
        let id = watch.get("videoId").and_then(Value::as_str)?;
        let kind = string_at(
            watch,
            "/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
        );

        if kind.is_some_and(|kind| kind.contains("PODCAST")) {
            return None;
        }

        return Some(Target::Video(id));
    }

    if let Some(browse) = nav.get("browseEndpoint") {
        let (link, id) = link_of(browse);

        return (!id.is_empty()).then_some(Target::Browse(link, id));
    }

    let id = string_at(nav, "/watchPlaylistEndpoint/playlistId")?;

    Some(Target::Playlist(id))
}

fn build(parts: Parts<'_>) -> Option<HomeItem> {
    let Parts {
        title,
        target,
        groups,
        runs,
        thumbs,
        duration_ms,
        explicit,
    } = parts;

    if title.trim().is_empty() {
        return None;
    }

    match target {
        Target::Video(id) => {
            let mut artists = linked_artists(&runs);

            if artists.is_empty() {
                artists = unlinked_song_artists(&groups);
            }

            Some(HomeItem::Song(Song {
                video_id: id.to_owned(),
                title,
                artists,
                album: linked_album(&runs),
                duration_ms: duration_ms.or_else(|| group_duration(&groups)),
                thumbs,
                explicit,
            }))
        }
        Target::Browse(Link::Album, id) => {
            let meta = album_meta(&groups, &runs);

            Some(HomeItem::Album(AlbumCard {
                browse_id: id.to_owned(),
                title,
                artists: meta.artists,
                year: meta.year,
                kind: meta.kind,
                thumbs,
            }))
        }
        Target::Browse(Link::Artist, id) => Some(HomeItem::Artist(ArtistCard {
            channel_id: id.to_owned(),
            name: title,
            thumbs,
        })),
        Target::Browse(Link::Playlist, id) | Target::Playlist(id) => {
            Some(HomeItem::Playlist(PlaylistCard {
                playlist_id: strip_vl(id).to_owned(),
                title,
                author: playlist_author(&groups),
                track_count: groups.iter().find_map(|group| count_in(group)),
                thumbs,
            }))
        }
        Target::Browse(..) => None,
    }
}

fn parse_list_item(item: &Value) -> Option<HomeItem> {
    let columns: Vec<Runs> = item
        .get("flexColumns")?
        .as_array()?
        .iter()
        .map(|column| runs_of(column.pointer("/musicResponsiveListItemFlexColumnRenderer/text")))
        .collect();
    let title_runs = columns.first()?;
    let video = string_at(item, "/playlistItemData/videoId").or_else(|| {
        title_runs
            .iter()
            .find_map(|run| string_at(run, "/navigationEndpoint/watchEndpoint/videoId"))
    });
    let target = match video {
        Some(id) => {
            let podcast = title_runs.iter().any(|run| {
                string_at(
                    run,
                    "/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
                )
                .is_some_and(|kind| kind.contains("PODCAST"))
            });

            if podcast {
                return None;
            }

            Target::Video(id)
        }
        None => target_of(item)?,
    };
    let rest: Runs = columns.iter().skip(1).flatten().copied().collect();
    let groups = columns
        .get(1)
        .map(|runs| split_groups(runs))
        .unwrap_or_default();
    let episode = groups
        .first()
        .is_some_and(|group| EPISODE_WORDS.contains(&join_text(group).to_lowercase().as_str()));

    if episode {
        return None;
    }

    let duration_ms = item
        .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")
        .and_then(|text| parse_duration(&join_text(&runs_of(Some(text)))));

    build(Parts {
        title: join_text(title_runs),
        target,
        groups,
        runs: rest,
        thumbs: thumbs_of(item.get("thumbnail")),
        duration_ms,
        explicit: is_explicit(item.get("badges")),
    })
}

fn parse_two_row(item: &Value) -> Option<HomeItem> {
    let subtitle = runs_of(item.get("subtitle"));

    build(Parts {
        title: join_text(&runs_of(item.get("title"))),
        target: target_of(item)?,
        groups: split_groups(&subtitle),
        runs: subtitle,
        thumbs: thumbs_of(item.get("thumbnailRenderer")),
        duration_ms: None,
        explicit: is_explicit(item.get("subtitleBadges")),
    })
}

fn parse_card(card: &Value) -> Option<HomeItem> {
    let title = runs_of(card.get("title"));
    let subtitle = runs_of(card.get("subtitle"));
    let target = target_of(title.first()?)?;

    build(Parts {
        title: join_text(&title),
        target,
        groups: split_groups(&subtitle),
        runs: subtitle,
        thumbs: thumbs_of(card.get("thumbnail")),
        duration_ms: None,
        explicit: false,
    })
}

fn collect_items(node: &Value, out: &mut Vec<HomeItem>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                match key.as_str() {
                    "musicResponsiveListItemRenderer" => out.extend(parse_list_item(value)),
                    "musicTwoRowItemRenderer" => out.extend(parse_two_row(value)),
                    "musicCardShelfRenderer" => collect_card(value, out),
                    "menu" | "overlay" | "thumbnailOverlay" => {}
                    _ => collect_items(value, out),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_items(item, out);
            }
        }
        _ => {}
    }
}

fn collect_card(card: &Value, out: &mut Vec<HomeItem>) {
    let top = parse_card(card);
    let mut inside = Vec::new();

    collect_items(card.get("contents").unwrap_or(&Value::Null), &mut inside);

    if let Some(HomeItem::Artist(artist)) = &top {
        for item in &mut inside {
            if let HomeItem::Song(song) = item
                && song.artists.is_empty()
            {
                song.artists = vec![ArtistRef {
                    id: Some(artist.channel_id.clone()),
                    name: artist.name.clone(),
                }];
            }
        }
    }

    out.extend(top);
    out.extend(inside);
}

fn items_in(node: &Value) -> Vec<HomeItem> {
    let mut items = Vec::new();

    collect_items(node, &mut items);

    items
}

fn songs_in(node: Option<&Value>) -> Vec<Song> {
    items_in(node.unwrap_or(&Value::Null))
        .into_iter()
        .filter_map(|item| match item {
            HomeItem::Song(song) => Some(song),
            _ => None,
        })
        .collect()
}

fn albums_in(node: &Value) -> Vec<AlbumCard> {
    items_in(node)
        .into_iter()
        .filter_map(|item| match item {
            HomeItem::Album(album) => Some(album),
            _ => None,
        })
        .collect()
}

fn dedup<T>(items: Vec<T>, key: impl Fn(&T) -> String) -> Vec<T> {
    let mut seen = HashSet::new();

    items
        .into_iter()
        .filter(|item| seen.insert(key(item)))
        .collect()
}

pub fn search(root: &Value) -> Result<SearchResults, YoutubeError> {
    let contents = root
        .get("contents")
        .ok_or_else(|| decode("the search answer has no contents"))?;
    let mut results = SearchResults::default();

    for item in items_in(contents) {
        match item {
            HomeItem::Song(song) => results.songs.push(song),
            HomeItem::Album(album) => results.albums.push(album),
            HomeItem::Artist(artist) => results.artists.push(artist),
            HomeItem::Playlist(playlist) => results.playlists.push(playlist),
        }
    }

    results.songs = dedup(results.songs, |song| song.video_id.clone());
    results.albums = dedup(results.albums, |album| album.browse_id.clone());
    results.artists = dedup(results.artists, |artist| artist.channel_id.clone());
    results.playlists = dedup(results.playlists, |playlist| playlist.playlist_id.clone());

    Ok(results)
}

pub fn album(root: &Value, browse_id: &str) -> Result<AlbumPage, YoutubeError> {
    let contents = root
        .get("contents")
        .ok_or_else(|| decode("the album answer has no contents"))?;
    let header = find_key(contents, "musicResponsiveHeaderRenderer")
        .ok_or_else(|| decode("the album answer has no header"))?;
    let title = join_text(&runs_of(header.get("title")));

    if title.is_empty() {
        return Err(decode("the album has no title"));
    }

    let subtitle = runs_of(header.get("subtitle"));
    let strapline = runs_of(header.get("straplineTextOne"));
    let groups = split_groups(&subtitle);
    let meta = album_meta(&groups, &strapline);
    let mut artists = linked_artists(&strapline);

    if artists.is_empty() {
        artists = names(&join_text(&strapline));
    }

    let thumbs = thumbs_of(header.get("thumbnail"));
    let card = AlbumCard {
        browse_id: browse_id.to_owned(),
        title: title.clone(),
        artists,
        year: meta.year,
        kind: meta.kind,
        thumbs: thumbs.clone(),
    };
    let shelf = root
        .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents")
        .and_then(|node| find_key(node, "musicShelfRenderer"));
    let audio_playlist_id = shelf.and_then(first_audio_playlist);
    let album_ref = AlbumRef {
        id: browse_id.to_owned(),
        name: title,
    };
    let tracks = songs_in(shelf.and_then(|shelf| shelf.get("contents")))
        .into_iter()
        .map(|mut song| {
            song.album = Some(album_ref.clone());
            song.thumbs = thumbs.clone();

            if song.artists.is_empty() {
                song.artists = card.artists.clone();
            }

            song
        })
        .collect::<Vec<_>>();

    Ok(AlbumPage {
        track_count: count_in(&runs_of(header.get("secondSubtitle"))),
        album: card,
        audio_playlist_id,
        tracks,
    })
}

fn first_audio_playlist(node: &Value) -> Option<String> {
    match node {
        Value::Object(map) => {
            if let Some(id) = map.get("playlistId").and_then(Value::as_str)
                && id.starts_with("OLAK5uy")
            {
                return Some(id.to_owned());
            }

            map.values().find_map(first_audio_playlist)
        }
        Value::Array(items) => items.iter().find_map(first_audio_playlist),
        _ => None,
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let decoded = (bytes[index] == b'%' && index + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[index + 1..index + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());

        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// 1 for the albums grid and 2 for singles and EPs, read from the protobuf
/// in the grid's `params` so it works in every interface language.
fn discography_kind(params: &str) -> Option<u8> {
    let decoded = percent_decode(params);
    let trimmed = decoded.trim_end_matches('=');
    let bytes = STANDARD_NO_PAD
        .decode(trimmed)
        .or_else(|_| URL_SAFE_NO_PAD.decode(trimmed))
        .ok()?;

    bytes
        .windows(4)
        .find(|window| window[0] == 6 && window[1] == 8 && window[3] == 0x1a)
        .map(|window| window[2])
}

fn title_kind(title: &str) -> Option<u8> {
    let title = title.to_lowercase();

    if title.contains("single") || title.contains("sencillo") || title.contains("ep") {
        Some(2)
    } else if title.contains("album") || title.contains("álbum") {
        Some(1)
    } else {
        None
    }
}

pub fn artist(root: &Value, channel_id: &str) -> Result<ArtistPage, YoutubeError> {
    let header = root
        .get("header")
        .and_then(|header| {
            [
                "musicImmersiveHeaderRenderer",
                "musicVisualHeaderRenderer",
                "musicHeaderRenderer",
            ]
            .iter()
            .find_map(|key| header.get(key))
        })
        .ok_or_else(|| decode("the artist answer has no header"))?;
    let name = join_text(&runs_of(header.get("title")));

    if name.is_empty() {
        return Err(decode("the artist has no name"));
    }

    let artist = ArtistCard {
        channel_id: channel_id.to_owned(),
        name,
        thumbs: thumbs_of(header.get("thumbnail")),
    };
    let sections = root
        .get("contents")
        .and_then(|contents| find_key(contents, "sectionListRenderer"))
        .and_then(|list| list.get("contents"))
        .and_then(Value::as_array)
        .ok_or_else(|| decode("the artist answer has no sections"))?;
    let mut page = ArtistPage {
        artist,
        ..ArtistPage::default()
    };

    for section in sections {
        if let Some(shelf) = section.get("musicShelfRenderer") {
            read_top_songs(shelf, &mut page);
        } else if let Some(carousel) = section.get("musicCarouselShelfRenderer") {
            read_discography(carousel, &mut page);
        }
    }

    Ok(page)
}

fn read_top_songs(shelf: &Value, page: &mut ArtistPage) {
    if !page.top_songs.is_empty() {
        return;
    }

    page.top_songs = songs_in(shelf.get("contents"));
    page.top_songs_playlist = string_at(shelf, "/bottomEndpoint/browseEndpoint/browseId")
        .or_else(|| {
            string_at(
                shelf,
                "/title/runs/0/navigationEndpoint/browseEndpoint/browseId",
            )
        })
        .filter(|id| id.starts_with("VL"))
        .map(|id| strip_vl(id).to_owned());
}

fn read_discography(carousel: &Value, page: &mut ArtistPage) {
    let header = carousel.pointer("/header/musicCarouselShelfBasicHeaderRenderer");
    let more = header
        .and_then(|header| {
            header.pointer("/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint")
        })
        .filter(|endpoint| {
            endpoint
                .get("browseId")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with("MPAD"))
        });
    let title = header
        .map(|header| join_text(&runs_of(header.get("title"))))
        .unwrap_or_default();
    let params = more
        .and_then(|endpoint| endpoint.get("params"))
        .and_then(Value::as_str);
    let kind = params
        .and_then(discography_kind)
        .or_else(|| title_kind(&title));
    let reference = more.and_then(|endpoint| {
        Some(BrowseRef {
            browse_id: endpoint.get("browseId")?.as_str()?.to_owned(),
            params: params.map(str::to_owned),
        })
    });
    let mut albums = albums_in(carousel.get("contents").unwrap_or(&Value::Null));

    for album in &mut albums {
        if album.artists.is_empty() {
            album.artists = vec![ArtistRef {
                id: Some(page.artist.channel_id.clone()),
                name: page.artist.name.clone(),
            }];
        }
    }

    match kind {
        Some(1) if page.albums.is_empty() => {
            page.albums = albums;
            page.albums_more = reference;
        }
        Some(2) if page.singles.is_empty() => {
            page.singles = albums;
            page.singles_more = reference;
        }
        _ => {}
    }
}

pub fn artist_releases(root: &Value) -> Result<Vec<AlbumCard>, YoutubeError> {
    let contents = root
        .get("contents")
        .ok_or_else(|| decode("the releases answer has no contents"))?;

    Ok(dedup(albums_in(contents), |album| album.browse_id.clone()))
}

fn shelf_continuation(shelf: &Value) -> Option<String> {
    let token = shelf
        .pointer("/continuations/0/nextContinuationData/continuation")
        .and_then(Value::as_str)
        .map(str::to_owned);

    token.or_else(|| {
        shelf
            .get("contents")?
            .as_array()?
            .iter()
            .find_map(item_token)
    })
}

fn item_token(item: &Value) -> Option<String> {
    string_at(
        item,
        "/continuationItemRenderer/continuationEndpoint/continuationCommand/token",
    )
    .map(str::to_owned)
}

pub fn playlist(root: &Value, playlist_id: &str) -> Result<PlaylistPage, YoutubeError> {
    let contents = root
        .get("contents")
        .ok_or_else(|| decode("the playlist answer has no contents"))?;
    let header = find_key(contents, "musicResponsiveHeaderRenderer")
        .ok_or_else(|| decode("the playlist answer has no header"))?;
    let title = join_text(&runs_of(header.get("title")));

    if title.is_empty() {
        return Err(decode("the playlist has no title"));
    }

    let strapline = join_text(&runs_of(header.get("straplineTextOne")));
    let author = Some(strapline).filter(|text| !text.is_empty()).or_else(|| {
        string_at(header, "/facepile/avatarStackViewModel/text/content").map(str::to_owned)
    });
    let description = header
        .pointer("/description/musicDescriptionShelfRenderer/description")
        .or_else(|| header.get("description"))
        .map(|text| join_text(&runs_of(Some(text))))
        .unwrap_or_default();
    let shelf = find_key(contents, "musicPlaylistShelfRenderer")
        .or_else(|| find_key(contents, "musicShelfRenderer"));
    let playlist = PlaylistCard {
        playlist_id: strip_vl(playlist_id).to_owned(),
        title,
        author,
        track_count: count_in(&runs_of(header.get("secondSubtitle"))),
        thumbs: thumbs_of(header.get("thumbnail")),
    };

    Ok(PlaylistPage {
        playlist,
        description,
        tracks: songs_in(shelf.and_then(|shelf| shelf.get("contents"))),
        continuation: shelf.and_then(shelf_continuation),
    })
}

pub fn track_page(root: &Value) -> Result<TrackPage, YoutubeError> {
    if let Some(shelf) = root.pointer("/continuationContents/musicPlaylistShelfContinuation") {
        return Ok(TrackPage {
            tracks: songs_in(shelf.get("contents")),
            continuation: shelf_continuation(shelf),
        });
    }

    let items = root
        .get("onResponseReceivedActions")
        .and_then(Value::as_array)
        .and_then(|actions| {
            actions.iter().find_map(|action| {
                action
                    .pointer("/appendContinuationItemsAction/continuationItems")
                    .and_then(Value::as_array)
            })
        })
        .ok_or_else(|| decode("the continuation answer has no items"))?;

    Ok(TrackPage {
        tracks: songs_in(Some(&Value::Array(items.clone()))),
        continuation: items.iter().find_map(item_token),
    })
}

fn carousel_shelves(root: &Value) -> Result<Vec<(&Value, HomeShelf)>, YoutubeError> {
    let sections = root
        .get("contents")
        .and_then(|contents| find_key(contents, "sectionListRenderer"))
        .and_then(|list| list.get("contents"))
        .and_then(Value::as_array)
        .ok_or_else(|| decode("the answer has no sections"))?;
    let mut shelves = Vec::new();

    for section in sections {
        let Some(carousel) = section.get("musicCarouselShelfRenderer") else {
            continue;
        };
        let title = carousel
            .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")
            .map(|text| join_text(&runs_of(Some(text))))
            .unwrap_or_default();
        let items = items_in(carousel.get("contents").unwrap_or(&Value::Null));

        if title.is_empty() || items.is_empty() {
            continue;
        }

        shelves.push((carousel, HomeShelf::new(title, items)));
    }

    Ok(shelves)
}

pub fn home(root: &Value) -> Result<Vec<HomeShelf>, YoutubeError> {
    Ok(carousel_shelves(root)?
        .into_iter()
        .map(|(_, shelf)| shelf)
        .collect())
}

/// The account's name and photo from the account menu.
pub fn account(root: &Value) -> Result<AccountInfo, YoutubeError> {
    let header = find_key(root, "activeAccountHeaderRenderer")
        .ok_or_else(|| decode("the account menu has no account"))?;
    let name = join_text(&runs_of(header.get("accountName")));

    if name.is_empty() {
        return Err(decode("the account has no name"));
    }

    Ok(AccountInfo {
        name,
        photo: thumbs_of(header.get("accountPhoto")),
    })
}

/// The token for the next page of a library list, if there is one.
fn next_page(root: &Value) -> Option<String> {
    find_key(root, "nextContinuationData")
        .and_then(|data| data.get("continuation"))
        .or_else(|| find_key(root, "continuationCommand").and_then(|command| command.get("token")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// What one page of a library list holds and where the next one is.
pub fn library_page(root: &Value) -> Result<(Vec<HomeItem>, Option<String>), YoutubeError> {
    let body = [
        "contents",
        "continuationContents",
        "onResponseReceivedActions",
    ]
    .iter()
    .find_map(|key| root.get(key))
    .ok_or_else(|| decode("the library answer has no contents"))?;

    Ok((items_in(body), next_page(body)))
}

/// The album shelves and the song-list shelves of the Explore page, in page
/// order. Genre buttons and the music video shelf are left out.
pub fn explore(root: &Value) -> Result<Vec<HomeShelf>, YoutubeError> {
    Ok(carousel_shelves(root)?
        .into_iter()
        .filter(|(carousel, shelf)| match shelf.kind {
            ShelfKind::Songs => carousel
                .pointer("/contents/0/musicResponsiveListItemRenderer")
                .is_some(),
            ShelfKind::Albums => true,
            _ => false,
        })
        .map(|(_, shelf)| shelf)
        .collect())
}

fn panel_song(item: &Value) -> Option<Song> {
    let renderer = item.get("playlistPanelVideoRenderer").or_else(|| {
        item.pointer(
            "/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer",
        )
    })?;
    let video_id = renderer.get("videoId")?.as_str()?;
    let title = join_text(&runs_of(renderer.get("title")));

    if title.is_empty() {
        return None;
    }

    let byline = {
        let long = runs_of(renderer.get("longBylineText"));

        if long.is_empty() {
            runs_of(renderer.get("shortBylineText"))
        } else {
            long
        }
    };
    let groups = split_groups(&byline);
    let mut artists = linked_artists(&byline);

    if artists.is_empty() {
        artists = groups
            .first()
            .map(|group| names(&join_text(group)))
            .unwrap_or_default();
    }

    Some(Song {
        video_id: video_id.to_owned(),
        title,
        artists,
        album: linked_album(&byline),
        duration_ms: parse_duration(&join_text(&runs_of(renderer.get("lengthText")))),
        thumbs: thumbs_of(renderer.get("thumbnail")),
        explicit: is_explicit(renderer.get("badges")),
    })
}

pub fn up_next(root: &Value) -> Result<Vec<Song>, YoutubeError> {
    let panel = root
        .get("contents")
        .and_then(|contents| find_key(contents, "playlistPanelRenderer"))
        .ok_or_else(|| decode("the queue answer has no playlist panel"))?;

    Ok(panel
        .get("contents")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(panel_song).collect())
        .unwrap_or_default())
}
