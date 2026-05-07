use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tracing::info;

/// Per-minute aggregated counters for the SNS pipeline.
///
/// - `resolved`  : targets selected by the dispatcher (one per device per event)
/// - `enqueued`  : messages placed into the broadcast channel by the publisher
/// - `lagged`    : messages dropped by the worker due to channel backlog (Lagged)
/// - `published` : successful AWS SNS publishes
/// - `failed`    : failed AWS SNS publishes (any error variant)
#[derive(Default)]
pub struct SnsMetrics {
    pub resolved: AtomicU64,
    pub enqueued: AtomicU64,
    pub lagged: AtomicU64,
    pub published: AtomicU64,
    pub failed: AtomicU64,
}

impl SnsMetrics {
    pub fn inc_resolved(&self) {
        self.resolved.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_enqueued(&self) {
        self.enqueued.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_lagged(&self, n: u64) {
        self.lagged.fetch_add(n, Ordering::Relaxed);
    }

    pub fn inc_published(&self) {
        self.published.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_failed(&self) {
        self.failed.fetch_add(1, Ordering::Relaxed);
    }
}

/// Spawns a task that logs aggregated SNS counters every `interval` and resets them to zero.
/// Each log line carries the label `sns_metrics_window`.
pub async fn run_metrics_reporter(metrics: Arc<SnsMetrics>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // skip the immediate first tick; wait for the first full window

    loop {
        ticker.tick().await;

        let resolved = metrics.resolved.swap(0, Ordering::Relaxed);
        let enqueued = metrics.enqueued.swap(0, Ordering::Relaxed);
        let lagged = metrics.lagged.swap(0, Ordering::Relaxed);
        let published = metrics.published.swap(0, Ordering::Relaxed);
        let failed = metrics.failed.swap(0, Ordering::Relaxed);

        info!(
            window_secs = interval.as_secs(),
            resolved, enqueued, lagged, published, failed, "sns_metrics_window"
        );
    }
}
