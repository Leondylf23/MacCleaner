//! A background thread that samples memory every couple of seconds, and processes while a view
//! that shows them is open.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;

use super::memory::{MemoryStats, read_memory_stats};
use super::processes::{AppGroup, ProcessCollector};

const INTERVAL: Duration = Duration::from_secs(2);
/// Five minutes of samples.
pub const HISTORY_LEN: usize = 150;

#[derive(Default)]
pub struct Snapshot {
    pub memory: Option<MemoryStats>,
    pub groups: Arc<Vec<AppGroup>>,
    /// Fraction of RAM in use, oldest first.
    pub history: Vec<f32>,
}

pub struct Monitor {
    latest: Arc<Mutex<Arc<Snapshot>>>,
    track_processes: Arc<AtomicBool>,
    wake: Sender<()>,
}

impl Monitor {
    pub fn start(ctx: egui::Context) -> Self {
        let latest = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let track_processes = Arc::new(AtomicBool::new(false));
        let (wake, wake_rx) = channel();
        let (latest_ref, track_ref) = (Arc::clone(&latest), Arc::clone(&track_processes));
        thread::Builder::new()
            .name("monitor".into())
            .spawn(move || sample_loop(latest_ref, track_ref, wake_rx, ctx))
            .expect("failed to start monitor thread");
        Self {
            latest,
            track_processes,
            wake,
        }
    }

    pub fn latest(&self) -> Arc<Snapshot> {
        Arc::clone(&self.latest.lock().unwrap())
    }

    /// Process sampling costs some CPU, so it only runs while someone is looking.
    pub fn track_processes(&self, on: bool) {
        if on && !self.track_processes.swap(true, Ordering::Relaxed) {
            self.refresh_now();
        } else if !on {
            self.track_processes.store(false, Ordering::Relaxed);
        }
    }

    pub fn refresh_now(&self) {
        let _ = self.wake.send(());
    }
}

fn sample_loop(
    latest: Arc<Mutex<Arc<Snapshot>>>,
    track_processes: Arc<AtomicBool>,
    wake: Receiver<()>,
    ctx: egui::Context,
) {
    let mut collector = ProcessCollector::new();
    let mut history = VecDeque::with_capacity(HISTORY_LEN);
    loop {
        let memory = read_memory_stats();
        if let Some(stats) = memory.filter(|m| m.total > 0) {
            if history.len() == HISTORY_LEN {
                history.pop_front();
            }
            history.push_back(stats.used() as f32 / stats.total as f32);
        }
        let groups = match (&mut collector, track_processes.load(Ordering::Relaxed)) {
            (Some(collector), true) => Arc::new(collector.collect()),
            _ => Arc::clone(&latest.lock().unwrap().groups),
        };
        *latest.lock().unwrap() = Arc::new(Snapshot {
            memory,
            groups,
            history: history.iter().copied().collect(),
        });
        ctx.request_repaint();

        match wake.recv_timeout(INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}
