use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

fn default_queue_namespace() -> String {
    "raiko2:queue".to_string()
}

const fn default_queue_maintenance_interval_ms() -> u64 {
    200
}

/// Queue backend type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QueueBackend {
    #[default]
    Memory,
    Redis,
}

impl std::str::FromStr for QueueBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "memory" => Ok(QueueBackend::Memory),
            "redis" => Ok(QueueBackend::Redis),
            _ => Err(format!("Unknown queue backend: {s}")),
        }
    }
}

/// Queue configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    #[serde(default)]
    pub backend: QueueBackend,
    #[serde(default = "default_queue_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub redis_url: Option<String>,
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
            backend: QueueBackend::Memory,
            namespace: default_queue_namespace(),
            redis_url: None,
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

        match self.backend {
            QueueBackend::Memory => Ok(()),
            QueueBackend::Redis => {
                if self.redis_url.as_deref().unwrap_or_default().is_empty() {
                    bail!("Redis queue backend requires redis_url to be set");
                }
                if self.namespace.is_empty() {
                    bail!("Redis queue backend requires namespace to be non-empty");
                }
                Ok(())
            }
        }
    }
}
