//! YouTube Music models to the domain types the UI uses, and tracks to the
//! Soloist entities the player events carry.

use sonear_soloist::protocol::{
    Cover, Creator, Decorations, Entity, Identity, Parent, PlaybackInfo, VisualIdentity,
};
use sonear_youtube::innertube::thumbs::variants;
use sonear_youtube::models as yt;

use crate::model::{
    Album, AlbumRef, ArtImage, Artist, ArtistRef, Artwork, ItemKind, Playlist, Track, open_url,
};

pub fn track_uri(video_id: &str) -> String {
    format!("ytm:track:{video_id}")
}

pub fn album_uri(browse_id: &str) -> String {
    format!("ytm:album:{browse_id}")
}

pub fn artist_uri(channel_id: &str) -> String {
    format!("ytm:artist:{channel_id}")
}

pub fn playlist_uri(playlist_id: &str) -> String {
    format!("ytm:playlist:{playlist_id}")
}

pub fn artwork(thumbs: &[yt::Thumb]) -> Artwork {
    Artwork(
        variants(thumbs)
            .into_iter()
            .map(|thumb| ArtImage {
                url: thumb.url,
                width: thumb.width,
            })
            .collect(),
    )
}

fn artist_ref(artist: &yt::ArtistRef) -> ArtistRef {
    ArtistRef {
        id: artist.id.clone(),
        name: artist.name.clone(),
    }
}

pub fn track(song: &yt::Song) -> Track {
    let uri = track_uri(&song.video_id);

    Track {
        id: Some(song.video_id.clone()),
        name: song.title.clone(),
        artists: song.artists.iter().map(artist_ref).collect(),
        album: song.album.as_ref().map(|album| AlbumRef {
            id: Some(album.id.clone()),
            uri: album_uri(&album.id),
            name: album.name.clone(),
        }),
        art: artwork(&song.thumbs),
        duration_ms: song.duration_ms.unwrap_or(0),
        is_local: false,
        playable: true,
        kind: ItemKind::Track,
        open_url: open_url(&uri),
        uri,
    }
}

pub fn album(card: &yt::AlbumCard, total_tracks: u32) -> Album {
    let uri = album_uri(&card.browse_id);

    Album {
        id: card.browse_id.clone(),
        name: card.title.clone(),
        artists: card.artists.iter().map(artist_ref).collect(),
        art: artwork(&card.thumbs),
        release_date: card.year.clone(),
        total_tracks,
        album_type: card.kind.to_lowercase(),
        open_url: open_url(&uri),
        uri,
    }
}

pub fn artist(card: &yt::ArtistCard) -> Artist {
    let uri = artist_uri(&card.channel_id);

    Artist {
        id: card.channel_id.clone(),
        name: card.name.clone(),
        art: artwork(&card.thumbs),
        open_url: open_url(&uri),
        uri,
    }
}

pub fn playlist(card: &yt::PlaylistCard, description: &str) -> Playlist {
    let uri = playlist_uri(&card.playlist_id);

    Playlist {
        id: card.playlist_id.clone(),
        name: card.title.clone(),
        description: description.to_owned(),
        art: artwork(&card.thumbs),
        owner_id: None,
        owner_name: card.author.clone(),
        collaborative: false,
        track_count: card.track_count,
        open_url: open_url(&uri),
        uri,
    }
}

/// Small, default and large covers picked from the sizes `art` has.
fn covers(art: &Artwork) -> VisualIdentity {
    let mut images: Vec<&ArtImage> = art.0.iter().collect();

    images.sort_by_key(|image| image.width.unwrap_or(0));

    let (Some(smallest), Some(largest)) = (images.first(), images.last()) else {
        return VisualIdentity::default();
    };
    let middle = images
        .iter()
        .find(|image| image.width.is_some_and(|width| width >= 300))
        .unwrap_or(largest);

    VisualIdentity {
        cover: [("small", smallest), ("default", middle), ("large", largest)]
            .into_iter()
            .map(|(size, image)| Cover {
                url: image.url.clone(),
                size: size.into(),
            })
            .collect(),
    }
}

fn named(uri: &str, kind: &str, name: &str, art: &Artwork) -> Entity {
    Entity {
        uri: uri.into(),
        entity_type: kind.into(),
        decorations: Decorations {
            identity: Identity { name: name.into() },
            visual_identity: covers(art),
            ..Decorations::default()
        },
    }
}

/// The entity a player event carries for `track`. `duration_ms` fills in a
/// length the track does not have.
pub fn track_entity(track: &Track, duration_ms: Option<u64>) -> Entity {
    let artists = track
        .artists
        .iter()
        .map(|artist| Creator {
            entity: Some(named(
                &artist.id.as_deref().map(artist_uri).unwrap_or_default(),
                "artist",
                &artist.name,
                &Artwork::default(),
            )),
        })
        .collect();
    let album = track
        .album
        .as_ref()
        .map(|album| Box::new(named(&album.uri, "album", &album.name, &track.art)));
    let duration = Some(track.duration_ms)
        .filter(|duration| *duration > 0)
        .or(duration_ms);

    Entity {
        uri: track.uri.clone(),
        entity_type: "track".into(),
        decorations: Decorations {
            identity: Identity {
                name: track.name.clone(),
            },
            visual_identity: covers(&track.art),
            creators: artists,
            parent: album.map(|entity| Parent {
                entity: Some(entity),
            }),
            playback: duration.map(|duration_ms| PlaybackInfo {
                duration_ms: Some(duration_ms),
            }),
        },
    }
}

/// The entity for the playing context.
pub fn context_entity(uri: &str, kind: &str, name: &str, art: &Artwork) -> Entity {
    named(uri, kind, name, art)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::entity_track;

    fn song() -> yt::Song {
        yt::Song {
            video_id: "IluRBvnYMoY".into(),
            title: "Give Life Back to Music".into(),
            artists: vec![
                yt::ArtistRef {
                    id: Some("UCRr1xG_2WIDs18a6cIiCxeA".into()),
                    name: "Daft Punk".into(),
                },
                yt::ArtistRef {
                    id: None,
                    name: "Someone".into(),
                },
            ],
            album: Some(yt::AlbumRef {
                id: "MPREb_K8qWMWVqXGi".into(),
                name: "Random Access Memories".into(),
            }),
            duration_ms: Some(276_000),
            thumbs: vec![yt::Thumb {
                url: "https://lh3.googleusercontent.com/abc=w60-h60-l90-rj".into(),
                width: Some(60),
            }],
            explicit: false,
        }
    }

    #[test]
    fn a_song_becomes_a_track_with_a_ytm_uri() {
        let track = track(&song());

        assert_eq!(track.uri, "ytm:track:IluRBvnYMoY");
        assert_eq!(track.id.as_deref(), Some("IluRBvnYMoY"));
        assert_eq!(track.duration_ms, 276_000);
        assert!(track.playable && !track.is_local);
        assert_eq!(
            track.open_url.as_deref(),
            Some("https://music.youtube.com/watch?v=IluRBvnYMoY")
        );
        assert_eq!(
            track.album.as_ref().unwrap().uri,
            "ytm:album:MPREb_K8qWMWVqXGi"
        );
        assert_eq!(
            track
                .art
                .0
                .iter()
                .map(|image| image.width)
                .collect::<Vec<_>>(),
            [Some(120), Some(300), Some(544)]
        );
    }

    #[test]
    fn an_unknown_length_is_zero() {
        let mut song = song();

        song.duration_ms = None;

        assert_eq!(track(&song).duration_ms, 0);
    }

    #[test]
    fn cards_convert_with_their_uris() {
        let album = album(
            &yt::AlbumCard {
                browse_id: "MPREb_x".into(),
                title: "Discovery".into(),
                artists: vec![],
                year: Some("2001".into()),
                kind: "Single".into(),
                thumbs: vec![],
            },
            13,
        );

        assert_eq!(album.uri, "ytm:album:MPREb_x");
        assert_eq!(album.album_type, "single");
        assert_eq!(album.year(), Some("2001"));
        assert_eq!(album.total_tracks, 13);

        let artist = artist(&yt::ArtistCard {
            channel_id: "UCx".into(),
            name: "A".into(),
            thumbs: vec![],
        });

        assert_eq!(
            artist.open_url.as_deref(),
            Some("https://music.youtube.com/channel/UCx")
        );

        let playlist = playlist(
            &yt::PlaylistCard {
                playlist_id: "PLx".into(),
                title: "P".into(),
                author: Some("Me".into()),
                track_count: Some(4),
                thumbs: vec![],
            },
            "about",
        );

        assert_eq!(playlist.uri, "ytm:playlist:PLx");
        assert_eq!(playlist.owner_name.as_deref(), Some("Me"));
        assert_eq!(playlist.owner_id, None);
        assert_eq!(playlist.description, "about");
    }

    #[test]
    fn a_track_survives_the_trip_through_an_entity() {
        let original = track(&song());
        let back = entity_track(&track_entity(&original, None));

        assert_eq!(back.uri, original.uri);
        assert_eq!(back.id, original.id);
        assert_eq!(back.name, original.name);
        assert_eq!(back.duration_ms, 276_000);
        assert_eq!(back.open_url, original.open_url);
        assert_eq!(back.album, original.album);
        assert_eq!(
            back.artists,
            [
                ArtistRef {
                    id: Some("UCRr1xG_2WIDs18a6cIiCxeA".into()),
                    name: "Daft Punk".into(),
                },
                ArtistRef {
                    id: None,
                    name: "Someone".into(),
                }
            ]
        );
        assert_eq!(back.art.url_for(300), original.art.url_for(300));
        assert_eq!(back.art.url_for(600), original.art.url_for(600));
        assert!(back.playable);
    }

    #[test]
    fn covers_map_to_the_three_sizes() {
        let entity = track_entity(&track(&song()), None);

        assert_eq!(
            entity.cover_url("small"),
            Some("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj")
        );
        assert_eq!(
            entity.cover_url("default"),
            Some("https://lh3.googleusercontent.com/abc=w300-h300-l90-rj")
        );
        assert_eq!(
            entity.cover_url("large"),
            Some("https://lh3.googleusercontent.com/abc=w544-h544-l90-rj")
        );
    }

    #[test]
    fn a_missing_length_can_come_from_the_player() {
        let mut bare = track(&song());

        bare.duration_ms = 0;

        assert_eq!(track_entity(&bare, Some(5_000)).duration_ms(), Some(5_000));
        assert_eq!(track_entity(&bare, None).duration_ms(), None);
    }

    #[test]
    fn context_entities_carry_their_kind() {
        let entity = context_entity("ytm:album:x", "album", "Name", &Artwork::default());

        assert_eq!(entity.entity_type, "album");
        assert_eq!(entity.name(), "Name");
    }
}
