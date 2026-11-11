use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use sonear_youtube::Client;

use super::*;
use crate::playback::PlaybackModel;
use crate::youtube::local::LocalLibrary;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct FakeEngine {
    calls: Mutex<Vec<String>>,
    events: broadcast::Sender<MpvEvent>,
    position: Mutex<Option<u64>>,
}

impl FakeEngine {
    fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }

    fn send(&self, event: MpvEvent) {
        let _ = self.events.send(event);
    }

    fn loads(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|call| call.starts_with("load:"))
            .collect()
    }
}

#[async_trait]
impl AudioEngine for FakeEngine {
    async fn load(&self, url: &str) -> Result<(), ToolError> {
        lock(&self.calls).push(format!("load:{url}"));

        Ok(())
    }

    async fn stop(&self) -> Result<(), ToolError> {
        lock(&self.calls).push("stop".into());

        Ok(())
    }

    async fn set_pause(&self, paused: bool) -> Result<(), ToolError> {
        lock(&self.calls).push(format!("pause:{paused}"));

        Ok(())
    }

    async fn seek(&self, position_ms: u64) -> Result<(), ToolError> {
        lock(&self.calls).push(format!("seek:{position_ms}"));

        Ok(())
    }

    async fn set_volume(&self, volume: u8) -> Result<(), ToolError> {
        lock(&self.calls).push(format!("volume:{volume}"));

        Ok(())
    }

    async fn position_ms(&self) -> Option<u64> {
        *lock(&self.position)
    }

    fn events(&self) -> broadcast::Receiver<MpvEvent> {
        self.events.subscribe()
    }
}

#[derive(Default)]
struct FakeLauncher {
    engines: Mutex<Vec<Arc<FakeEngine>>>,
    volumes: Mutex<Vec<u8>>,
    fail: Mutex<Option<ToolError>>,
}

impl FakeLauncher {
    fn engine(&self, index: usize) -> Arc<FakeEngine> {
        lock(&self.engines)[index].clone()
    }

    fn launched(&self) -> usize {
        lock(&self.engines).len()
    }
}

#[async_trait]
impl EngineLauncher for FakeLauncher {
    async fn launch(&self, volume: u8) -> Result<Arc<dyn AudioEngine>, ToolError> {
        if let Some(error) = lock(&self.fail).clone() {
            return Err(error);
        }

        let engine = Arc::new(FakeEngine {
            calls: Mutex::new(Vec::new()),
            events: broadcast::channel(64).0,
            position: Mutex::new(None),
        });

        lock(&self.volumes).push(volume);
        lock(&self.engines).push(engine.clone());

        Ok(engine)
    }
}

#[derive(Default)]
struct FakeStreams {
    resolved: Mutex<Vec<String>>,
    prefetched: Mutex<Vec<String>>,
    forgotten: Mutex<Vec<String>>,
    failing: Mutex<HashSet<String>>,
    delays: Mutex<HashMap<String, Duration>>,
    errors: Mutex<VecDeque<ToolError>>,
}

impl FakeStreams {
    fn resolved(&self) -> Vec<String> {
        lock(&self.resolved).clone()
    }
}

#[async_trait]
impl StreamSource for FakeStreams {
    async fn resolve(&self, video_id: &str) -> Result<String, ToolError> {
        lock(&self.resolved).push(video_id.to_owned());

        let delay = lock(&self.delays).get(video_id).copied();

        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }

        if lock(&self.failing).contains(video_id) {
            return Err(lock(&self.errors)
                .pop_front()
                .unwrap_or_else(|| ToolError::Resolve("sign in".into())));
        }

        Ok(format!("https://stream.example/{video_id}"))
    }

    fn prefetch(&self, video_id: &str) {
        lock(&self.prefetched).push(video_id.to_owned());
    }

    fn forget(&self, video_id: &str) {
        lock(&self.forgotten).push(video_id.to_owned());
    }
}

struct Rig {
    backend: Arc<YoutubeBackend>,
    launcher: Arc<FakeLauncher>,
    streams: Arc<FakeStreams>,
    catalog: Arc<YoutubeCatalog>,
    feed: broadcast::Receiver<ClientEvent>,
    notices: broadcast::Receiver<AppEvent>,
    model: PlaybackModel,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let launcher = Arc::new(FakeLauncher::default());
        let streams = Arc::new(FakeStreams::default());
        let catalog = YoutubeCatalog::new(
            Client::with_base("en", "http://127.0.0.1:1/"),
            LocalLibrary::in_memory(),
        );
        let (app_events, notices) = broadcast::channel(64);
        let backend = YoutubeBackend::spawn(
            &Handle::current(),
            launcher.clone(),
            streams.clone(),
            catalog.clone(),
            ConfigHandle::load(dir.path().join("config.toml")),
            app_events,
        );
        let feed = backend.subscribe();

        Self {
            backend,
            launcher,
            streams,
            catalog,
            feed,
            notices,
            model: PlaybackModel::default(),
            _dir: dir,
        }
    }

    async fn settle(&mut self) {
        tokio::time::sleep(Duration::from_millis(1)).await;

        for _ in 0..20 {
            tokio::task::yield_now().await;
        }

        while let Ok(event) = self.feed.try_recv() {
            if let ClientEvent::Event(event) = event {
                self.model.apply(&event);
            }
        }
    }

    fn notices(&mut self) -> Vec<String> {
        let mut found = Vec::new();

        while let Ok(event) = self.notices.try_recv() {
            if let AppEvent::Notice(text) = event {
                found.push(text);
            }
        }

        found
    }

    fn item(&self) -> String {
        self.model
            .item
            .as_ref()
            .map(|item| item.name().to_owned())
            .unwrap_or_default()
    }

    async fn play(&mut self, names: &[&str]) {
        self.backend
            .play_list(names.iter().map(|name| track(name)).collect(), 0, None)
            .unwrap();
        self.settle().await;
    }

    /// The engine reports the loaded track playing.
    async fn start_playing(&mut self, engine: usize) {
        let engine = self.launcher.engine(engine);

        engine.send(MpvEvent::FileLoaded);
        engine.send(MpvEvent::PlaybackRestart);
        self.settle().await;
    }
}

/// The test clock moves a millisecond or two while the actor settles.
fn assert_near(actual: u64, expected: u64) {
    assert!(
        (expected..expected + 10).contains(&actual),
        "{actual} is not near {expected}"
    );
}

fn track(name: &str) -> Track {
    Track {
        uri: format!("ytm:track:{name}"),
        id: Some(name.to_owned()),
        name: name.to_owned(),
        duration_ms: 200_000,
        playable: true,
        ..Track::default()
    }
}

#[tokio::test(start_paused = true)]
async fn the_player_announces_itself_and_starts_empty() {
    let mut rig = Rig::new();

    rig.settle().await;

    assert!(rig.model.logged_in && rig.model.is_active);
    assert_eq!(rig.model.status, PlayStatus::Stopped);
    assert!(rig.model.item.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_track_buffers_until_mpv_plays_it() {
    let mut rig = Rig::new();

    rig.play(&["a", "b", "c"]).await;

    assert_eq!(rig.item(), "a");
    assert_eq!(rig.model.status, PlayStatus::Buffering);
    assert_eq!(rig.launcher.launched(), 1);
    assert_eq!(
        rig.launcher.engine(0).calls(),
        ["load:https://stream.example/a", "pause:false"]
    );

    rig.start_playing(0).await;

    assert_eq!(rig.model.status, PlayStatus::Playing);
    assert_eq!(*lock(&rig.streams.prefetched), ["b"]);
    assert_eq!(rig.catalog.local().plays(5)[0].track.name, "a");
}

#[tokio::test(start_paused = true)]
async fn the_end_of_a_track_starts_the_next_and_prefetches_the_one_after() {
    let mut rig = Rig::new();

    rig.play(&["a", "b", "c"]).await;
    rig.start_playing(0).await;
    rig.launcher.engine(0).send(MpvEvent::EndFile {
        reason: EndReason::Eof,
        error: None,
    });
    rig.settle().await;

    assert_eq!(rig.item(), "b");
    assert_eq!(rig.model.status, PlayStatus::Buffering);

    rig.start_playing(0).await;

    assert_eq!(rig.model.status, PlayStatus::Playing);
    assert_eq!(*lock(&rig.streams.prefetched), ["b", "c"]);
    assert_eq!(rig.launcher.launched(), 1);
}

#[tokio::test(start_paused = true)]
async fn our_own_replace_does_not_skip_a_track() {
    let mut rig = Rig::new();

    rig.play(&["a", "b", "c"]).await;
    rig.start_playing(0).await;
    rig.launcher.engine(0).send(MpvEvent::EndFile {
        reason: EndReason::Stop,
        error: None,
    });
    rig.settle().await;

    assert_eq!(rig.item(), "a");
    assert_eq!(rig.model.status, PlayStatus::Playing);
}

#[tokio::test(start_paused = true)]
async fn queued_tracks_play_before_the_context_resumes() {
    let mut rig = Rig::new();

    rig.catalog.remember(&[track("q1"), track("q2")]);
    rig.play(&["a", "b", "c"]).await;
    rig.start_playing(0).await;
    rig.backend.add_to_queue("ytm:track:q1").unwrap();
    rig.backend.add_to_queue("ytm:track:q2").unwrap();
    rig.settle().await;

    assert_eq!(rig.model.upcoming[0].source, "queue");
    assert_eq!(rig.model.upcoming[0].item.name(), "q1");
    assert_eq!(rig.model.upcoming[1].source, "queue");
    assert_eq!(rig.model.upcoming[2].source, "context");
    assert_eq!(rig.model.upcoming[2].item.name(), "b");

    let end = |rig: &Rig| {
        rig.launcher.engine(0).send(MpvEvent::EndFile {
            reason: EndReason::Eof,
            error: None,
        });
    };

    end(&rig);
    rig.settle().await;
    assert_eq!(rig.item(), "q1");
    rig.start_playing(0).await;

    end(&rig);
    rig.settle().await;
    assert_eq!(rig.item(), "q2");
    rig.start_playing(0).await;

    end(&rig);
    rig.settle().await;
    assert_eq!(rig.item(), "b");
}

#[tokio::test(start_paused = true)]
async fn only_ytm_tracks_can_be_queued() {
    let mut rig = Rig::new();

    rig.play(&["a"]).await;
    rig.backend.add_to_queue("spotify:track:x").unwrap();
    rig.settle().await;

    assert!(rig.model.upcoming.is_empty());
}

#[tokio::test(start_paused = true)]
async fn pausing_while_buffering_loads_the_track_paused() {
    let mut rig = Rig::new();

    rig.backend
        .play_list(vec![track("a"), track("b")], 0, None)
        .unwrap();
    rig.backend.pause().unwrap();
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Paused);
    assert_eq!(
        rig.launcher.engine(0).calls(),
        ["load:https://stream.example/a", "pause:true"]
    );

    rig.start_playing(0).await;

    assert_eq!(rig.model.status, PlayStatus::Paused);

    rig.backend.play(None).unwrap();
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Playing);
    assert_eq!(
        rig.launcher.engine(0).calls().last().map(String::as_str),
        Some("pause:false")
    );
}

#[tokio::test(start_paused = true)]
async fn a_stale_resolve_is_ignored_after_a_quick_skip() {
    let mut rig = Rig::new();

    lock(&rig.streams.delays).insert("a".into(), Duration::from_secs(5));
    rig.play(&["a", "b", "c"]).await;
    rig.backend.next().unwrap();
    rig.settle().await;

    assert_eq!(rig.item(), "b");

    tokio::time::advance(Duration::from_secs(6)).await;
    rig.settle().await;

    assert_eq!(
        rig.launcher.engine(0).loads(),
        ["load:https://stream.example/b"]
    );
    assert_eq!(rig.launcher.launched(), 1);

    rig.start_playing(0).await;

    assert_eq!(rig.item(), "b");
    assert_eq!(rig.model.status, PlayStatus::Playing);
}

#[tokio::test(start_paused = true)]
async fn a_failing_track_is_retried_once_then_skipped_with_a_notice() {
    let mut rig = Rig::new();

    lock(&rig.streams.failing).insert("a".into());
    rig.play(&["a", "b"]).await;

    assert_eq!(rig.streams.resolved(), ["a", "a", "b"]);
    assert_eq!(*lock(&rig.streams.forgotten), ["a", "a"]);
    assert_eq!(rig.notices().len(), 1);
    assert_eq!(rig.item(), "b");
    assert_eq!(
        rig.launcher.engine(0).loads(),
        ["load:https://stream.example/b"]
    );
}

#[tokio::test(start_paused = true)]
async fn a_mpv_load_error_also_retries_the_track() {
    let mut rig = Rig::new();

    rig.play(&["a", "b"]).await;
    rig.launcher.engine(0).send(MpvEvent::EndFile {
        reason: EndReason::Error,
        error: Some("loading failed".into()),
    });
    rig.settle().await;

    assert_eq!(rig.streams.resolved(), ["a", "a"]);
    assert_eq!(*lock(&rig.streams.forgotten), ["a"]);
    assert_eq!(rig.item(), "a");
    assert!(rig.notices().is_empty());
}

#[tokio::test(start_paused = true)]
async fn three_failed_tracks_in_a_row_pause_the_player() {
    let mut rig = Rig::new();

    for id in ["a", "b", "c", "d"] {
        lock(&rig.streams.failing).insert(id.into());
    }

    rig.play(&["a", "b", "c", "d"]).await;

    assert_eq!(rig.streams.resolved(), ["a", "a", "b", "b", "c", "c"]);
    assert_eq!(rig.notices().len(), 3);
    assert_eq!(rig.model.status, PlayStatus::Paused);
    assert_eq!(rig.item(), "c");
}

#[tokio::test(start_paused = true)]
async fn missing_tools_pause_at_once_with_a_notice_and_no_retry() {
    let mut rig = Rig::new();

    *lock(&rig.launcher.fail) = Some(ToolError::NotInstalled);
    rig.play(&["a", "b"]).await;

    let notices = rig.notices();

    assert_eq!(notices, [ToolError::NotInstalled.user_message()]);
    assert_eq!(rig.model.status, PlayStatus::Paused);
    assert_eq!(rig.item(), "a");
    assert_eq!(rig.streams.resolved(), ["a"]);
}

#[tokio::test(start_paused = true)]
async fn when_mpv_exits_the_next_play_resumes_where_it_stopped() {
    let mut rig = Rig::new();

    rig.play(&["a", "b"]).await;
    rig.start_playing(0).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    rig.launcher.engine(0).send(MpvEvent::Exited);
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Paused);
    assert_eq!(rig.item(), "a");
    assert_near(rig.model.position_ms_at(crate::playback::now_ms()), 10_000);

    rig.backend.play(None).unwrap();
    rig.settle().await;

    assert_eq!(rig.launcher.launched(), 2);
    assert_eq!(rig.model.status, PlayStatus::Buffering);

    let engine = rig.launcher.engine(1);

    engine.send(MpvEvent::FileLoaded);
    rig.settle().await;

    let calls = engine.calls();

    assert_eq!(calls[..2], ["load:https://stream.example/a", "pause:false"]);
    assert!(calls[2].starts_with("seek:1000"), "{calls:?}");

    engine.send(MpvEvent::PlaybackRestart);
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Playing);
}

#[tokio::test(start_paused = true)]
async fn the_end_of_the_context_stops_unless_repeating() {
    let mut rig = Rig::new();
    let end = |rig: &Rig| {
        rig.launcher.engine(0).send(MpvEvent::EndFile {
            reason: EndReason::Eof,
            error: None,
        });
    };

    rig.play(&["a", "b"]).await;
    rig.start_playing(0).await;
    rig.backend.next().unwrap();
    rig.settle().await;
    rig.start_playing(0).await;
    end(&rig);
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Paused);
    assert_eq!(rig.item(), "a");
    assert_eq!(rig.model.position_ms_at(crate::playback::now_ms()), 0);
    assert!(rig.launcher.engine(0).calls().contains(&"stop".to_owned()));

    rig.backend.play(None).unwrap();
    rig.settle().await;
    rig.start_playing(0).await;
    rig.backend.set_repeat_context(true).unwrap();
    rig.backend.next().unwrap();
    rig.settle().await;
    rig.start_playing(0).await;
    end(&rig);
    rig.settle().await;

    assert_eq!(rig.item(), "a");
    assert_eq!(rig.model.status, PlayStatus::Buffering);
}

#[tokio::test(start_paused = true)]
async fn a_cache_stall_shows_buffering_and_recovers() {
    let mut rig = Rig::new();

    rig.play(&["a"]).await;
    rig.start_playing(0).await;

    let engine = rig.launcher.engine(0);

    engine.send(MpvEvent::Buffering(true));
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Buffering);

    engine.send(MpvEvent::Buffering(false));
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Playing);
}

#[tokio::test(start_paused = true)]
async fn pause_and_seek_move_the_position_anchor() {
    let mut rig = Rig::new();

    rig.play(&["a"]).await;
    rig.start_playing(0).await;
    tokio::time::advance(Duration::from_secs(4)).await;
    rig.backend.pause().unwrap();
    rig.settle().await;

    assert_eq!(rig.model.status, PlayStatus::Paused);
    assert_near(rig.model.position_ms_at(crate::playback::now_ms()), 4_000);

    rig.backend.seek(60_000).unwrap();
    rig.settle().await;

    assert_eq!(rig.model.position_ms_at(crate::playback::now_ms()), 60_000);
    assert!(
        rig.launcher
            .engine(0)
            .calls()
            .contains(&"seek:60000".to_owned())
    );

    *lock(&rig.launcher.engine(0).position) = Some(61_500);
    rig.launcher.engine(0).send(MpvEvent::PlaybackRestart);
    rig.settle().await;

    assert_eq!(rig.model.position_ms_at(crate::playback::now_ms()), 61_500);
}

#[tokio::test(start_paused = true)]
async fn previous_restarts_a_track_that_has_played_a_while() {
    let mut rig = Rig::new();

    rig.play(&["a", "b"]).await;
    rig.start_playing(0).await;
    rig.backend.next().unwrap();
    rig.settle().await;
    rig.start_playing(0).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    rig.backend.previous().unwrap();
    rig.settle().await;

    assert_eq!(rig.item(), "b");
    assert!(
        rig.launcher
            .engine(0)
            .calls()
            .contains(&"seek:0".to_owned())
    );

    tokio::time::advance(Duration::from_millis(100)).await;
    rig.backend.seek(0).unwrap();
    rig.backend.previous().unwrap();
    rig.settle().await;

    assert_eq!(rig.item(), "a");
}

#[tokio::test(start_paused = true)]
async fn the_volume_set_before_playing_reaches_mpv_when_it_launches() {
    let mut rig = Rig::new();

    rig.backend.set_volume(55).unwrap();
    rig.settle().await;
    rig.play(&["a"]).await;

    assert_eq!(*lock(&rig.launcher.volumes), [55]);
    assert_eq!(rig.model.volume, 55);

    rig.backend.set_volume(20).unwrap();
    rig.settle().await;

    assert!(
        rig.launcher
            .engine(0)
            .calls()
            .contains(&"volume:20".to_owned())
    );
}

#[tokio::test(start_paused = true)]
async fn shuffle_and_repeat_show_in_the_options() {
    let mut rig = Rig::new();

    rig.play(&["a", "b", "c"]).await;
    rig.backend.set_shuffle(true).unwrap();
    rig.backend.set_repeat_track(true).unwrap();
    rig.settle().await;

    let options = &rig.model.options;

    assert!(options.shuffle);
    assert_eq!(options.repeat, sonear_soloist::protocol::RepeatMode::Track);
    assert_eq!(rig.item(), "a");
}
