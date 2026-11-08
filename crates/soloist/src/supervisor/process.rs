use std::io;
use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::{ProcessHandle, SpawnedProcess, Spawner};
use crate::version::{SoloistVersion, VersionError};

/// Starts the real `soloist.exe` and ties it to a Job Object so it dies with
/// the app.
pub struct ProcessSpawner {
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl ProcessSpawner {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let job = job::Job::kill_on_close()
                .inspect_err(|error| tracing::warn!(%error, "could not create the job object"))
                .ok();

            Self { job }
        }

        #[cfg(not(windows))]
        Self {}
    }
}

impl Default for ProcessSpawner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Spawner for ProcessSpawner {
    fn spawn(&self, exe: &Path, args: &[String]) -> io::Result<SpawnedProcess> {
        let mut command = Command::new(exe);

        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(windows)]
        command.creation_flags(crate::CREATE_NO_WINDOW);

        let mut child = command.spawn()?;

        #[cfg(windows)]
        if let (Some(job), Some(handle)) = (&self.job, child.raw_handle())
            && let Err(error) = job.assign(handle)
        {
            tracing::warn!(%error, "could not assign Soloist to the job object");
        }

        let pid = child.id();
        let (lines, output) = mpsc::unbounded_channel();

        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(read_lines(stdout, lines.clone()));
        }

        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(read_lines(stderr, lines));
        }

        Ok(SpawnedProcess {
            handle: Box::new(RealProcess(child)),
            pid,
            output,
        })
    }

    async fn version(&self, exe: &Path) -> Result<SoloistVersion, VersionError> {
        crate::version::probe(exe).await
    }
}

async fn read_lines(reader: impl AsyncRead + Unpin, lines: mpsc::UnboundedSender<String>) {
    let mut reader = BufReader::new(reader).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        if lines.send(line).is_err() {
            break;
        }
    }
}

struct RealProcess(Child);

#[async_trait]
impl ProcessHandle for RealProcess {
    async fn wait(&mut self) -> io::Result<Option<i32>> {
        Ok(self.0.wait().await?.code())
    }

    async fn kill(&mut self) {
        let _ = self.0.kill().await;
    }
}

#[cfg(windows)]
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
