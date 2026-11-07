//! The fake music catalogue behind demo mode.

use std::sync::OnceLock;

use crate::model::{
    Album, AlbumRef, Artist, ArtistRef, Artwork, Episode, ItemKind, Playlist, Show, Track,
};

pub const PLAYLIST_URI: &str = "spotify:playlist:demo-night-drive";
pub const DEMO_USER: &str = "demo-user";
pub const NOT_OWNED_ID: &str = "demo-weekend-radio";

const NAMED_TRACKS: [(&str, &str, u64); 8] = [
    ("Midnight City", "The Paper Skies", 222),
    ("Afterglow", "The Paper Skies", 258),
    ("Undertow", "Soliste", 189),
    ("Stay Awake", "Northbound", 236),
    ("Satellites", "Northbound", 222),
    ("Solstice", "Soliste", 258),
    ("Moonlight", "Northbound", 189),
    ("Parallel", "The Paper Skies", 236),
];

const FILLER_TITLES: [&str; 24] = [
    "Neon Hours",
    "Glass Highway",
    "Low Tide",
    "Afterimage",
    "Slow Burn",
    "Tunnel Vision",
    "Paper Lanterns",
    "Static Bloom",
    "Vanishing Point",
    "Harbor Lights",
    "Night Shift",
    "Orbit",
    "Cold Coffee",
    "Rearview",
    "Signal Fade",
    "Dusk Protocol",
    "Ember",
    "Ghost Lane",
    "Blue Hour",
    "Wavelength",
    "Last Exit",
    "Halcyon",
    "Slipstream",
    "Pale Fire",
];

const ADJECTIVES: [&str; 12] = [
    "Quiet", "Golden", "Distant", "Velvet", "Hollow", "Electric", "Paper", "Midnight", "Silver",
    "Faded", "Open", "Restless",
];
const NOUNS: [&str; 12] = [
    "Rooms", "Tides", "Streets", "Signals", "Gardens", "Echoes", "Windows", "Engines", "Shores",
    "Letters", "Colors", "Hours",
];

pub const ARTISTS: [&str; 5] = [
    "The Paper Skies",
    "Soliste",
    "Northbound",
    "Lumen Park",
    "Hollow Vale",
];
const FILLER_SECONDS: [u64; 8] = [250, 281, 243, 297, 262, 310, 236, 275];
const ALBUM_NAMES: [&str; 12] = [
    "Night Drive",
    "Afterglow Sessions",
    "Glass Highway",
    "Low Tide",
    "Paper Lanterns",
    "Static Bloom",
    "Harbor Lights",
    "Blue Hour",
    "Halcyon",
    "Slipstream",
    "Pale Fire",
    "Signal Fade",
];

pub const POOL_SIZE: usize = 120;
pub const NIGHT_DRIVE_TRACKS: usize = 32;
const TRACKS_PER_ALBUM: usize = 10;

#[derive(Debug, Clone)]
pub struct DemoTrack {
    pub index: usize,
    pub uri: String,
    pub title: String,
    pub artist: &'static str,
    pub artist_index: usize,
    pub album_index: usize,
    pub duration_ms: u64,
}

impl DemoTrack {
    pub fn album_uri(&self) -> String {
        album_uri(self.album_index)
    }

    pub fn artist_uri(&self) -> String {
        artist_uri(self.artist_index)
    }

    pub fn model(&self) -> Track {
        Track {
            uri: self.uri.clone(),
            id: Some(format!("demo{:02}", self.index)),
            name: self.title.clone(),
            artists: vec![ArtistRef {
                id: Some(artist_id(self.artist_index)),
                name: self.artist.to_owned(),
            }],
            album: Some(AlbumRef {
                id: Some(album_id(self.album_index)),
                uri: self.album_uri(),
                name: ALBUM_NAMES[self.album_index].to_owned(),
            }),
            art: Artwork::default(),
            duration_ms: self.duration_ms,
            is_local: false,
            playable: true,
            kind: ItemKind::Track,
            open_url: Some(format!(
                "https://open.spotify.com/track/demo{:02}",
                self.index
            )),
        }
    }
}

pub fn album_id(index: usize) -> String {
    format!("demo-album-{index}")
}

pub fn album_uri(index: usize) -> String {
    format!("spotify:album:{}", album_id(index))
}

pub fn artist_id(index: usize) -> String {
    format!("demo-artist-{index}")
}

pub fn artist_uri(index: usize) -> String {
    format!("spotify:artist:{}", artist_id(index))
}

#[derive(Debug, Clone)]
pub struct DemoPlaylist {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub start: usize,
    pub count: usize,
    pub owner: &'static str,
}

impl DemoPlaylist {
    pub fn uri(&self) -> String {
        format!("spotify:playlist:{}", self.id)
    }

    pub fn owned(&self) -> bool {
        self.id != NOT_OWNED_ID
    }

    pub fn indices(&self) -> Vec<usize> {
        (self.start..self.start + self.count).collect()
    }

    pub fn model(&self) -> Playlist {
        Playlist {
            id: self.id.to_owned(),
            uri: self.uri(),
            name: self.name.to_owned(),
            description: self.description.to_owned(),
            art: Artwork::default(),
            owner_id: Some(if self.owned() {
                DEMO_USER.to_owned()
            } else {
                "echo-station".to_owned()
            }),
            owner_name: Some(self.owner.to_owned()),
            collaborative: false,
            track_count: Some(self.count as u32),
            open_url: Some(format!("https://open.spotify.com/playlist/{}", self.id)),
        }
    }
}

pub struct Catalog {
    pub tracks: Vec<DemoTrack>,
    pub playlists: Vec<DemoPlaylist>,
}

pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();

    CATALOG.get_or_init(build)
}

fn build() -> Catalog {
    let named = NAMED_TRACKS
        .iter()
        .map(|(title, artist, seconds)| ((*title).to_owned(), *artist, *seconds));
    let filler = FILLER_TITLES.iter().enumerate().map(|(index, title)| {
        (
            (*title).to_owned(),
            ARTISTS[(index + 3) % ARTISTS.len()],
            FILLER_SECONDS[index % FILLER_SECONDS.len()],
        )
    });
    let generated = (0..POOL_SIZE - NIGHT_DRIVE_TRACKS).map(|index| {
        (
            format!(
                "{} {}",
                ADJECTIVES[index % ADJECTIVES.len()],
                NOUNS[(index * 5 + index / 12) % NOUNS.len()]
            ),
            ARTISTS[(index * 2 + 1) % ARTISTS.len()],
            FILLER_SECONDS[(index + 3) % FILLER_SECONDS.len()],
        )
    });
    let tracks = named
        .chain(filler)
        .chain(generated)
        .enumerate()
        .map(|(index, (title, artist, seconds))| DemoTrack {
            index,
            uri: format!("spotify:track:demo{index:02}"),
            title,
            artist,
            artist_index: ARTISTS.iter().position(|name| *name == artist).unwrap_or(0),
            album_index: index / TRACKS_PER_ALBUM,
            duration_ms: seconds * 1000,
        })
        .collect();
    let playlist = |id, name, description, start, count, owner| DemoPlaylist {
        id,
        name,
        description,
        start,
        count,
        owner,
    };

    Catalog {
        tracks,
        playlists: vec![
            playlist(
                "demo-night-drive",
                "Night Drive",
                "A soundtrack for the hours after dark.",
                0,
                NIGHT_DRIVE_TRACKS,
                "Hypox",
            ),
            playlist(
                "demo-on-repeat",
                "On Repeat",
                "The songs you keep coming back to.",
                8,
                30,
                "Hypox",
            ),
            playlist(
                "demo-favorites",
                "Favorites",
                "Everything that stuck.",
                0,
                POOL_SIZE - 2,
                "Hypox",
            ),
            playlist(
                "demo-late-night-coding",
                "Late Night Coding",
                "Steady beats for long sessions.",
                20,
                64,
                "Hypox",
            ),
            playlist(
                "demo-sunday-morning",
                "Sunday Morning",
                "Slow starts.",
                40,
                41,
                "Hypox",
            ),
            playlist(
                NOT_OWNED_ID,
                "Weekend Radio",
                "A playlist someone else curates.",
                50,
                40,
                "Echo Station",
            ),
        ],
    }
}

impl Catalog {
    pub fn track(&self, uri: &str) -> Option<&DemoTrack> {
        self.tracks.iter().find(|track| track.uri == uri)
    }

    pub fn playlist(&self, id: &str) -> Option<&DemoPlaylist> {
        self.playlists.iter().find(|playlist| playlist.id == id)
    }

    pub fn album_count(&self) -> usize {
        self.tracks.len().div_ceil(TRACKS_PER_ALBUM)
    }

    pub fn album_indices(&self, album: usize) -> Vec<usize> {
        self.tracks
            .iter()
            .filter(|track| track.album_index == album)
            .map(|track| track.index)
            .collect()
    }

    pub fn artist_indices(&self, artist: usize) -> Vec<usize> {
        self.tracks
            .iter()
            .filter(|track| track.artist_index == artist)
            .map(|track| track.index)
            .collect()
    }

    pub fn album_model(&self, album: usize) -> Album {
        let tracks = self.album_indices(album);
        let first = tracks.first().map(|index| &self.tracks[*index]);

        Album {
            id: album_id(album),
            uri: album_uri(album),
            name: ALBUM_NAMES[album].to_owned(),
            artists: first
                .map(|track| {
                    vec![ArtistRef {
                        id: Some(artist_id(track.artist_index)),
                        name: track.artist.to_owned(),
                    }]
                })
                .unwrap_or_default(),
            art: Artwork::default(),
            release_date: Some(format!("{}-03-14", 2019 + album)),
            total_tracks: tracks.len() as u32,
            album_type: "album".to_owned(),
            open_url: Some(format!(
                "https://open.spotify.com/album/{}",
                album_id(album)
            )),
        }
    }

    pub fn artist_model(&self, artist: usize) -> Artist {
        Artist {
            id: artist_id(artist),
            uri: artist_uri(artist),
            name: ARTISTS[artist].to_owned(),
            art: Artwork::default(),
            open_url: Some(format!(
                "https://open.spotify.com/artist/{}",
                artist_id(artist)
            )),
        }
    }

    /// Track indices of an album, playlist or artist context URI.
    pub fn context_indices(&self, uri: &str) -> Option<Vec<usize>> {
        let (kind, id) = crate::model::split_uri(uri)?;

        match kind {
            "playlist" => self.playlist(id).map(DemoPlaylist::indices),
            "album" => {
                let index = id.strip_prefix("demo-album-")?.parse().ok()?;

                (index < self.album_count()).then(|| self.album_indices(index))
            }
            "artist" => {
                let index = id.strip_prefix("demo-artist-")?.parse().ok()?;

                (index < ARTISTS.len()).then(|| self.artist_indices(index))
            }
            _ => None,
        }
    }
}

pub fn shows() -> Vec<Show> {
    vec![
        Show {
            id: "demo-show-0".into(),
            uri: "spotify:show:demo-show-0".into(),
            name: "Late Night Tech".into(),
            publisher: "Echo Station".into(),
            description: "Conversations about the software behind the music.".into(),
            art: Artwork::default(),
            episode_count: Some(6),
            open_url: Some("https://open.spotify.com/show/demo-show-0".into()),
        },
        Show {
            id: "demo-show-1".into(),
            uri: "spotify:show:demo-show-1".into(),
            name: "Quiet Hours".into(),
            publisher: "Northbound Audio".into(),
            description: "Short essays for the end of the day.".into(),
            art: Artwork::default(),
            episode_count: Some(4),
            open_url: Some("https://open.spotify.com/show/demo-show-1".into()),
        },
    ]
}

pub fn episodes(show: &Show) -> Vec<Episode> {
    (0..show.episode_count.unwrap_or(0))
        .map(|index| Episode {
            id: format!("{}-ep{index}", show.id),
            uri: format!("spotify:episode:{}-ep{index}", show.id),
            name: format!("Episode {}: {}", index + 1, ADJECTIVES[index as usize % 12]),
            description: "A short demo episode.".into(),
            art: Artwork::default(),
            duration_ms: (1_200 + u64::from(index) * 137) * 1000,
            release_date: Some(format!("2026-10-{:02}", 28 - index)),
            show_name: Some(show.name.clone()),
            open_url: None,
        })
        .collect()
}
