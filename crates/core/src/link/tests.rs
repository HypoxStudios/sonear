use super::*;

const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

fn parsed(text: &str) -> Option<String> {
    SpotifyLink::parse(text).map(|link| link.uri())
}

#[test]
fn uris_parse_for_every_kind() {
    for kind in ["track", "episode", "album", "playlist", "artist", "show"] {
        let uri = format!("spotify:{kind}:{ID}");

        assert_eq!(parsed(&uri), Some(uri));
    }
}

#[test]
fn legacy_user_playlist_uris_parse() {
    assert_eq!(
        parsed(&format!("spotify:user:someone:playlist:{ID}")),
        Some(format!("spotify:playlist:{ID}"))
    );
}

#[test]
fn urls_parse_with_or_without_scheme() {
    let want = Some(format!("spotify:album:{ID}"));

    assert_eq!(
        parsed(&format!("https://open.spotify.com/album/{ID}")),
        want
    );
    assert_eq!(parsed(&format!("http://open.spotify.com/album/{ID}")), want);
    assert_eq!(parsed(&format!("open.spotify.com/album/{ID}")), want);
    assert_eq!(
        parsed(&format!("HTTPS://Open.Spotify.com/album/{ID}")),
        want
    );
}

#[test]
fn intl_embed_user_and_trailing_parts_are_handled() {
    let want = Some(format!("spotify:playlist:{ID}"));
    let base = "https://open.spotify.com";

    assert_eq!(parsed(&format!("{base}/intl-de/playlist/{ID}")), want);
    assert_eq!(parsed(&format!("{base}/intl-pt-br/playlist/{ID}")), want);
    assert_eq!(parsed(&format!("{base}/embed/playlist/{ID}")), want);
    assert_eq!(parsed(&format!("{base}/intl-fr/embed/playlist/{ID}")), want);
    assert_eq!(parsed(&format!("{base}/user/someone/playlist/{ID}")), want);
    assert_eq!(parsed(&format!("{base}/playlist/{ID}/")), want);
}

#[test]
fn query_and_fragment_are_ignored() {
    let want = Some(format!("spotify:track:{ID}"));
    let base = "https://open.spotify.com/track";

    assert_eq!(parsed(&format!("{base}/{ID}?si=abcdef0123456789")), want);
    assert_eq!(
        parsed(&format!("{base}/{ID}?si=x&utm_source=copy-link#top")),
        want
    );
    assert_eq!(parsed(&format!("{base}/{ID}#frag")), want);
}

#[test]
fn uri_query_and_fragment_are_ignored() {
    let want = Some(format!("spotify:track:{ID}"));

    assert_eq!(parsed(&format!("spotify:track:{ID}?si=abc")), want);
    assert_eq!(
        parsed(&format!("spotify:track:{ID}?context=spotify:album:{ID}")),
        want
    );
    assert_eq!(parsed(&format!("spotify:track:{ID}#frag")), want);
    assert_eq!(parsed(&format!("spotify:track:{ID}?")), want);
}

#[test]
fn path_style_uris_parse() {
    let want = Some(format!("spotify:album:{ID}"));

    assert_eq!(parsed(&format!("spotify://album/{ID}")), want);
    assert_eq!(parsed(&format!("spotify:/album/{ID}")), want);
    assert_eq!(parsed(&format!("spotify://album/{ID}/")), want);
    assert_eq!(parsed(&format!("spotify://album/{ID}?si=x#y")), want);
    assert_eq!(parsed(&format!("SPOTIFY://album/{ID}")), want);
    assert_eq!(parsed(&format!("spotify://user/{ID}")), None);
    assert_eq!(parsed(&format!("spotify://album/{ID}/more")), None);
    assert_eq!(parsed(&format!("spotify://album/../{ID}")), None);
}

#[test]
fn surrounding_noise_is_trimmed() {
    let want = Some(format!("spotify:show:{ID}"));

    assert_eq!(parsed(&format!("  <spotify:show:{ID}>\n")), want);
    assert_eq!(
        parsed(&format!("\"https://open.spotify.com/show/{ID}\"")),
        want
    );
    assert_eq!(parsed(&format!("'open.spotify.com/show/{ID}'")), want);
}

#[test]
fn accessors_describe_the_link() {
    let link = SpotifyLink::parse(&format!("spotify:artist:{ID}")).unwrap();

    assert_eq!(link, SpotifyLink::Artist(ID.to_owned()));
    assert_eq!(link.id(), ID);
    assert_eq!(link.kind_label(), "Artist");
    assert_eq!(SpotifyLink::Track(ID.into()).kind_label(), "Track");
}

#[test]
fn other_hosts_are_rejected() {
    assert_eq!(parsed(&format!("https://evil.com/track/{ID}")), None);
    assert_eq!(
        parsed(&format!("https://open.spotify.com.evil.com/track/{ID}")),
        None
    );
    assert_eq!(
        parsed(&format!("https://evil.com/open.spotify.com/track/{ID}")),
        None
    );
    assert_eq!(
        parsed(&format!("https://open.spotify.com@evil.com/track/{ID}")),
        None
    );
    assert_eq!(parsed(&format!("https://spotify.link/{ID}")), None);
    assert_eq!(
        parsed(&format!("https://play.spotify.com/track/{ID}")),
        None
    );
    assert_eq!(parsed(&format!("ftp://open.spotify.com/track/{ID}")), None);
}

#[test]
fn other_kinds_are_rejected() {
    for kind in ["user", "collection", "local", "genre", "station"] {
        assert_eq!(parsed(&format!("spotify:{kind}:{ID}")), None);
        assert_eq!(
            parsed(&format!("https://open.spotify.com/{kind}/{ID}")),
            None
        );
    }
}

#[test]
fn bad_ids_are_rejected() {
    let short = &ID[..21];

    assert_eq!(parsed("spotify:track:../me"), None);
    assert_eq!(parsed(&format!("spotify:track:{short}?")), None);
    assert_eq!(parsed(&format!("spotify:track:{short}?si=x")), None);
    assert_eq!(parsed("spotify:track:../me?x"), None);
    assert_eq!(parsed(&format!("spotify://track/{short}")), None);
    assert_eq!(parsed(&format!("spotify:track:{short}")), None);
    assert_eq!(parsed(&format!("spotify:track:{ID}A")), None);
    assert_eq!(parsed(&format!("spotify:track:{short}-")), None);
    assert_eq!(parsed("spotify:track:"), None);
    assert_eq!(
        parsed(&format!("https://open.spotify.com/track/{short}")),
        None
    );
    assert_eq!(
        parsed(&format!("https://open.spotify.com/track/{ID}A")),
        None
    );
}

#[test]
fn extra_segments_are_rejected() {
    let base = "https://open.spotify.com";

    assert_eq!(parsed(&format!("{base}/track/{ID}/more")), None);
    assert_eq!(parsed(&format!("{base}/album/{ID}/track/{ID}")), None);
    assert_eq!(parsed(&format!("spotify:track:{ID}:more")), None);
    assert_eq!(parsed(&format!("spotify:user:me:album:{ID}")), None);
    assert_eq!(parsed(&format!("{base}/user//playlist/{ID}")), None);
    assert_eq!(parsed(&format!("{base}/intl-1/track/{ID}")), None);
}

#[test]
fn junk_and_oversized_input_is_rejected() {
    let long = "a".repeat(600);

    assert_eq!(parsed(""), None);
    assert_eq!(parsed("hello world"), None);
    assert_eq!(parsed("spotify:"), None);
    assert_eq!(parsed("https://open.spotify.com"), None);
    assert_eq!(
        parsed(&format!("https://open.spotify.com/track/{ID}?{long}")),
        None
    );
}

#[test]
fn is_spotify_id_wants_exactly_22_alphanumerics() {
    assert!(is_spotify_id(ID));
    assert!(!is_spotify_id(&ID[..21]));
    assert!(!is_spotify_id(&format!("{ID}x")));
    assert!(!is_spotify_id("4uLU6hMCjMI75M1A2tKU_C"));
    assert!(!is_spotify_id("4uLU6hMCjMI75M1A2tK\u{e9}\u{e9}"));
}
