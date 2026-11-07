use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Notify;

use super::*;
use crate::library::LibraryResult;

#[derive(Default)]
struct Fake {
    contains_calls: Mutex<Vec<usize>>,
    saved: Mutex<Vec<String>>,
    removed: Mutex<Vec<String>>,
    fail_writes: bool,
    gate: Option<Arc<Notify>>,
}

#[async_trait]
impl LibraryBackend for Fake {
    async fn contains(&self, uris: &[String]) -> LibraryResult<Vec<bool>> {
        self.contains_calls.lock().unwrap().push(uris.len());

        Ok(uris.iter().map(|uri| uri.ends_with('7')).collect())
    }

    async fn save(&self, uris: &[String]) -> LibraryResult<()> {
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }

        if self.fail_writes {
            return Err(LibraryError::Forbidden);
        }

        self.saved.lock().unwrap().extend(uris.iter().cloned());

        Ok(())
    }

    async fn remove(&self, uris: &[String]) -> LibraryResult<()> {
        if self.fail_writes {
            return Err(LibraryError::Network("down".into()));
        }

        self.removed.lock().unwrap().extend(uris.iter().cloned());

        Ok(())
    }
}

fn store(fake: Fake) -> (Arc<LikeStore>, Arc<Fake>, broadcast::Receiver<AppEvent>) {
    let fake = Arc::new(fake);
    let (events, receiver) = broadcast::channel(64);

    (LikeStore::new(fake.clone(), events), fake, receiver)
}

fn uris(count: usize) -> Vec<String> {
    (0..count).map(|n| format!("spotify:track:{n}")).collect()
}

#[tokio::test]
async fn unknown_uris_are_checked_in_chunks_once() {
    let (likes, fake, _) = store(Fake::default());
    let all = uris(95);

    likes.ensure(&all).await;

    let mut sizes = fake.contains_calls.lock().unwrap().clone();

    sizes.sort_unstable();

    assert_eq!(sizes, [15, 40, 40]);
    assert_eq!(likes.is_liked("spotify:track:7"), Some(true));
    assert_eq!(likes.is_liked("spotify:track:8"), Some(false));
    assert_eq!(likes.is_liked("spotify:track:999"), None);

    likes.ensure(&all).await;

    assert_eq!(fake.contains_calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn nothing_new_to_check_posts_no_event() {
    let (likes, _, mut events) = store(Fake::default());
    let all = uris(3);

    likes.ensure(&all).await;

    assert!(matches!(events.try_recv(), Ok(AppEvent::Likes)));

    likes.ensure(&all).await;

    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn local_files_and_episodes_are_never_checked() {
    let (likes, fake, _) = store(Fake::default());

    likes
        .ensure(&["spotify:local:a:b:c:1".into(), "spotify:episode:e".into()])
        .await;

    assert!(fake.contains_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn toggle_is_optimistic_and_saves() {
    let gate = Arc::new(Notify::new());
    let (likes, fake, _) = store(Fake {
        gate: Some(gate.clone()),
        ..Fake::default()
    });
    let uri = "spotify:track:1";
    let pending = tokio::spawn({
        let likes = likes.clone();

        async move { likes.toggle(uri).await }
    });

    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(likes.is_liked(uri), Some(true));
    assert!(fake.saved.lock().unwrap().is_empty());

    gate.notify_one();

    assert_eq!(pending.await.unwrap(), Ok(true));
    assert_eq!(*fake.saved.lock().unwrap(), [uri]);
}

#[tokio::test]
async fn a_second_click_during_a_save_is_ignored() {
    let gate = Arc::new(Notify::new());
    let (likes, fake, _) = store(Fake {
        gate: Some(gate.clone()),
        ..Fake::default()
    });
    let uri = "spotify:track:1";
    let pending = tokio::spawn({
        let likes = likes.clone();

        async move { likes.toggle(uri).await }
    });

    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(likes.toggle(uri).await, Ok(true));
    assert!(fake.removed.lock().unwrap().is_empty());

    gate.notify_one();

    assert_eq!(pending.await.unwrap(), Ok(true));
    assert_eq!(likes.toggle(uri).await, Ok(false));
    assert_eq!(*fake.removed.lock().unwrap(), [uri]);
}

#[tokio::test]
async fn toggling_a_liked_track_removes_it() {
    let (likes, fake, _) = store(Fake::default());

    likes.mark(&["spotify:track:5".to_owned()], true);

    assert_eq!(likes.toggle("spotify:track:5").await, Ok(false));
    assert_eq!(likes.is_liked("spotify:track:5"), Some(false));
    assert_eq!(*fake.removed.lock().unwrap(), ["spotify:track:5"]);
}

#[tokio::test]
async fn a_failed_save_rolls_back_and_posts_a_notice() {
    let (likes, _, mut events) = store(Fake {
        fail_writes: true,
        ..Fake::default()
    });
    let uri = "spotify:track:1";

    assert_eq!(likes.toggle(uri).await, Err(LibraryError::Forbidden));
    assert_eq!(likes.is_liked(uri), Some(false));

    let mut notice = None;

    while let Ok(event) = events.try_recv() {
        if let AppEvent::Notice(text) = event {
            notice = Some(text);
        }
    }

    assert!(notice.unwrap().contains("Couldn't update"));
}

#[tokio::test]
async fn a_failed_remove_restores_the_heart() {
    let (likes, _, _) = store(Fake {
        fail_writes: true,
        ..Fake::default()
    });

    likes.mark(&["spotify:track:2".to_owned()], true);

    assert!(likes.toggle("spotify:track:2").await.is_err());
    assert_eq!(likes.is_liked("spotify:track:2"), Some(true));
}
