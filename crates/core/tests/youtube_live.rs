//! Plays real YouTube Music audio through the real catalog, yt-dlp and mpv
//! (muted with `--ao=null`). Needs the network and a folder named by
//! `SONEAR_YT_TOOLS` that holds `mpv/mpv.exe` and `yt-dlp/yt-dlp.exe`:
//! `cargo test -p sonear-core --test youtube_live -- --ignored --nocapture`
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sonear_core::model::{PlayOffset, SearchKind};
use sonear_core::youtube::{
    LocalLibrary, MpvLauncher, ResolverStreams, YoutubeBackend, YoutubeCatalog, YoutubeLibrary,
};
use sonear_core::{ConfigHandle, LibraryBackend, PlaybackModel, Player, PlayerBackend};
use sonear_youtube::tools::ToolPaths;
use sonear_youtube::{Client, Resolver};
use tokio::runtime::Handle;
use tokio::sync::broadcast;

struct NoSession;

#[async_trait::async_trait]
impl sonear_core::SessionProvider for NoSession {
    async fn fetch(&self, _interactive: bool) -> sonear_core::SessionOutcome {
        sonear_core::SessionOutcome::SignedOut
    }
}

const WAIT: Duration = Duration::from_secs(40);

async fn wait_for(player: &Player, what: &str, ready: impl Fn(&PlaybackModel) -> bool) -> Duration {
    let started = Instant::now();

    while started.elapsed() < WAIT {
        if ready(&player.snapshot()) {
            return started.elapsed();
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let model = player.snapshot();

    panic!(
        "timed out waiting for {what}: {:?} on {:?}",
        model.status,
        item_uri(&model)
    );
}

fn item_uri(model: &PlaybackModel) -> Option<&str> {
    model.item.as_ref().map(|item| item.uri.as_str())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the network and SONEAR_YT_TOOLS"]
async fn searches_plays_queues_skips_and_pauses() {
    let Some(tools) = std::env::var_os("SONEAR_YT_TOOLS").map(PathBuf::from) else {
        eprintln!("SONEAR_YT_TOOLS is not set, skipping");

        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let paths = ToolPaths::new(&tools);
    let config = ConfigHandle::load(dir.path().join("config.toml"));
    let (events, _keep) = broadcast::channel(256);
    let mut notices = events.subscribe();

    tokio::spawn(async move {
        while let Ok(event) = notices.recv().await {
            if let sonear_core::AppEvent::Notice(text) = event {
                eprintln!("notice: {text}");
            }
        }
    });

    let local = LocalLibrary::new(
        dir.path().join("library.json"),
        dir.path().join("plays.json"),
    );
    let catalog = YoutubeCatalog::new(Client::new("en"), local);
    let backend = YoutubeBackend::spawn(
        &Handle::current(),
        Arc::new(MpvLauncher::with_extra_args(
            paths.clone(),
            vec!["--ao=null".to_owned()],
        )),
        Arc::new(ResolverStreams(Resolver::new(paths.ytdlp_exe()))),
        catalog.clone(),
        config.clone(),
        events.clone(),
    );
    let player = Player::new(
        backend.clone(),
        Handle::current(),
        config.clone(),
        events.clone(),
    );
    let account = sonear_core::YoutubeAccount::new(
        catalog.clone(),
        Arc::new(NoSession),
        config.clone(),
        dir.path().join("webview"),
        events.clone(),
    );
    let library = YoutubeLibrary::new(catalog.clone(), backend.clone(), config.clone(), account);

    player.attach_library(library.clone());

    let results = library
        .search(
            "daft punk",
            &[
                SearchKind::Track,
                SearchKind::Artist,
                SearchKind::Album,
                SearchKind::Playlist,
            ],
            0,
        )
        .await
        .unwrap();
    let album = results.albums.items.first().expect("an album").clone();
    let tracks = library.album_tracks(&album.id, 0).await.unwrap().items;

    assert!(tracks.len() >= 3, "{} has too few tracks", album.name);

    let queued = results
        .tracks
        .items
        .iter()
        .find(|track| !tracks.iter().any(|other| other.uri == track.uri))
        .expect("a searched track outside the album")
        .clone();

    eprintln!("album: {} ({} tracks)", album.name, tracks.len());
    player.play_in_context(&album.uri, PlayOffset::Position(1));

    let second = tracks[1].uri.clone();
    let to_playing = wait_for(&player, "the second track playing", |model| {
        model.is_playing()
            && item_uri(model) == Some(second.as_str())
            && model
                .context
                .as_ref()
                .is_some_and(|context| context.uri == album.uri)
    })
    .await;

    eprintln!("play to playing: {to_playing:?}");
    assert!(
        player
            .snapshot()
            .upcoming
            .iter()
            .any(|entry| entry.source == "context"),
        "the context should list what plays next"
    );

    backend.add_to_queue(&queued.uri).unwrap();
    wait_for(&player, "the queued track listed first", |model| {
        model
            .upcoming
            .first()
            .is_some_and(|entry| entry.source == "queue" && entry.item.uri == queued.uri)
    })
    .await;

    player.next();

    let to_skip = wait_for(&player, "the queued track playing", |model| {
        model.is_playing() && item_uri(model) == Some(queued.uri.as_str())
    })
    .await;

    eprintln!("skip to playing: {to_skip:?}");

    player.pause();
    wait_for(&player, "pause", |model| !model.is_playing()).await;

    let plays = catalog.local().plays(10);

    assert!(
        plays.iter().any(|play| play.track.uri == queued.uri),
        "the queued track should be recorded: {plays:?}"
    );
    assert!(plays.iter().any(|play| play.track.uri == second));
}
