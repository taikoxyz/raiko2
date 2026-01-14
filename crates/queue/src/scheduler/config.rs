use std::time::Duration;

use super::RetryPolicy;

#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    pub lease_duration: Duration,
    pub retry: RetryPolicy,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        }
    }
}
