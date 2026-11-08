//! Spotify's own radio, reached through the undocumented
//! `spotify:station:<kind>:<id>` URIs that Soloist accepts in `play`.

const TRACK_PREFIX: &str = "spotify:track:";
const ARTIST_PREFIX: &str = "spotify:artist:";
const STATION_TRACK_PREFIX: &str = "spotify:station:track:";
const STATION_ARTIST_PREFIX: &str = "spotify:station:artist:";

/// The station URI seeded by a track or an artist, `None` for anything else.
pub fn station_uri(seed_uri: &str) -> Option<String> {
    if let Some(id) = seed_uri.strip_prefix(TRACK_PREFIX) {
        return valid_id(id).then(|| format!("{STATION_TRACK_PREFIX}{id}"));
    }

    let id = seed_uri.strip_prefix(ARTIST_PREFIX)?;

    valid_id(id).then(|| format!("{STATION_ARTIST_PREFIX}{id}"))
}

/// The seed URI of a station, `None` for anything that is not a station.
pub fn station_seed(uri: &str) -> Option<String> {
    if let Some(id) = uri.strip_prefix(STATION_TRACK_PREFIX) {
        return valid_id(id).then(|| format!("{TRACK_PREFIX}{id}"));
    }

    let id = uri.strip_prefix(STATION_ARTIST_PREFIX)?;

    valid_id(id).then(|| format!("{ARTIST_PREFIX}{id}"))
}

pub fn is_station(uri: &str) -> bool {
    station_seed(uri).is_some()
}

/// Looser than a real Spotify ID (22 base62 characters) because demo IDs are
/// short and hyphenated.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests;
