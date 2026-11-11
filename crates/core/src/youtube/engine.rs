//! The seams that keep real processes out of the player's tests: the audio
//! engine (mpv), the thing that launches it, and the stream URL source
//! (yt-dlp).

use std::sync::Arc;

use async_trait::async_trait;
use sonear_youtube::ToolError;
use sonear_youtube::mpv::MpvEvent;
use sonear_youtube::tools::ToolPaths;
use sonear_youtube::{Resolver, mpv::MpvClient};
use tokio::sync::broadcast;

#[async_trait]
pub trait AudioEngine: Send + Sync {
    async fn load(&self, url: &str) -> Result<(), ToolError>;
    async fn stop(&self) -> Result<(), ToolError>;
    async fn set_pause(&self, paused: bool) -> Result<(), ToolError>;
    async fn seek(&self, position_ms: u64) -> Result<(), ToolError>;
    async fn set_volume(&self, volume: u8) -> Result<(), ToolError>;
    async fn position_ms(&self) -> Option<u64>;
    fn events(&self) -> broadcast::Receiver<MpvEvent>;
}

#[async_trait]
pub trait EngineLauncher: Send + Sync {
    async fn launch(&self, volume: u8) -> Result<Arc<dyn AudioEngine>, ToolError>;
}

#[async_trait]
pub trait StreamSource: Send + Sync {
    async fn resolve(&self, video_id: &str) -> Result<String, ToolError>;
    fn prefetch(&self, video_id: &str);
    fn forget(&self, video_id: &str);
}

/// Stream URLs from yt-dlp.
pub struct ResolverStreams(pub Resolver);

#[async_trait]
impl StreamSource for ResolverStreams {
    async fn resolve(&self, video_id: &str) -> Result<String, ToolError> {
        Ok(self.0.resolve(video_id).await?.url)
    }

    fn prefetch(&self, video_id: &str) {
        self.0.prefetch(video_id);
    }

    fn forget(&self, video_id: &str) {
        self.0.forget(video_id);
    }
}

/// Drives a client the engine does not own the process of.
#[allow(dead_code)]
struct ClientEngine {
    client: MpvClient,
    /// Keeps the process alive; dropping it ends mpv.
    _process: Option<Box<dyn std::any::Any + Send + Sync>>,
}

#[async_trait]
impl AudioEngine for ClientEngine {
    async fn load(&self, url: &str) -> Result<(), ToolError> {
        self.client.load(url).await
    }

    async fn stop(&self) -> Result<(), ToolError> {
        self.client.stop().await
    }

    async fn set_pause(&self, paused: bool) -> Result<(), ToolError> {
        self.client.set_pause(paused).await
    }

    async fn seek(&self, position_ms: u64) -> Result<(), ToolError> {
        self.client.seek(position_ms).await
    }

    async fn set_volume(&self, volume: u8) -> Result<(), ToolError> {
        self.client.set_volume(volume).await
    }

    async fn position_ms(&self) -> Option<u64> {
        self.client.position_ms().await
    }

    fn events(&self) -> broadcast::Receiver<MpvEvent> {
        self.client.events()
    }
}

/// Starts mpv from the downloaded tools.
pub struct MpvLauncher {
    paths: ToolPaths,
    extra_args: Vec<String>,
}

impl MpvLauncher {
    pub fn new(paths: ToolPaths) -> Self {
        Self::with_extra_args(paths, Vec::new())
    }

    /// Adds command line options to every mpv it starts, e.g. `--ao=null`.
    pub fn with_extra_args(paths: ToolPaths, extra_args: Vec<String>) -> Self {
        Self { paths, extra_args }
    }
}

#[cfg(windows)]
#[async_trait]
impl EngineLauncher for MpvLauncher {
    async fn launch(&self, volume: u8) -> Result<Arc<dyn AudioEngine>, ToolError> {
        use sonear_youtube::mpv::{Mpv, MpvOptions};

        let exe = self.paths.mpv_exe();

        if !exe.is_file() {
            return Err(ToolError::NotInstalled);
        }

        let mpv = Mpv::spawn(
            &exe,
            MpvOptions {
                volume,
                audio_device: None,
                extra_args: self.extra_args.clone(),
            },
        )
        .await?;
        let client = mpv.client().clone();

        Ok(Arc::new(ClientEngine {
            client,
            _process: Some(Box::new(mpv)),
        }))
    }
}

#[cfg(not(windows))]
#[async_trait]
impl EngineLauncher for MpvLauncher {
    async fn launch(&self, _volume: u8) -> Result<Arc<dyn AudioEngine>, ToolError> {
        let _ = (&self.paths, &self.extra_args);

        Err(ToolError::NotInstalled)
    }
}
