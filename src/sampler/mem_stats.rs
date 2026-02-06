// SPDX-License-Identifier: MIT
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::fex::smaps::{MemSampler, MemSnapshot};

pub struct MemStatsWorker {
    latest: Arc<Mutex<MemSnapshot>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MemStatsWorker {
    /// Spawns a background thread that periodically samples `/proc/{pid}/smaps`.
    ///
    /// # Errors
    ///
    /// Returns an error if the initial `MemSampler` cannot be created.
    pub fn spawn(pid: i32, sample_period: Duration) -> anyhow::Result<Self> {
        let mut sampler = MemSampler::new(pid)?;
        let latest = Arc::new(Mutex::new(MemSnapshot::default()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let latest_clone = Arc::clone(&latest);
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = thread::Builder::new()
            .name("mem-sampler".into())
            .spawn(move || {
                while !shutdown_clone.load(Ordering::Relaxed) {
                    if let Ok(snap) = sampler.sample()
                        && let Ok(mut guard) = latest_clone.lock()
                    {
                        *guard = snap;
                    }
                    thread::sleep(sample_period);
                }
            })
            .map_err(|e| anyhow::anyhow!("failed to spawn mem-sampler thread: {e}"))?;

        Ok(Self {
            latest,
            shutdown,
            handle: Some(handle),
        })
    }

    #[must_use]
    pub fn latest(&self) -> MemSnapshot {
        self.latest
            .lock()
            .map_or_else(|_| MemSnapshot::default(), |guard| guard.clone())
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for MemStatsWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}
