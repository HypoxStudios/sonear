//! Work on a plain thread that a window polls while it runs.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gpui_kit::{Context, Task};

/// How often a window checks on a job.
pub const POLL_EVERY: Duration = Duration::from_millis(33);

/// Calls `tick` on the view every `POLL_EVERY` until it returns true. Runs only
/// while a job does, so nothing wakes up afterwards.
pub fn watch<V: 'static>(
    cx: &mut Context<V>,
    tick: fn(&mut V, &mut Context<V>) -> bool,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL_EVERY).await;

            match this.update(cx, tick) {
                Ok(false) => {}
                _ => break,
            }
        }
    })
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub fraction: f32,
    pub label: String,
    /// Bytes done and total, while files are being copied.
    pub bytes: Option<(u64, u64)>,
}

/// Handed to the work so it can say how far along it is.
#[derive(Clone)]
pub struct Reporter(Arc<Mutex<Snapshot>>);

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Reporter {
    pub fn step(&self, fraction: f32, label: &str) {
        let mut snapshot = locked(&self.0);

        snapshot.fraction = fraction;
        snapshot.label = label.to_owned();
    }

    pub fn bytes(&self, done: u64, total: u64) {
        let mut snapshot = locked(&self.0);

        snapshot.fraction = done as f32 / total.max(1) as f32;
        snapshot.bytes = Some((done, total));
    }
}

pub struct Job<T> {
    progress: Arc<Mutex<Snapshot>>,
    result: Arc<Mutex<Option<T>>>,
}

impl<T: Send + 'static> Job<T> {
    /// Runs `work` on a new thread and keeps its result until `take`. The job
    /// takes at least `min` in all, so a quick one does not flash past.
    pub fn spawn(min: Duration, work: impl FnOnce(&Reporter) -> T + Send + 'static) -> Self {
        let progress = Arc::new(Mutex::new(Snapshot::default()));
        let result = Arc::new(Mutex::new(None));
        let reporter = Reporter(progress.clone());
        let slot = result.clone();

        std::thread::spawn(move || {
            let started = Instant::now();
            let outcome = work(&reporter);

            if let Some(left) = min.checked_sub(started.elapsed()) {
                std::thread::sleep(left);
            }

            *locked(&slot) = Some(outcome);
        });

        Self { progress, result }
    }

    pub fn snapshot(&self) -> Snapshot {
        locked(&self.progress).clone()
    }

    /// The result once the work is done. Only returns it once.
    pub fn take(&self) -> Option<T> {
        locked(&self.result).take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_reports_and_finishes() {
        let job = Job::spawn(Duration::ZERO, |reporter| {
            reporter.step(0.5, "Halfway");
            7
        });

        let result = loop {
            if let Some(result) = job.take() {
                break result;
            }

            std::thread::sleep(Duration::from_millis(5));
        };

        assert_eq!(result, 7);
        assert_eq!(job.snapshot().label, "Halfway");
        assert_eq!(job.take(), None);
    }

    #[test]
    fn a_quick_job_is_held_for_the_minimum() {
        let started = Instant::now();
        let job = Job::spawn(Duration::from_millis(150), |_| 1);

        while job.take().is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(started.elapsed() >= Duration::from_millis(150));
    }
}
