use std::io::{Cursor, Write};
use std::sync::Mutex;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const MPV_ZIP: &str = "mpv-v0.40.0-x86_64-pc-windows-msvc.zip";

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));

    for (name, content) in files {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }

    writer.finish().unwrap().into_inner()
}

fn good_zip() -> Vec<u8> {
    zip_of(&[
        ("mpv.exe", b"new mpv"),
        ("vulkan-1.dll", b"dll"),
        ("installer.bat", b"echo"),
        ("../evil.exe", b"evil"),
        ("sub/inner.exe", b"inner"),
    ])
}

async fn serve(server: &MockServer, route: &str, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path(route.to_owned()))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
}

async fn release(server: &MockServer, repo: &str, tag: &str, assets: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/{repo}/releases/latest")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tag_name": tag,
            "assets": assets,
        })))
        .mount(server)
        .await;
}

fn mpv_assets(server: &MockServer, zip: &[u8], digest: Option<String>) -> serde_json::Value {
    let url = |name: &str| format!("{}/dl/{name}", server.uri());
    let mut real = json!({ "name": MPV_ZIP, "browser_download_url": url("mpv.zip") });

    if let Some(digest) = digest {
        real["digest"] = digest.into();
    }

    let _ = zip;

    json!([
        {
            "name": "mpv-v0.40.0-x86_64-pc-windows-msvc-pdb.zip",
            "browser_download_url": url("pdb.zip"),
            "digest": "sha256:00",
        },
        {
            "name": "libmpv-x86_64-v0.40.0.7z",
            "browser_download_url": url("lib.7z"),
            "digest": "sha256:00",
        },
        real,
    ])
}

fn installer(dir: &Path, server: &MockServer) -> Installer {
    Installer::with_api_base(ToolPaths::new(dir), &server.uri())
}

#[tokio::test]
async fn installs_mpv_from_the_verified_zip() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let zip = good_zip();

    release(
        &server,
        MPV_REPO,
        "v0.40.0",
        mpv_assets(&server, &zip, Some(format!("sha256:{}", sha256(&zip)))),
    )
    .await;
    serve(&server, "/dl/mpv.zip", zip).await;

    let events = Mutex::new(Vec::new());
    let installer = installer(dir.path(), &server);
    let tag = installer
        .install_mpv(&|event| events.lock().unwrap().push(event))
        .await
        .unwrap();
    let paths = ToolPaths::new(dir.path());

    assert_eq!(tag, "v0.40.0");
    assert_eq!(fs::read(paths.mpv_exe()).unwrap(), b"new mpv");
    assert!(paths.mpv_dir().join("vulkan-1.dll").exists());
    assert!(!paths.mpv_dir().join("installer.bat").exists());
    assert!(!paths.mpv_dir().join("inner.exe").exists());
    assert!(!dir.path().join("evil.exe").exists());
    assert_eq!(
        ToolVersions::installed(&paths).mpv.as_deref(),
        Some("v0.40.0")
    );

    let events = events.lock().unwrap();

    assert!(matches!(
        events.first(),
        Some(ToolProgress::Downloading {
            tool: Tool::Mpv,
            ..
        })
    ));
    assert_eq!(events.last(), Some(&ToolProgress::Done { tool: Tool::Mpv }));

    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("install-"))
        .collect();

    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test]
async fn a_new_mpv_replaces_the_old_install() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let zip = good_zip();
    let paths = ToolPaths::new(dir.path());

    fs::create_dir_all(paths.mpv_dir()).unwrap();
    fs::write(paths.mpv_exe(), b"old mpv").unwrap();
    fs::write(paths.mpv_dir().join("stale.dll"), b"stale").unwrap();
    release(
        &server,
        MPV_REPO,
        "v0.40.0",
        mpv_assets(&server, &zip, Some(format!("sha256:{}", sha256(&zip)))),
    )
    .await;
    serve(&server, "/dl/mpv.zip", zip).await;

    installer(dir.path(), &server)
        .install_mpv(&|_| {})
        .await
        .unwrap();

    assert_eq!(fs::read(paths.mpv_exe()).unwrap(), b"new mpv");
    assert!(!paths.mpv_dir().join("stale.dll").exists());
    assert!(!dir.path().join("mpv.old").exists());
}

#[tokio::test]
async fn a_wrong_checksum_installs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let zip = good_zip();
    let paths = ToolPaths::new(dir.path());

    fs::create_dir_all(paths.mpv_dir()).unwrap();
    fs::write(paths.mpv_exe(), b"old mpv").unwrap();
    release(
        &server,
        MPV_REPO,
        "v0.40.0",
        mpv_assets(&server, &zip, Some(format!("sha256:{}", sha256(b"other")))),
    )
    .await;
    serve(&server, "/dl/mpv.zip", zip).await;

    let result = installer(dir.path(), &server).install_mpv(&|_| {}).await;

    assert_eq!(result, Err(ToolError::Checksum));
    assert_eq!(fs::read(paths.mpv_exe()).unwrap(), b"old mpv");
    assert_eq!(ToolVersions::installed(&paths).mpv, None);
}

#[tokio::test]
async fn a_missing_digest_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let zip = good_zip();

    release(
        &server,
        MPV_REPO,
        "v0.40.0",
        mpv_assets(&server, &zip, None),
    )
    .await;
    serve(&server, "/dl/mpv.zip", zip).await;

    let result = installer(dir.path(), &server).install_mpv(&|_| {}).await;

    assert_eq!(result, Err(ToolError::Checksum));
    assert!(!ToolPaths::new(dir.path()).mpv_exe().exists());
}

#[tokio::test]
async fn an_archive_without_mpv_exe_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let zip = zip_of(&[("vulkan-1.dll", b"dll")]);

    release(
        &server,
        MPV_REPO,
        "v0.40.0",
        mpv_assets(&server, &zip, Some(format!("sha256:{}", sha256(&zip)))),
    )
    .await;
    serve(&server, "/dl/mpv.zip", zip).await;

    let result = installer(dir.path(), &server).install_mpv(&|_| {}).await;

    assert!(matches!(result, Err(ToolError::Io(_))));
    assert!(!ToolPaths::new(dir.path()).mpv_dir().exists());
}

fn ytdlp_assets(server: &MockServer, digest: Option<String>, with_sums: bool) -> serde_json::Value {
    let url = |name: &str| format!("{}/dl/{name}", server.uri());
    let mut exe = json!({ "name": "yt-dlp.exe", "browser_download_url": url("yt-dlp.exe") });
    let mut assets = Vec::new();

    if let Some(digest) = digest {
        exe["digest"] = digest.into();
    }

    assets.push(exe);

    if with_sums {
        assets.push(json!({ "name": "SHA2-256SUMS", "browser_download_url": url("sums") }));
    }

    assets.into()
}

#[tokio::test]
async fn installs_ytdlp_checked_against_both_digest_and_sums() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let exe = b"yt-dlp binary".to_vec();
    let hash = sha256(&exe);
    let sums = format!("{hash}  yt-dlp.exe\n{}  yt-dlp\n", sha256(b"x"));

    release(
        &server,
        YTDLP_REPO,
        "2026.11.01",
        ytdlp_assets(&server, Some(format!("sha256:{hash}")), true),
    )
    .await;
    serve(&server, "/dl/yt-dlp.exe", exe.clone()).await;
    serve(&server, "/dl/sums", sums.into_bytes()).await;

    let installer = installer(dir.path(), &server);

    installer.install_ytdlp(&|_| {}).await.unwrap();

    let paths = ToolPaths::new(dir.path());

    assert_eq!(fs::read(paths.ytdlp_exe()).unwrap(), exe);
    assert_eq!(
        ToolVersions::installed(&paths).ytdlp.as_deref(),
        Some("2026.11.01")
    );
    assert_eq!(installer.latest_ytdlp_tag().await.unwrap(), "2026.11.01");
}

#[tokio::test]
async fn sums_alone_are_enough_and_disagreement_is_refused() {
    let exe = b"yt-dlp binary".to_vec();
    let hash = sha256(&exe);

    for (digest, sums_hash, expected_ok) in [
        (None, hash.clone(), true),
        (Some(format!("sha256:{hash}")), sha256(b"other"), false),
        (None, sha256(b"other"), false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;

        release(
            &server,
            YTDLP_REPO,
            "2026.11.01",
            ytdlp_assets(&server, digest, true),
        )
        .await;
        serve(&server, "/dl/yt-dlp.exe", exe.clone()).await;
        serve(
            &server,
            "/dl/sums",
            format!("{sums_hash}  yt-dlp.exe\n").into_bytes(),
        )
        .await;

        let result = installer(dir.path(), &server).install_ytdlp(&|_| {}).await;

        assert_eq!(result.is_ok(), expected_ok);

        if !expected_ok {
            assert_eq!(result, Err(ToolError::Checksum));
            assert!(!ToolPaths::new(dir.path()).ytdlp_exe().exists());
        }
    }
}

#[tokio::test]
async fn ytdlp_without_any_checksum_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    release(
        &server,
        YTDLP_REPO,
        "2026.11.01",
        ytdlp_assets(&server, None, false),
    )
    .await;
    serve(&server, "/dl/yt-dlp.exe", b"bin".to_vec()).await;

    let result = installer(dir.path(), &server).install_ytdlp(&|_| {}).await;

    assert_eq!(result, Err(ToolError::Checksum));
}

#[tokio::test]
async fn update_only_downloads_a_different_release() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let exe = b"yt-dlp binary".to_vec();

    release(
        &server,
        YTDLP_REPO,
        "2026.11.01",
        ytdlp_assets(&server, Some(format!("sha256:{}", sha256(&exe))), false),
    )
    .await;
    serve(&server, "/dl/yt-dlp.exe", exe).await;

    let installer = installer(dir.path(), &server);

    assert!(installer.update_ytdlp(&|_| {}).await.unwrap());
    assert!(!installer.update_ytdlp(&|_| {}).await.unwrap());
}

#[tokio::test]
async fn install_missing_skips_what_is_there() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let paths = ToolPaths::new(dir.path());
    let exe = b"yt-dlp binary".to_vec();
    let zip = good_zip();

    release(
        &server,
        MPV_REPO,
        "v0.40.0",
        mpv_assets(&server, &zip, Some(format!("sha256:{}", sha256(&zip)))),
    )
    .await;
    release(
        &server,
        YTDLP_REPO,
        "2026.11.01",
        ytdlp_assets(&server, Some(format!("sha256:{}", sha256(&exe))), false),
    )
    .await;
    serve(&server, "/dl/mpv.zip", zip).await;
    serve(&server, "/dl/yt-dlp.exe", exe).await;

    installer(dir.path(), &server)
        .install_missing(&|_| {})
        .await
        .unwrap();

    assert!(ToolVersions::installed(&paths).is_complete());
}

#[tokio::test]
async fn a_release_lookup_failure_is_a_download_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let result = installer(dir.path(), &server).latest_ytdlp_tag().await;

    assert!(matches!(result, Err(ToolError::Download(_))));
}

#[test]
fn strict_mode_only_trusts_github_over_https() {
    for (url, allowed) in [
        ("https://github.com/a/b", true),
        ("https://objects.githubusercontent.com/x", true),
        ("http://github.com/a", false),
        ("https://github.com.evil.example/a", false),
        ("https://evil.example/githubusercontent.com", false),
    ] {
        assert_eq!(
            allowed_host(&Url::parse(url).unwrap(), true),
            allowed,
            "{url}"
        );
    }
}

#[test]
fn checksums_are_found_by_file_name() {
    let sums = "aa  yt-dlp\nBB *yt-dlp.exe\ncc  yt-dlp_x86.exe\n";

    assert_eq!(checksum_in(sums, "yt-dlp.exe").as_deref(), Some("bb"));
    assert_eq!(checksum_in(sums, "nope"), None);
}
