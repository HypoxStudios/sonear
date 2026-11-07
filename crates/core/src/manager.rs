use std::fs;
use std::sync::Arc;
use std::time::Duration;

use sonear_soloist::install::{
    DOWNLOAD_URL, InstallError, InstallProgress, Installer, ProcessProbe, SignatureVerifier,
};
use sonear_soloist::supervisor::{LaunchParams, SoloistDirs, SupervisorConfig};
use sonear_soloist::version::SoloistVersion;
use sonear_soloist::{Status, Supervisor};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::warn;

use crate::config::ConfigHandle;
use crate::events::AppEvent;
use crate::paths::AppPaths;
use crate::secrets::{SOLOIST_API_KEY, Secret, SecretError, SecretStore};

const EXPIRY_CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Ties the supervisor to the key store, config and installer.
pub struct SoloistManager {
    supervisor: Supervisor,
    secrets: Arc<dyn SecretStore>,
    config: ConfigHandle,
    installer: Installer,
    events: broadcast::Sender<AppEvent>,
}

impl SoloistManager {
    pub fn new(
        paths: &AppPaths,
        config: ConfigHandle,
        secrets: Arc<dyn SecretStore>,
        verifier: Arc<dyn SignatureVerifier>,
        events: broadcast::Sender<AppEvent>,
    ) -> Arc<Self> {
        let dirs = SoloistDirs::new(paths.soloist_root());
        let supervisor = Supervisor::with_process_spawner(SupervisorConfig::new(dirs));
        let installer = Installer::new(paths.soloist_root(), verifier, Arc::new(ProcessProbe));

        Arc::new(Self {
            supervisor,
            secrets,
            config,
            installer,
            events,
        })
    }

    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    pub fn status(&self) -> Status {
        self.supervisor.status()
    }

    /// Forwards supervisor status changes as `AppEvent::Soloist`, then starts
    /// Soloist and begins the daily expiry check.
    pub fn run(self: &Arc<Self>, runtime: &Handle) {
        let mut status = self.supervisor.subscribe();
        let events = self.events.clone();

        runtime.spawn(async move {
            loop {
                match status.recv().await {
                    Ok(status) => {
                        let _ = events.send(AppEvent::Soloist(status));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let manager = self.clone();

        runtime.spawn(async move {
            manager.start().await;

            loop {
                manager.publish_expiry().await;
                tokio::time::sleep(EXPIRY_CHECK_EVERY).await;
            }
        });
    }

    pub async fn start(&self) {
        let params = self.launch_params();

        self.supervisor.start(params).await;
    }

    /// Relaunches Soloist with the current settings, for example a new device name.
    pub async fn restart(&self) {
        let params = self.launch_params();

        self.supervisor.restart(params).await;
    }

    /// Stops Soloist and deletes its key, pairing and cache. The installed
    /// program stays. Returns what could not be deleted.
    pub async fn forget(&self) -> Vec<String> {
        let mut problems = Vec::new();

        self.supervisor.stop().await;

        if let Err(error) = self.secrets.delete(SOLOIST_API_KEY) {
            problems.push(format!("Soloist key: {error}"));
        }

        let dirs = self.supervisor.dirs();

        for (name, dir) in [
            ("Soloist data", dirs.data()),
            ("Soloist cache", dirs.cache()),
        ] {
            if let Err(error) = fs::remove_dir_all(&dir)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                problems.push(format!("{name}: {error}"));
            }
        }

        problems
    }

    fn launch_params(&self) -> LaunchParams {
        let key = match self.secrets.get(SOLOIST_API_KEY) {
            Ok(key) => key,
            Err(error) => {
                warn!(%error, "could not read the Soloist key");
                None
            }
        };
        let config = self.config.get();

        LaunchParams {
            device_name: config.device_name,
            key: key.map(|key| key.expose().to_owned()),
            volume: config.last_volume,
        }
    }

    pub async fn save_key(&self, key: &str) -> Result<(), SecretError> {
        self.secrets
            .set(SOLOIST_API_KEY, &Secret::new(key.trim()))?;
        self.start().await;

        Ok(())
    }

    /// Downloads and verifies the latest build while Soloist keeps playing,
    /// then restarts it on the new build. A failed download or verification
    /// leaves the current build running.
    pub async fn install(
        &self,
        progress: &(dyn Fn(InstallProgress) + Send + Sync),
    ) -> Result<SoloistVersion, InstallError> {
        let prepared = self.installer.prepare(DOWNLOAD_URL, progress).await?;

        self.supervisor.stop().await;

        let result = self.installer.commit(prepared, progress).await;

        self.start().await;
        self.publish_expiry().await;

        result
    }

    async fn publish_expiry(&self) {
        let today = chrono::Utc::now().date_naive();
        let days_left = self
            .supervisor
            .refresh_version()
            .await
            .map(|version| version.days_left(today));

        let _ = self.events.send(AppEvent::SoloistExpiry { days_left });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[tokio::test]
    async fn forget_deletes_the_key_data_and_cache_but_keeps_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(dir.path().join("config"), dir.path().join("local"));
        let secrets = Arc::new(MemoryStore::default());
        let (events, _) = broadcast::channel(8);
        let manager = SoloistManager::new(
            &paths,
            ConfigHandle::load(paths.config_file()),
            secrets.clone(),
            crate::default_verifier(),
            events,
        );
        let dirs = manager.supervisor().dirs().clone();

        for sub in [dirs.bin(), dirs.data(), dirs.cache()] {
            fs::create_dir_all(&sub).unwrap();
            fs::write(sub.join("file"), b"x").unwrap();
        }

        secrets.set(SOLOIST_API_KEY, &Secret::new("key")).unwrap();

        let problems = manager.forget().await;

        assert!(problems.is_empty(), "{problems:?}");
        assert!(secrets.get(SOLOIST_API_KEY).unwrap().is_none());
        assert!(!dirs.data().exists());
        assert!(!dirs.cache().exists());
        assert!(dirs.bin().join("file").exists());
        assert!(manager.forget().await.is_empty());
    }
}
