//! Worker management for the Engine.
//!
//! This module provides supervised worker spawning and maintenance task management.
//! Workers are automatically restarted on failure with backoff.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{Notify, watch};

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
    fn ready_notifier(&self) -> Arc<Notify>;
}

/// Lifecycle handle for a supervised worker pool.
pub struct WorkerGroup {
    shutdown_tx: watch::Sender<bool>,
    handles: Mutex<Option<Vec<tokio::task::JoinHandle<()>>>>,
}

impl WorkerGroup {
    /// Stop every worker and wait for its supervisor to exit.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        let handles = match self.handles.lock() {
            Ok(mut handles) => handles.take(),
            Err(poisoned) => {
                tracing::error!("worker handle lock poisoned during shutdown");
                poisoned.into_inner().take()
            }
        };
        if let Some(handles) = handles {
            for handle in handles {
                if let Err(error) = handle.await {
                    tracing::warn!(%error, "worker supervisor did not stop cleanly");
                }
            }
        }
    }
}

impl Drop for WorkerGroup {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        let handles = match self.handles.lock() {
            Ok(mut handles) => handles.take(),
            Err(poisoned) => {
                tracing::error!("worker handle lock poisoned during drop");
                poisoned.into_inner().take()
            }
        };
        if let Some(handles) = handles {
            for handle in handles {
                handle.abort();
            }
        }
    }
}

/// Spawn supervised workers for the given runnable.
pub fn spawn_workers<R: Runnable>(runnable: R, config: &WorkerConfig) -> WorkerGroup {
    let notify = runnable.ready_notifier();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut handles = Vec::with_capacity(config.concurrency + 1);

    for i in 0..config.concurrency {
        handles.push(spawn_worker_supervised(
            runnable.clone(),
            notify.clone(),
            format!("engine-{i}"),
            config.restart_backoff,
            config.error_backoff,
            shutdown_rx.clone(),
        ));
    }

    handles.push(spawn_maintenance_supervised(
        runnable,
        config.maintenance_interval,
        config.restart_backoff,
        shutdown_rx,
    ));
    WorkerGroup {
        shutdown_tx,
        handles: Mutex::new(Some(handles)),
    }
}

/// Spawn a supervised worker that restarts on failure.
fn spawn_worker_supervised<R: Runnable>(
    runnable: R,
    notify: Arc<Notify>,
    worker_id: String,
    restart_backoff: Duration,
    error_backoff: Duration,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let runnable = runnable.clone();
            let notify = notify.clone();
            let worker_inner = worker_id.clone();
            let mut handle = tokio::spawn(async move {
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

            let outcome = tokio::select! {
                outcome = &mut handle => Some(outcome),
                _ = shutdown.changed() => None,
            };
            let Some(outcome) = outcome else {
                handle.abort();
                if let Err(error) = handle.await
                    && !error.is_cancelled()
                {
                    tracing::warn!(worker = %worker_id, %error, "worker task failed during shutdown");
                }
                break;
            };
            match outcome {
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

            tokio::select! {
                () = tokio::time::sleep(restart_backoff) => {}
                _ = shutdown.changed() => break,
            }
        }
    })
}

/// Spawn a supervised maintenance task that restarts on failure.
fn spawn_maintenance_supervised<R: Runnable>(
    runnable: R,
    maintenance_interval: Duration,
    restart_backoff: Duration,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let runnable = runnable.clone();
            let mut handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(maintenance_interval);
                loop {
                    interval.tick().await;
                    if let Err(err) = runnable.maintenance_tick().await {
                        tracing::warn!(error = %err, "scheduler maintenance tick failed");
                    }
                }
            });

            let outcome = tokio::select! {
                outcome = &mut handle => Some(outcome),
                _ = shutdown.changed() => None,
            };
            let Some(outcome) = outcome else {
                handle.abort();
                if let Err(error) = handle.await
                    && !error.is_cancelled()
                {
                    tracing::warn!(%error, "maintenance task failed during shutdown");
                }
                break;
            };
            match outcome {
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

            tokio::select! {
                () = tokio::time::sleep(restart_backoff) => {}
                _ = shutdown.changed() => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_worker_config_default() {
        let config = WorkerConfig::default();
        assert_eq!(config.concurrency, 1);
        assert_eq!(config.maintenance_interval, Duration::from_millis(200));
    }

    #[derive(Clone, Default)]
    struct CountingRunnable {
        runs: Arc<AtomicUsize>,
        notify: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl Runnable for CountingRunnable {
        async fn run_one(&self, _worker_id: &str) -> Result<bool, String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        }

        async fn maintenance_tick(&self) -> Result<(), String> {
            Ok(())
        }

        fn ready_notifier(&self) -> Arc<Notify> {
            Arc::clone(&self.notify)
        }
    }

    #[tokio::test]
    async fn shutdown_joins_workers_and_prevents_more_ticks() {
        let runnable = CountingRunnable::default();
        let group = spawn_workers(
            runnable.clone(),
            &WorkerConfig {
                concurrency: 1,
                maintenance_interval: Duration::from_secs(60),
                ..WorkerConfig::default()
            },
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while runnable.runs.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("worker started");

        group.shutdown().await;
        let runs_after_shutdown = runnable.runs.load(Ordering::SeqCst);
        runnable.notify.notify_waiters();
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(runnable.runs.load(Ordering::SeqCst), runs_after_shutdown);
    }
}
