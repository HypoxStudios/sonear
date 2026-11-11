use super::*;

const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

#[test]
fn tracks_and_artists_make_stations() {
    assert_eq!(
        station_uri(&format!("spotify:track:{ID}")),
        Some(format!("spotify:station:track:{ID}"))
    );
    assert_eq!(
        station_uri(&format!("spotify:artist:{ID}")),
        Some(format!("spotify:station:artist:{ID}"))
    );
}

#[test]
fn other_kinds_and_bad_ids_make_none() {
    for uri in [
        format!("spotify:album:{ID}"),
        format!("spotify:playlist:{ID}"),
        format!("spotify:episode:{ID}"),
        "spotify:local:a:b:c:1".to_owned(),
        "spotify:track:".to_owned(),
        "spotify:track:a/../b".to_owned(),
        format!("spotify:track:{ID}:extra"),
        String::new(),
    ] {
        assert_eq!(station_uri(&uri), None, "{uri}");
    }
}

#[test]
fn seed_is_the_inverse() {
    for seed in [
        format!("spotify:track:{ID}"),
        format!("spotify:artist:{ID}"),
    ] {
        let station = station_uri(&seed).unwrap_or_default();

        assert!(is_station(&station));
        assert_eq!(station_seed(&station), Some(seed));
    }
}

#[test]
fn non_stations_have_no_seed() {
    assert_eq!(station_seed(&format!("spotify:track:{ID}")), None);
    assert_eq!(station_seed("spotify:station:album:x"), None);
    assert_eq!(station_seed("spotify:station:track:"), None);
    assert!(!is_station(""));
}

#[test]
fn youtube_tracks_make_song_radio_only() {
    assert_eq!(
        station_uri("ytm:track:IluRBvnY_-Y").as_deref(),
        Some("ytm:station:track:IluRBvnY_-Y")
    );
    assert_eq!(
        station_seed("ytm:station:track:IluRBvnY_-Y").as_deref(),
        Some("ytm:track:IluRBvnY_-Y")
    );
    assert!(is_station("ytm:station:track:IluRBvnYMoY"));
    assert_eq!(station_uri("ytm:artist:UCRr1xG_2WIDs18a6cIiCxeA"), None);
    assert_eq!(station_uri("ytm:album:MPREb_x"), None);
    assert_eq!(station_uri("ytm:track:a/b"), None);
    assert_eq!(station_seed("ytm:station:artist:UCx"), None);
}
