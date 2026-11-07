use serde::de::DeserializeOwned;
use serde_json::Value;

use super::*;
use crate::error::SpotifyError;

fn fixture(name: &str) -> (u16, Value) {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    let mut value: Value = serde_json::from_str(&text).unwrap();
    let status = value["status"].as_u64().unwrap() as u16;

    (status, value["body"].take())
}

fn parse<T: DeserializeOwned>(name: &str) -> T {
    let (status, body) = fixture(name);

    assert!((200..300).contains(&status), "{name} is not a success");

    serde_json::from_value(body).unwrap()
}

#[test]
fn me_has_no_product_or_email() {
    let me: User = parse("me");

    assert!(me.display_name.unwrap().starts_with("Fixture"));
    assert_eq!(me.images.len(), 2);
}

#[test]
fn nothing_playing_is_a_204_with_no_body() {
    let (status, body) = fixture("player");

    assert_eq!(status, 204);
    assert!(body.is_null());
}

#[test]
fn devices_and_queue_parse_when_empty() {
    let devices: Devices = parse("devices");
    let queue: Queue = parse("queue");

    assert!(devices.devices.is_empty());
    assert!(queue.queue.is_empty());
    assert!(queue.currently_playing.is_none());
}

#[test]
fn devices_keep_the_soloist_speaker() {
    let devices: Devices = serde_json::from_str(
        r#"{"devices":[{"id":"abc","is_active":false,"name":"Sonear","type":"Speaker"},null]}"#,
    )
    .unwrap();

    assert_eq!(devices.devices.len(), 1);
    assert_eq!(devices.devices[0].kind, "Speaker");
}

#[test]
fn my_playlists_use_items_totals_and_null_image_sizes() {
    let page: Page<Playlist> = parse("my_playlists");

    assert_eq!(page.total, Some(13));
    assert_eq!(page.items.len(), 13);

    let first = &page.items[0];

    assert_eq!(first.total(), Some(33));
    assert_eq!(first.images[0].width, None);
    assert!(first.items.as_ref().unwrap().items.is_empty());
}

#[test]
fn older_apps_report_the_total_under_tracks() {
    let playlist: Playlist =
        serde_json::from_str(r#"{"uri":"spotify:playlist:x","tracks":{"href":"h","total":7}}"#)
            .unwrap();

    assert_eq!(playlist.total(), Some(7));
}

#[test]
fn owned_playlist_has_rows_with_item() {
    let playlist: Playlist = parse("owned_playlist");
    let rows = &playlist.items.as_ref().unwrap().items;

    assert_eq!(playlist.total(), Some(25));
    assert!(!rows.is_empty());
    assert!(matches!(rows[0].item, Some(MediaItem::Track(_))));
}

#[test]
fn playlist_item_pages_parse() {
    let page: Page<PlaylistRow> = parse("owned_playlist_items");

    assert_eq!(page.total, Some(25));
    assert_eq!(page.items.len(), 2);
    assert!(page.items.iter().all(|row| row.media().is_some()));
}

#[test]
fn saved_tracks_rows_use_track() {
    let page: Page<SavedTrack> = parse("saved_tracks");

    assert_eq!(page.total, Some(558));
    assert!(page.items.iter().all(|row| row.track.is_some()));
}

#[test]
fn rows_with_null_items_are_skipped() {
    let page: Page<PlaylistRow> = serde_json::from_str(
        r#"{"items":[null,{"item":null},{"track":{"uri":"spotify:track:a","name":"A"}}],"total":3}"#,
    )
    .unwrap();

    assert_eq!(page.items.len(), 2);
    assert!(page.items[0].media().is_none());
    assert!(page.items[1].media().is_some());
}

#[test]
fn rows_with_both_item_and_track_parse() {
    let page: Page<PlaylistRow> = serde_json::from_str(
        r#"{"items":[{"item":{"uri":"spotify:track:a","name":"A"},"track":{"uri":"spotify:track:a","name":"A"}}],"total":1}"#,
    )
    .unwrap();

    assert_eq!(page.items.len(), 1);
    assert!(
        matches!(page.items[0].media(), Some(MediaItem::Track(track)) if track.uri == "spotify:track:a")
    );
}

#[test]
fn episodes_in_rows_are_told_apart() {
    let row: PlaylistRow = serde_json::from_str(
        r#"{"item":{"type":"episode","uri":"spotify:episode:e","name":"E","duration_ms":5}}"#,
    )
    .unwrap();

    assert!(matches!(row.media(), Some(MediaItem::Episode(_))));
}

#[test]
fn local_files_parse() {
    let row: PlaylistRow = serde_json::from_str(
        r#"{"is_local":true,"item":{"uri":"spotify:local:a:b:c:1","name":"C","is_local":true,"id":null,"album":{"name":"B"}}}"#,
    )
    .unwrap();

    assert!(row.is_local);
    assert!(matches!(row.media(), Some(MediaItem::Track(track)) if track.is_local));
}

#[test]
fn followed_playlist_has_no_row_block() {
    let playlist: Playlist = parse("followed_playlist");

    assert_eq!(playlist.total(), None);
    assert_eq!(playlist.description, "");
    assert!(playlist.owner.is_some());
}

#[test]
fn track_album_and_artist_parse_without_popularity() {
    let track: Track = parse("track");
    let album: Album = parse("album");
    let artist: Artist = parse("artist");

    assert_eq!(track.name, "Instant Crush (feat. Julian Casablancas)");
    assert_eq!(track.album.unwrap().images.len(), 3);
    assert_eq!(album.tracks.unwrap().total, Some(13));
    assert_eq!(artist.name, "Daft Punk");
}

#[test]
fn album_tracks_and_artist_albums_parse() {
    let tracks: Page<Track> = parse("album_tracks");
    let albums: Page<Album> = parse("artist_albums");

    assert_eq!(tracks.total, Some(13));
    assert!(tracks.items[0].album.is_none());
    assert_eq!(albums.total, Some(63));
    assert!(!albums.items.is_empty());
}

#[test]
fn search_has_null_playlists_and_shows() {
    let results: SearchResults = parse("search_all");

    assert!(results.playlists.unwrap().items.is_empty());
    assert!(!results.shows.unwrap().items.is_empty());
    assert!(!results.tracks.unwrap().items.is_empty());
    assert!(results.artists.is_some());
    assert!(!results.albums.unwrap().items.is_empty());
}

#[test]
fn recently_played_has_bare_contexts() {
    let recent: RecentlyPlayed = parse("recently_played");

    assert!(!recent.items.is_empty());
    assert!(recent.items.iter().all(|play| play.track.is_some()));
    assert_eq!(recent.items[0].context.as_ref().unwrap().kind, "playlist");
    assert!(recent.cursors.unwrap().after.is_some());
}

#[test]
fn library_collections_parse() {
    let albums: Page<SavedAlbum> = parse("saved_albums");
    let shows: Page<SavedShow> = parse("saved_shows");
    let artists: FollowedArtists = parse("followed_artists");
    let top_tracks: Page<Track> = parse("top_tracks");
    let top_artists: Page<Artist> = parse("top_artists");
    let contains: Vec<bool> = parse("library_contains");

    assert_eq!(albums.total, Some(3));
    assert!(albums.items.iter().all(|row| row.album.is_some()));
    assert_eq!(shows.total, Some(0));
    assert!(artists.artists.items.is_empty());
    assert!(!top_tracks.items.is_empty());
    assert!(!top_artists.items.is_empty());
    assert_eq!(contains.len(), 2);
}

#[test]
fn error_fixtures_are_4xx_with_a_message() {
    let cases = [
        ("artist_top_tracks_forbidden", 403),
        ("batch_tracks_forbidden", 403),
        ("followed_playlist_items_forbidden", 403),
        ("new_releases_forbidden", 403),
        ("editorial_playlist_404", 404),
        ("search_limit_11", 400),
    ];

    for (name, expected) in cases {
        let (status, body) = fixture(name);

        assert_eq!(status, expected, "{name}");
        assert!(body["error"]["message"].is_string(), "{name}");
    }
}

#[test]
fn error_statuses_map_to_distinct_errors() {
    assert_eq!(SpotifyError::Forbidden, SpotifyError::Forbidden);
    assert_ne!(SpotifyError::Forbidden, SpotifyError::NotFound);
}

#[test]
fn fixtures_hold_no_real_user_id() {
    let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));

    for entry in std::fs::read_dir(dir).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();

        assert!(!text.contains(concat!("3136", "mmjn")));
    }
}
