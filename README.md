<p align="center">
  <img src="crates/app/assets/mark.svg" alt="Sonear" width="96" height="96">
</p>

<h1 align="center">Sonear</h1>

<p align="center">
  <strong>Spotify, without the browser.</strong><br>
  A native desktop client, built in Rust and drawn by your GPU.
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#features">Features</a> ·
  <a href="#faq">FAQ</a> ·
  <a href="docs/legal/privacy.md">Privacy</a>
</p>

<p align="center">
  <a href="https://github.com/HypoxStudios/sonear/actions/workflows/ci.yml"><img src="https://github.com/HypoxStudios/sonear/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/Windows-C9B2F5" alt="Windows">
  <img src="https://img.shields.io/badge/Rust-C9B2F5" alt="Rust">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-C9B2F5" alt="Apache 2.0"></a>
</p>

![Sonear playing music: colours change with every song, lyrics follow along, the palette searches, and the queue reorders](docs/demo.webp)

<p align="center"><sub>Demo mode, with made-up artists and original artwork. Sonear is independent and not affiliated with Spotify.</sub></p>

## Features

| | |
|---|---|
| **A queue that's yours** | Drag, drop, remove, reorder. Sonear runs the queue itself, so you can change your mind. |
| **Ctrl+K** | Search, playback, navigation and Spotify links, all from one box. Every shortcut can be rebound. |
| **Synced lyrics** | Line by line, from [LRCLIB](https://lrclib.net). Click a line to jump to it. |
| **Color from the cover** | The whole app takes its accent from what's playing. |
| **Mini player** | Small, draggable and always on top. |
| **Tray and media keys** | Close the window and the music keeps going. Your keyboard and the Windows overlay still work. |
| **History** | A plain list of what you played. No stats, no recaps. |

<p align="center">
  <img src="docs/palette.png" alt="The Ctrl+K palette" width="56%">
  &nbsp;
  <img src="docs/mini-player.png" alt="The mini player" width="38%">
</p>

## Light on purpose

| | |
|---|---|
| Idle | **0.8%** of one core |
| Playing | **4.2%** of one core |
| Playing from the tray | **0.5%** of one core |
| Memory | **~95 MB** |
| Size | **36 MB** |

<sub>Release build in demo mode, on one Windows 11 machine, without Soloist. Real libraries use more memory, around 140 MB.</sub>

## Install

Sonear requires **Spotify Premium** and Windows 11. Build it with [Rust](https://rustup.rs) 1.92+, the Visual Studio C++ build tools and CMake:

```
git clone https://github.com/HypoxStudios/sonear
cd sonear
cargo run -p sonear --release
```

The first launch walks you through connecting your own [Spotify developer app](https://developer.spotify.com/dashboard) and installing [Soloist](https://developer.spotify.com/documentation/soloist), Spotify's official player.

Just want a look? Demo mode needs no account:

```
cargo run -p sonear --features demo --target-dir target/demo
```

## FAQ

<details>
<summary><strong>Why do I need my own developer app?</strong></summary>

Spotify gives third-party apps access per developer app, and wide access is reserved for large companies. So you create your own, for your personal use. It's free and takes a couple of minutes.

</details>

<details>
<summary><strong>Is this official?</strong></summary>

No. Sonear is independent, and isn't made, sponsored or endorsed by Spotify. It uses only official APIs, and music plays through Spotify's own Soloist. Attribution, artwork and data handling follow Spotify's [Developer Policy](https://developer.spotify.com/policy) and [branding guidelines](https://developer.spotify.com/documentation/design).

</details>

<details>
<summary><strong>What does it send, and where?</strong></summary>

Nothing goes to us. There are no analytics, and everything stays on your computer. Sonear talks to Spotify, and to LRCLIB while the lyrics panel is open. **Settings → Delete all Sonear data** wipes everything. Details are in the [privacy policy](docs/legal/privacy.md).

</details>

<details>
<summary><strong>What are the limits?</strong></summary>

Mostly Spotify's:

- Soloist builds expire after 90 days. Sonear warns you and updates with one click.
- Followed playlists don't list their tracks, but they play fine, and Sonear fills them in as it hears them.
- Search shows 10 results per page, artist pages show albums only, and there are no recommendations.
- History starts from your last 50 plays and grows from there, without podcasts.
- Crossfade follows your Spotify account's setting, and Windows has no output device picker.

</details>

<details>
<summary><strong>Is it safe?</strong></summary>

Your login and keys live in Windows Credential Manager, and Soloist only listens on your own machine. Soloist does take its key on the command line, which other programs running as you could read. That's how Soloist works. Report security problems to <support@hypox.studio>.

</details>

<details>
<summary><strong>What's coming?</strong></summary>

An MCP server for your own tools, an assistant you can ask for music, and spoken intros between songs. All optional and off by default.

</details>

## Contributing

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

`crates/soloist` runs the player, `crates/spotify` talks to the Web API, `crates/core` holds playback, queue and library logic, and `crates/app` is the GPUI interface.

## Credits

Built on [GPUI](https://github.com/zed-industries/zed) and [gpui-kit](https://crates.io/crates/gpui-kit), with lyrics from [LRCLIB](https://lrclib.net). Inspired by [Spotifast](https://github.com/crmne/spotifast) and [spotatui](https://github.com/LargeModGames/spotatui).

[Apache 2.0](LICENSE). Spotify's marks belong to Spotify; see [NOTICE](NOTICE). [End-user agreement](docs/legal/terms.md).

Questions? <support@hypox.studio>
