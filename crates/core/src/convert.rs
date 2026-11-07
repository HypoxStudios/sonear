//! Wire types to domain types.

use sonear_soloist::protocol::Entity;
use sonear_spotify::models as wire;

use crate::model::{
    Album, AlbumRef, ArtImage, Artist, ArtistRef, Artwork, Device, Episode, ItemKind, Page,
    Playlist, Profile, Show, Track, split_uri,
};

fn art(images: &[wire::Image]) -> Artwork {
    Artwork(
        images
            .iter()
            .map(|image| ArtImage {
                url: image.url.clone(),
                width: image.width.or(image.height),
            })
            .collect(),
    )
}

fn artist_ref(artist: &wire::Artist) -> ArtistRef {
    ArtistRef {
        id: artist.id.clone(),
        name: artist.name.clone(),
    }
}

pub fn page<W, T>(wire: wire::Page<W>, offset: u32, convert: impl FnMut(W) -> Option<T>) -> Page<T>
where
    W: serde::de::DeserializeOwned,
{
    let received = wire.items.len() as u32;
    let items: Vec<T> = wire.items.into_iter().filter_map(convert).collect();
    let offset = wire.offset.unwrap_or(offset);
    let has_more = wire.next.is_some()
        || wire
            .total
            .is_some_and(|total| offset + received < total && received > 0);

    Page {
        items,
        total: wire.total,
        offset,
        has_more,
        cursor: None,
    }
}

/// `album` supplies the cover and album link for tracks that come without one.
pub fn track(track: &wire::Track, album: Option<&wire::Album>) -> Track {
    let album = track.album.as_ref().or(album);

    Track {
        uri: track.uri.clone(),
        id: track.id.clone(),
        name: track.name.clone(),
        artists: track.artists.iter().map(artist_ref).collect(),
        album: album.map(|album| AlbumRef {
            id: album.id.clone(),
            uri: album.uri.clone(),
            name: album.name.clone(),
        }),
        art: album.map(|album| art(&album.images)).unwrap_or_default(),
        duration_ms: track.duration_ms,
        is_local: track.is_local,
        playable: track.is_playable.unwrap_or(true),
        kind: ItemKind::Track,
        open_url: track.external_urls.spotify.clone(),
    }
}

pub fn album(album: &wire::Album) -> Option<Album> {
    Some(Album {
        id: album.id.clone()?,
        uri: album.uri.clone(),
        name: album.name.clone(),
        artists: album.artists.iter().map(artist_ref).collect(),
        art: art(&album.images),
        release_date: album.release_date.clone(),
        total_tracks: album.total_tracks.unwrap_or(0),
        album_type: album.album_type.clone().unwrap_or_default(),
        open_url: album.external_urls.spotify.clone(),
    })
}

pub fn artist(artist: &wire::Artist) -> Option<Artist> {
    Some(Artist {
        id: artist.id.clone()?,
        uri: artist.uri.clone(),
        name: artist.name.clone(),
        art: art(&artist.images),
        open_url: artist.external_urls.spotify.clone(),
    })
}

pub fn playlist(playlist: &wire::Playlist) -> Option<Playlist> {
    Some(Playlist {
        id: playlist.id.clone()?,
        uri: playlist.uri.clone(),
        name: playlist.name.clone(),
        description: playlist.description.clone(),
        art: art(&playlist.images),
        owner_id: playlist.owner.as_ref().and_then(|owner| owner.id.clone()),
        owner_name: playlist
            .owner
            .as_ref()
            .and_then(|owner| owner.display_name.clone()),
        collaborative: playlist.collaborative,
        track_count: playlist.total(),
        open_url: playlist.external_urls.spotify.clone(),
    })
}

pub fn show(show: &wire::Show) -> Option<Show> {
    Some(Show {
        id: show.id.clone()?,
        uri: show.uri.clone(),
        name: show.name.clone(),
        publisher: show.publisher.clone().unwrap_or_default(),
        description: show.description.clone().unwrap_or_default(),
        art: art(&show.images),
        episode_count: show.total_episodes,
        open_url: show.external_urls.spotify.clone(),
    })
}

pub fn episode(episode: &wire::Episode, show: Option<&wire::Show>) -> Option<Episode> {
    let show = episode.show.as_ref().or(show);
    let images = if episode.images.is_empty() {
        show.map(|show| show.images.as_slice()).unwrap_or_default()
    } else {
        episode.images.as_slice()
    };

    Some(Episode {
        id: episode.id.clone()?,
        uri: episode.uri.clone(),
        name: episode.name.clone(),
        description: episode.description.clone().unwrap_or_default(),
        art: art(images),
        duration_ms: episode.duration_ms,
        release_date: episode.release_date.clone(),
        show_name: show.map(|show| show.name.clone()),
        open_url: episode.external_urls.spotify.clone(),
    })
}

pub fn media_item(item: &wire::MediaItem) -> Option<Track> {
    match item {
        wire::MediaItem::Track(wire_track) => Some(track(wire_track, None)),
        wire::MediaItem::Episode(wire_episode) => episode(wire_episode, None).map(Track::from),
    }
}

pub fn profile(user: &wire::User) -> Profile {
    Profile {
        id: user.id.clone().unwrap_or_default(),
        display_name: user
            .display_name
            .clone()
            .or_else(|| user.id.clone())
            .unwrap_or_default(),
        art: art(&user.images),
    }
}

pub fn device(device: &wire::Device) -> Option<Device> {
    Some(Device {
        id: device.id.clone()?,
        name: device.name.clone(),
        active: device.is_active,
    })
}

fn cover_width(size: &str) -> Option<u32> {
    match size {
        "small" => Some(64),
        "default" => Some(300),
        "large" => Some(640),
        "xlarge" => Some(1280),
        _ => None,
    }
}

/// A track or episode as Soloist describes it. `duration_ms` is 0 when the
/// entity carries no length.
pub fn entity_track(entity: &Entity) -> Track {
    let (kind, id) = split_uri(&entity.uri).unzip();
    let album = entity.album().map(|album| AlbumRef {
        id: split_uri(&album.uri).map(|(_, id)| id.to_owned()),
        uri: album.uri.clone(),
        name: album.name().to_owned(),
    });
    let open_url = kind
        .zip(id)
        .map(|(kind, id)| format!("https://open.spotify.com/{kind}/{id}"));

    Track {
        uri: entity.uri.clone(),
        id: id.map(str::to_owned),
        name: entity.name().to_owned(),
        artists: entity
            .decorations
            .creators
            .iter()
            .filter_map(|creator| creator.entity.as_ref())
            .filter(|artist| !artist.name().is_empty())
            .map(|artist| ArtistRef {
                id: split_uri(&artist.uri).map(|(_, id)| id.to_owned()),
                name: artist.name().to_owned(),
            })
            .collect(),
        album,
        art: Artwork(
            entity
                .decorations
                .visual_identity
                .cover
                .iter()
                .map(|cover| ArtImage {
                    url: cover.url.clone(),
                    width: cover_width(&cover.size),
                })
                .collect(),
        ),
        duration_ms: entity.duration_ms().unwrap_or(0),
        is_local: kind == Some("local"),
        playable: true,
        kind: if entity.entity_type == "episode" || kind == Some("episode") {
            ItemKind::Episode
        } else {
            ItemKind::Track
        },
        open_url,
    }
}

#[cfg(test)]
mod tests {
    use sonear_soloist::protocol::{
        Cover, Creator, Decorations, Identity, Parent, PlaybackInfo, VisualIdentity,
    };

    use super::*;

    fn named(uri: &str, kind: &str, name: &str) -> Entity {
        Entity {
            uri: uri.into(),
            entity_type: kind.into(),
            decorations: Decorations {
                identity: Identity { name: name.into() },
                ..Decorations::default()
            },
        }
    }

    #[test]
    fn an_entity_becomes_a_track() {
        let mut entity = named("spotify:track:t1", "track", "Song");

        entity.decorations.creators = vec![Creator {
            entity: Some(named("spotify:artist:a1", "artist", "Band")),
        }];
        entity.decorations.parent = Some(Parent {
            entity: Some(Box::new(named("spotify:album:al1", "album", "Record"))),
        });
        entity.decorations.visual_identity = VisualIdentity {
            cover: vec![Cover {
                url: "https://img/1".into(),
                size: "large".into(),
            }],
        };
        entity.decorations.playback = Some(PlaybackInfo {
            duration_ms: Some(1234),
        });

        let track = entity_track(&entity);

        assert_eq!(track.id.as_deref(), Some("t1"));
        assert_eq!(track.name, "Song");
        assert_eq!(track.artist_line(), "Band");
        assert_eq!(track.artists[0].id.as_deref(), Some("a1"));
        assert_eq!(track.album.as_ref().unwrap().name, "Record");
        assert_eq!(track.album.as_ref().unwrap().id.as_deref(), Some("al1"));
        assert_eq!(track.art.0[0].width, Some(640));
        assert_eq!(track.duration_ms, 1234);
        assert!(track.playable);
        assert_eq!(track.kind, ItemKind::Track);
        assert_eq!(
            track.open_url.as_deref(),
            Some("https://open.spotify.com/track/t1")
        );
    }

    #[test]
    fn a_bare_entity_converts_without_a_length() {
        let track = entity_track(&named("spotify:episode:e1", "episode", "Talk"));

        assert_eq!(track.duration_ms, 0);
        assert!(track.album.is_none());
        assert!(track.artists.is_empty());
        assert_eq!(track.kind, ItemKind::Episode);
    }
}
