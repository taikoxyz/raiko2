//! Worker management for the Engine.
//!
//! This module provides supervised worker spawning and maintenance task management.
//! Workers are automatically restarted on failure with backoff.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// Configuration for worker pool.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// Number of concurrent workers.
    pub concurrency: usize,
    /// Interval between maintenance ticks.
    pub maintenance_interval: Duration,
    /// Backoff duration after worker restart.
    pub restart_backoff: Duration,
    /// Delay after worker tick failure.
    pub error_backoff: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: 1,
            maintenance_interval: Duration::from_millis(200),
            restart_backoff: Duration::from_secs(1),
            error_backoff: Duration::from_millis(200),
        }
    }
}

/// Trait for types that can be run by a worker.
#[async_trait::async_trait]
pub trait Runnable: Clone + Send + Sync + 'static {
    /// Run one unit of work. Returns true if work was done, false if idle.
    async fn run_one(&self, worker_id: &str) -> Result<bool, String>;

    /// Run a maintenance tick.
    async fn maintenance_tick(&self) -> Result<(), String>;

    /// Get notifier for new work availability.
    fn notifier(&self) -> Arc<Notify>;
}

/// Spawn supervised workers for the given runnable.
pub fn spawn_workers<R: Runnable>(runnable: R, config: &WorkerConfig) {
    let notify = runnable.notifier();

    for i in 0..config.concurrency {
        spawn_worker_supervised(
            runnable.clone(),
            notify.clone(),
            format!("engine-{i}"),
            config.restart_backoff,
            config.error_backoff,
        );
    }

    spawn_maintenance_supervised(
        runnable,
        config.maintenance_interval,
        config.restart_backoff,
    );
}

/// Spawn a supervised worker that restarts on failure.
fn spawn_worker_supervised<R: Runnable>(
    runnable: R,
    notify: Arc<Notify>,
    worker_id: String,
    restart_backoff: Duration,
    error_backoff: Duration,
) {
    tokio::spawn(async move {
        loop {
            let runnable = runnable.clone();
            let notify = notify.clone();
            let worker_inner = worker_id.clone();
            let handle = tokio::spawn(async move {
                loop {
                    match runnable.run_one(&worker_inner).await {
                        Ok(true) => {}
                        Ok(false) => notify.notified().await,
                        Err(err) => {
                            tracing::warn!(
                                worker = %worker_inner,
                                error = %err,
                                "engine worker tick failed"
                            );
                            tokio::time::sleep(error_backoff).await;
                        }
                    }
                }
            });

            match handle.await {
                Ok(()) => {
                    tracing::warn!(worker = %worker_id, "engine worker exited unexpectedly");
                }
                Err(err) => {
                    if err.is_panic() {
                        tracing::error!(worker = %worker_id, "engine worker panicked; restarting");
                    } else {
                        tracing::warn!(worker = %worker_id, "engine worker aborted; restarting");
                    }
                }
            }

            tokio::time::sleep(restart_backoff).await;
        }
    });
}

/// Spawn a supervised maintenance task that restarts on failure.
fn spawn_maintenance_supervised<R: Runnable>(
    runnable: R,
    maintenance_interval: Duration,
    restart_backoff: Duration,
) {
    tokio::spawn(async move {
        loop {
            let runnable = runnable.clone();
            let handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(maintenance_interval);
                loop {
                    interval.tick().await;
                    if let Err(err) = runnable.maintenance_tick().await {
                        tracing::warn!(error = %err, "scheduler maintenance tick failed");
                    }
                }
            });

            match handle.await {
                Ok(()) => {
                    tracing::warn!("scheduler maintenance task exited unexpectedly");
                }
                Err(err) => {
                    if err.is_panic() {
                        tracing::error!("scheduler maintenance task panicked; restarting");
                    } else {
                        tracing::warn!("scheduler maintenance task aborted; restarting");
                    }
                }
            }

            tokio::time::sleep(restart_backoff).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_config_default() {
        let config = WorkerConfig::default();
        assert_eq!(config.concurrency, 1);
        assert_eq!(config.maintenance_interval, Duration::from_millis(200));
    }
}
