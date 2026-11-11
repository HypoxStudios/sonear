//! Runs the real yt-dlp and mpv. Needs the network and a folder holding
//! `mpv/mpv.exe` and `yt-dlp.exe`, named by `SONEAR_YT_TOOLS`:
//! `cargo test -p sonear-youtube --test live -- --ignored`
#![cfg(windows)]

use std::path::PathBuf;
use std::time::Duration;

use sonear_youtube::mpv::{Mpv, MpvEvent, MpvOptions};
use sonear_youtube::resolver::Resolver;
use tokio::time::{Instant, sleep, timeout};

#[tokio::test]
#[ignore = "needs the network and SONEAR_YT_TOOLS"]
async fn resolves_loads_seeks_and_kills() {
    let Some(tools) = std::env::var_os("SONEAR_YT_TOOLS").map(PathBuf::from) else {
        eprintln!("SONEAR_YT_TOOLS is not set, skipping");

        return;
    };
    let resolver = Resolver::new(tools.join("yt-dlp.exe"));
    let id = std::env::var("SONEAR_YT_ID").unwrap_or_else(|_| "IluRBvnYMoY".to_owned());
    let stream = resolver.resolve(&id).await.unwrap();

    assert!(stream.url.starts_with("https://"));
    assert!(stream.expires_at.is_some());

    let mut mpv = Mpv::spawn(
        &tools.join("mpv").join("mpv.exe"),
        MpvOptions {
            volume: 0,
            audio_device: None,
            extra_args: vec!["--ao=null".into()],
        },
    )
    .await
    .unwrap();
    let client = mpv.client().clone();
    let mut events = client.events();

    client.load(&stream.url).await.unwrap();

    timeout(Duration::from_secs(30), async {
        loop {
            let event = events.recv().await.unwrap();

            eprintln!("mpv event: {event:?}");

            if event == MpvEvent::FileLoaded {
                break;
            }
        }
    })
    .await
    .expect("mpv never reported file-loaded");

    client.seek(60_000).await.unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut position = None;

    while Instant::now() < deadline {
        position = client.position_ms().await;

        if position.is_some_and(|ms| ms >= 59_000) {
            break;
        }

        sleep(Duration::from_millis(200)).await;
    }

    let position = position.expect("mpv reported no position");

    assert!((59_000..75_000).contains(&position), "position {position}");

    client.set_pause(true).await.unwrap();
    client.stop().await.unwrap();
    mpv.kill().await;

    timeout(Duration::from_secs(5), async {
        loop {
            if events.recv().await.unwrap() == MpvEvent::Exited {
                break;
            }
        }
    })
    .await
    .expect("the connection should close after the kill");
}
