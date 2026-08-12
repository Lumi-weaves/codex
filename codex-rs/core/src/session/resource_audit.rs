//! Session-local cadence state for periodic active-resource audits.
//!
//! The scheduler is armed only while the owning session has awaited resources.
//! Its task holds a weak session reference while sleeping, so an audit can
//! never keep a session alive. Ordinary runtime events do not touch this
//! cadence; only an explicit interval change may restart the deadline.

use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::task::JoinHandle;

pub(crate) const DEFAULT_RESOURCE_AUDIT_INTERVAL_SECONDS: u64 = 5 * 60;
pub(crate) const MIN_RESOURCE_AUDIT_INTERVAL_SECONDS: u64 = 10;
pub(crate) const MAX_RESOURCE_AUDIT_INTERVAL_SECONDS: u64 = 60 * 60;

pub(crate) struct ResourceAuditScheduler {
    interval_seconds: AtomicU64,
    sequence: AtomicU64,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Default for ResourceAuditScheduler {
    fn default() -> Self {
        Self {
            interval_seconds: AtomicU64::new(DEFAULT_RESOURCE_AUDIT_INTERVAL_SECONDS),
            sequence: AtomicU64::new(0),
            task: Mutex::new(None),
        }
    }
}

impl ResourceAuditScheduler {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn interval_seconds(&self) -> u64 {
        self.interval_seconds.load(Ordering::Acquire)
    }

    pub(crate) fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_seconds())
    }

    pub(crate) fn set_interval_seconds(&self, interval_seconds: u64) {
        self.interval_seconds
            .store(interval_seconds, Ordering::Release);
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn replace_task(&self, task: JoinHandle<()>) {
        if let Some(previous) = self.lock_task().replace(task) {
            previous.abort();
        }
    }

    pub(crate) fn disarm(&self) {
        if let Some(task) = self.lock_task().take() {
            task.abort();
        }
    }

    pub(crate) fn is_armed(&self) -> bool {
        self.lock_task()
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    fn lock_task(&self) -> MutexGuard<'_, Option<JoinHandle<()>>> {
        self.task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for ResourceAuditScheduler {
    fn drop(&mut self) {
        let task = self
            .task
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            task.abort();
        }
    }
}
