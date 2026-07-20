use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

const fn default_queue_maintenance_interval_ms() -> u64 {
    200
}

/// Queue configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    #[serde(default = "default_queue_workers")]
    pub workers: usize,
    #[serde(default = "default_queue_maintenance_interval_ms")]
    pub maintenance_interval_ms: u64,
}

const fn default_queue_workers() -> usize {
    6
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            workers: default_queue_workers(),
            maintenance_interval_ms: default_queue_maintenance_interval_ms(),
        }
    }
}

impl QueueConfig {
    pub fn validate(&self) -> Result<()> {
        if self.workers == 0 {
            bail!("Queue workers must be > 0");
        }

        if self.maintenance_interval_ms == 0 {
            bail!("Queue maintenance_interval_ms must be > 0");
        }

        Ok(())
    }
}
