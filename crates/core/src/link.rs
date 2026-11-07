//! Spotify links and URIs, parsed strictly: only the forms below are accepted,
//! and the ID is always checked before it can reach a request or a command.

const HOST: &str = "open.spotify.com";
const MAX_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpotifyLink {
    Track(String),
    Episode(String),
    Album(String),
    Playlist(String),
    Artist(String),
    Show(String),
}

/// Whether `id` is a Spotify ID: exactly 22 ASCII letters or digits.
pub fn is_spotify_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

impl SpotifyLink {
    /// Reads a `spotify:` URI or an open.spotify.com link. Short links
    /// (`spotify.link`) need a network lookup and are not accepted.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text
            .trim()
            .trim_matches(|c: char| matches!(c, '<' | '>' | '"' | '\''))
            .trim();

        if text.is_empty() || text.len() > MAX_LEN {
            return None;
        }

        if let Some(rest) = strip_prefix_ignore_case(text, "spotify:") {
            return parse_uri(rest);
        }

        parse_url(text)
    }

    pub fn uri(&self) -> String {
        format!("spotify:{}:{}", self.kind(), self.id())
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Track(id)
            | Self::Episode(id)
            | Self::Album(id)
            | Self::Playlist(id)
            | Self::Artist(id)
            | Self::Show(id) => id,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Track(_) => "Track",
            Self::Episode(_) => "Episode",
            Self::Album(_) => "Album",
            Self::Playlist(_) => "Playlist",
            Self::Artist(_) => "Artist",
            Self::Show(_) => "Show",
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Track(_) => "track",
            Self::Episode(_) => "episode",
            Self::Album(_) => "album",
            Self::Playlist(_) => "playlist",
            Self::Artist(_) => "artist",
            Self::Show(_) => "show",
        }
    }

    fn from_parts(kind: &str, id: &str) -> Option<Self> {
        if !is_spotify_id(id) {
            return None;
        }

        let id = id.to_owned();

        Some(match kind {
            "track" => Self::Track(id),
            "episode" => Self::Episode(id),
            "album" => Self::Album(id),
            "playlist" => Self::Playlist(id),
            "artist" => Self::Artist(id),
            "show" => Self::Show(id),
            _ => return None,
        })
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;

    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

fn parse_uri(rest: &str) -> Option<SpotifyLink> {
    let rest = rest.split(['?', '#']).next().unwrap_or_default();

    if let Some(path) = rest.strip_prefix("//").or_else(|| rest.strip_prefix('/')) {
        let path = path.strip_suffix('/').unwrap_or(path);

        return match path.split('/').collect::<Vec<_>>().as_slice() {
            [kind, id] => SpotifyLink::from_parts(kind, id),
            _ => None,
        };
    }

    let parts: Vec<&str> = rest.split(':').collect();

    match parts.as_slice() {
        [kind, id] => SpotifyLink::from_parts(kind, id),
        ["user", user, "playlist", id] if !user.is_empty() => {
            SpotifyLink::from_parts("playlist", id)
        }
        _ => None,
    }
}

fn parse_url(text: &str) -> Option<SpotifyLink> {
    let rest = strip_prefix_ignore_case(text, "https://")
        .or_else(|| strip_prefix_ignore_case(text, "http://"))
        .unwrap_or(text);
    let (host, path) = rest.split_once('/')?;

    if !host.eq_ignore_ascii_case(HOST) {
        return None;
    }

    let path = path.split(['?', '#']).next().unwrap_or_default();
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut segments: Vec<&str> = path.split('/').collect();

    if segments.first().is_some_and(|first| is_intl(first)) {
        segments.remove(0);
    }

    if segments.first() == Some(&"embed") {
        segments.remove(0);
    }

    match segments.as_slice() {
        [kind, id] => SpotifyLink::from_parts(kind, id),
        ["user", user, "playlist", id] if !user.is_empty() => {
            SpotifyLink::from_parts("playlist", id)
        }
        _ => None,
    }
}

fn is_intl(segment: &str) -> bool {
    segment.strip_prefix("intl-").is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
    })
}

#[cfg(test)]
mod tests;
