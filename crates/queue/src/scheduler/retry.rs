use std::time::Duration;

#[derive(Clone, Debug)]
pub enum RetryPolicy {
    None,
    Fixed {
        max_attempts: u32,
        delay: Duration,
    },
    Exponential {
        max_attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
    },
}

impl RetryPolicy {
    pub(crate) fn retry_delay(&self, attempt: u32) -> Option<Duration> {
        match self {
            RetryPolicy::None => None,
            RetryPolicy::Fixed {
                max_attempts,
                delay,
            } => {
                if attempt >= *max_attempts {
                    None
                } else {
                    Some(*delay)
                }
            }
            RetryPolicy::Exponential {
                max_attempts,
                base_delay,
                max_delay,
            } => {
                if attempt >= *max_attempts {
                    return None;
                }

                let exponent = attempt.saturating_sub(1);
                let base_ms = base_delay.as_millis().min(u64::MAX as u128) as u64;
                let max_ms = max_delay.as_millis().min(u64::MAX as u128) as u64;
                let factor = 1u64.checked_shl(exponent).unwrap_or(u64::MAX);
                let delay_ms = base_ms.saturating_mul(factor).min(max_ms);
                Some(Duration::from_millis(delay_ms))
            }
        }
    }
}
