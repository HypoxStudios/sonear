use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::net::windows::named_pipe::ClientOptions;
use tokio::process::{Child, Command};
use tracing::{debug, warn};

use super::client::MpvClient;
use crate::error::ToolError;

const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
const RETRY_EVERY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MpvOptions {
    pub volume: u8,
    pub audio_device: Option<String>,
    pub extra_args: Vec<String>,
}

/// A running mpv. Dropping it kills the process.
pub struct Mpv {
    child: Child,
    client: MpvClient,
    _job: Option<job::Job>,
}

fn pipe_name() -> String {
    format!(
        "\\\\.\\pipe\\sonear-mpv-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    )
}

fn arguments(pipe: &str, options: &MpvOptions) -> Vec<String> {
    let mut args = vec![
        "--idle=yes".to_owned(),
        "--no-video".to_owned(),
        "--no-terminal".to_owned(),
        "--no-config".to_owned(),
        "--force-window=no".to_owned(),
        "--player-operation-mode=cplayer".to_owned(),
        format!("--input-ipc-server={pipe}"),
        format!("--volume={}", options.volume.min(100)),
    ];

    if let Some(device) = &options.audio_device {
        args.push(format!("--audio-device={device}"));
    }

    args.extend(options.extra_args.iter().cloned());

    args
}

impl Mpv {
    pub async fn spawn(exe: &Path, options: MpvOptions) -> Result<Self, ToolError> {
        let pipe = pipe_name();
        let mut command = Command::new(exe);

        command
            .args(arguments(&pipe, &options))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(crate::CREATE_NO_WINDOW);

        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ToolError::NotInstalled
            } else {
                ToolError::Spawn(error.to_string())
            }
        })?;
        let job = job::Job::kill_on_close()
            .and_then(|job| {
                job.assign(
                    child
                        .raw_handle()
                        .ok_or(std::io::Error::other("no handle"))?,
                )?;

                Ok(job)
            })
            .inspect_err(|error| warn!(%error, "could not put mpv in a job object"))
            .ok();
        let started = Instant::now();

        loop {
            match ClientOptions::new().open(&pipe) {
                Ok(transport) => {
                    debug!(pipe, "connected to mpv");

                    return Ok(Self {
                        child,
                        client: MpvClient::connect(transport).await,
                        _job: job,
                    });
                }
                Err(error) => {
                    if let Ok(Some(status)) = child.try_wait() {
                        return Err(ToolError::Spawn(format!("mpv exited early ({status})")));
                    }

                    if started.elapsed() > CONNECT_DEADLINE {
                        let _ = child.kill().await;

                        return Err(ToolError::Spawn(format!(
                            "could not connect to mpv: {error}"
                        )));
                    }

                    tokio::time::sleep(RETRY_EVERY).await;
                }
            }
        }
    }

    pub fn client(&self) -> &MpvClient {
        &self.client
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

mod job {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::RawHandle;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job object handle is a kernel handle usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn kill_on_close() -> io::Result<Self> {
            // SAFETY: plain Win32 calls with valid arguments; the handle is
            // closed in `Drop`.
            unsafe {
                let handle = CreateJobObjectW(None, None).map_err(io::Error::other)?;
                let job = Self(handle);
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();

                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const c_void,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .map_err(io::Error::other)?;

                Ok(job)
            }
        }

        pub fn assign(&self, process: RawHandle) -> io::Result<()> {
            // SAFETY: `process` is a live process handle owned by the caller.
            unsafe { AssignProcessToJobObject(self.0, HANDLE(process)) }.map_err(io::Error::other)
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle was created by `CreateJobObjectW` and is
            // closed exactly once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_names_are_unique_and_namespaced() {
        let first = pipe_name();

        assert!(first.starts_with("\\\\.\\pipe\\sonear-mpv-"));
        assert_ne!(first, pipe_name());
    }

    #[test]
    fn launch_arguments_match_the_verified_command_line() {
        let options = MpvOptions {
            volume: 150,
            audio_device: Some("wasapi/{guid}".into()),
            extra_args: vec!["--ao=null".into()],
        };
        let args = arguments("\\\\.\\pipe\\p", &options);

        assert_eq!(
            args,
            [
                "--idle=yes",
                "--no-video",
                "--no-terminal",
                "--no-config",
                "--force-window=no",
                "--player-operation-mode=cplayer",
                "--input-ipc-server=\\\\.\\pipe\\p",
                "--volume=100",
                "--audio-device=wasapi/{guid}",
                "--ao=null",
            ]
        );
    }
}
