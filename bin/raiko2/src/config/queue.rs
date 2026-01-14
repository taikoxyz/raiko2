use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

fn default_queue_namespace() -> String {
    "raiko2:queue".to_string()
}

const fn default_queue_maintenance_interval_ms() -> u64 {
    200
}

const fn default_queue_retry_max_attempts() -> u32 {
    3
}

const fn default_queue_retry_fixed_delay_ms() -> u64 {
    1_000
}

const fn default_queue_retry_base_delay_ms() -> u64 {
    1_000
}

const fn default_queue_retry_max_delay_ms() -> u64 {
    30_000
}

/// Queue retry strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RetryStrategy {
    None,
    Fixed,
    #[default]
    Exponential,
}

impl std::str::FromStr for RetryStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "none" => Ok(RetryStrategy::None),
            "fixed" => Ok(RetryStrategy::Fixed),
            "exponential" | "exp" => Ok(RetryStrategy::Exponential),
            _ => Err(format!("Unknown retry strategy: {s}")),
        }
    }
}

/// Queue retry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default)]
    pub strategy: RetryStrategy,
    #[serde(default = "default_queue_retry_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_queue_retry_fixed_delay_ms")]
    pub fixed_delay_ms: u64,
    #[serde(default = "default_queue_retry_base_delay_ms")]
    pub base_delay_ms: u64,
    #[serde(default = "default_queue_retry_max_delay_ms")]
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            strategy: RetryStrategy::default(),
            max_attempts: default_queue_retry_max_attempts(),
            fixed_delay_ms: default_queue_retry_fixed_delay_ms(),
            base_delay_ms: default_queue_retry_base_delay_ms(),
            max_delay_ms: default_queue_retry_max_delay_ms(),
        }
    }
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
            _ => Err(format!("Unknown queue backend: {}", s)),
        }
    }
}

/// Queue configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub retry: RetryConfig,
}

const fn default_queue_workers() -> usize {
    1
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            backend: QueueBackend::Memory,
            namespace: default_queue_namespace(),
            redis_url: None,
            workers: default_queue_workers(),
            maintenance_interval_ms: default_queue_maintenance_interval_ms(),
            retry: RetryConfig::default(),
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

        if self.retry.strategy != RetryStrategy::None && self.retry.max_attempts == 0 {
            bail!("Queue retry max_attempts must be > 0 when retry is enabled");
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
