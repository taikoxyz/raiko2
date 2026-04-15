use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::RetryPolicy;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskExecutionPolicy {
    pub lease_duration: Duration,
    pub retry: RetryPolicy,
}

impl TaskExecutionPolicy {
    #[must_use]
    pub const fn with_retry_limit(&self, max_attempts: u32) -> Self {
        let retry = match &self.retry {
            RetryPolicy::None => RetryPolicy::None,
            RetryPolicy::Fixed { delay, .. } => RetryPolicy::Fixed {
                max_attempts,
                delay: *delay,
            },
            RetryPolicy::Exponential {
                base_delay,
                max_delay,
                ..
            } => RetryPolicy::Exponential {
                max_attempts,
                base_delay: *base_delay,
                max_delay: *max_delay,
            },
        };
        Self {
            lease_duration: self.lease_duration,
            retry,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    pub lease_duration: Duration,
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
            retry: RetryPolicy::None,
        }
    }
}
