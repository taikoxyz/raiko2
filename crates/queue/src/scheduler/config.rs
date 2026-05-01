use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::RetryPolicy;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskExecutionPolicy {
    pub lease_duration: Duration,
    pub retry: RetryPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchedulerConfig {
    pub lease_duration: Duration,
    pub task_timeout: Duration,
    pub retry: RetryPolicy,
}

impl SchedulerConfig {
    #[must_use]
    pub fn execution_policy(&self) -> TaskExecutionPolicy {
        TaskExecutionPolicy {
            lease_duration: self.lease_duration,
            retry: self.retry.clone(),
        }
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            lease_duration: Duration::from_secs(60),
            task_timeout: Duration::from_secs(60),
            retry: RetryPolicy::None,
        }
    }
}
