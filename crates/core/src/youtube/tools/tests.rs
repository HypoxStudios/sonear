use std::io::{Cursor, Write};

use serde_json::json;
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn mpv_zip() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));

    writer
        .start_file("mpv.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"mpv").unwrap();

    writer.finish().unwrap().into_inner()
}

async fn serve(server: &MockServer, route: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(route.to_owned()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn serve_bytes(server: &MockServer, route: &str, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path(route.to_owned()))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
}

/// A server that has a release of each tool.
async fn releases(server: &MockServer, ytdlp_tag: &str) {
    let zip = mpv_zip();
    let ytdlp = b"yt-dlp".to_vec();
    let url = |name: &str| format!("{}/dl/{name}", server.uri());

    serve(
        server,
        "/mpv-player/mpv/releases/latest",
        json!({
            "tag_name": "v0.40.0",
            "assets": [{
                "name": "mpv-v0.40.0-x86_64-pc-windows-msvc.zip",
                "browser_download_url": url("mpv.zip"),
                "digest": format!("sha256:{}", sha256(&zip)),
            }],
        }),
    )
    .await;
    serve(
        server,
        "/yt-dlp/yt-dlp/releases/latest",
        json!({
            "tag_name": ytdlp_tag,
            "assets": [{
                "name": "yt-dlp.exe",
                "browser_download_url": url("yt-dlp.exe"),
                "digest": format!("sha256:{}", sha256(&ytdlp)),
            }],
        }),
    )
    .await;
    serve_bytes(server, "/dl/mpv.zip", zip).await;
    serve_bytes(server, "/dl/yt-dlp.exe", ytdlp).await;
}

fn tools(
    dir: &std::path::Path,
    server: &MockServer,
) -> (Arc<YoutubeTools>, broadcast::Receiver<AppEvent>) {
    let paths = ToolPaths::new(dir);
    let (events, receiver) = broadcast::channel(256);
    let installer = Installer::with_api_base(paths.clone(), &server.uri());

    (YoutubeTools::new(paths, installer, events), receiver)
}

fn drain(receiver: &mut broadcast::Receiver<AppEvent>) -> Vec<ToolsStatus> {
    let mut found = Vec::new();

    while let Ok(event) = receiver.try_recv() {
        if let AppEvent::Tools(status) = event {
            found.push(status);
        }
    }

    found
}

#[tokio::test]
async fn tools_start_missing_and_install_to_ready() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    releases(&server, "2026.11.01").await;

    let (tools, mut events) = tools(dir.path(), &server);

    assert_eq!(tools.status(), ToolsStatus::Missing);
    assert!(!tools.versions().is_complete());

    tools.install().await.unwrap();

    let seen = drain(&mut events);

    assert!(matches!(
        seen.first(),
        Some(ToolsStatus::Installing {
            tool: Tool::Mpv,
            received: 0,
            ..
        })
    ));
    assert!(seen.iter().any(|status| matches!(
        status,
        ToolsStatus::Installing {
            tool: Tool::YtDlp,
            ..
        }
    )));
    assert_eq!(seen.last(), Some(&ToolsStatus::Ready));
    assert_eq!(tools.status(), ToolsStatus::Ready);
    assert!(tools.versions().is_complete());
    assert!(tools.paths().mpv_exe().is_file());
}

#[tokio::test]
async fn a_failed_install_reports_why_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let (tools, mut events) = tools(dir.path(), &server);
    let error = tools.install().await.unwrap_err();

    assert!(matches!(error, ToolError::Download(_)));
    assert_eq!(tools.status(), ToolsStatus::Failed(error.user_message()));
    assert!(matches!(
        drain(&mut events).last(),
        Some(ToolsStatus::Failed(_))
    ));

    server.reset().await;
    releases(&server, "2026.11.01").await;
    tools.install().await.unwrap();

    assert_eq!(tools.status(), ToolsStatus::Ready);
}

#[tokio::test]
async fn installing_with_everything_present_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    releases(&server, "2026.11.01").await;

    let (tools, _events) = tools(dir.path(), &server);

    tools.install().await.unwrap();

    let requests = server.received_requests().await.unwrap().len();

    tools.install().await.unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), requests);
}

#[tokio::test]
async fn installed_tools_start_ready() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ToolPaths::new(dir.path());

    std::fs::create_dir_all(paths.mpv_dir()).unwrap();
    std::fs::create_dir_all(paths.ytdlp_dir()).unwrap();
    std::fs::write(paths.mpv_exe(), b"x").unwrap();
    std::fs::write(paths.ytdlp_exe(), b"x").unwrap();
    ToolVersions {
        mpv: Some("v0.40.0".into()),
        ytdlp: Some("2026.10.01".into()),
    }
    .save(&paths)
    .unwrap();

    let server = MockServer::start().await;
    let (tools, _events) = self::tools(dir.path(), &server);

    assert_eq!(tools.status(), ToolsStatus::Ready);
}

#[tokio::test]
async fn ytdlp_updates_only_when_a_newer_release_exists() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    releases(&server, "2026.11.01").await;

    let (tools, _events) = tools(dir.path(), &server);

    tools.install().await.unwrap();

    assert!(!tools.update_ytdlp().await.unwrap());

    server.reset().await;
    releases(&server, "2026.11.09").await;

    assert!(tools.update_ytdlp().await.unwrap());
    assert_eq!(tools.versions().ytdlp.as_deref(), Some("2026.11.09"));
}

#[tokio::test(start_paused = true)]
async fn running_publishes_the_starting_status() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let (tools, mut events) = tools(dir.path(), &server);

    tools.run(&Handle::current());
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert_eq!(drain(&mut events), [ToolsStatus::Missing]);
}
