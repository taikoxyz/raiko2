use std::time::Duration;

use alloy_primitives::U256;
use raiko2_primitives::{RaikoError, RaikoResult};

use super::Submission;
use crate::boundless_config::MIN_REBID_TIMEOUT_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetryDirective {
    ReuseRequestId,
    RotateRequestId,
}

#[derive(Debug)]
pub(super) struct BiddingSession {
    attempt: u64,
    max_attempts: u32,
    next_request_id: Option<U256>,
    previous_max_price_wei: Option<U256>,
    last_retry_reason: Option<String>,
}

impl BiddingSession {
    pub(super) const fn new(max_attempts: u32) -> Self {
        Self::at_attempt(1, max_attempts)
    }

    pub(super) const fn at_attempt(attempt: u64, max_attempts: u32) -> Self {
        Self {
            attempt,
            max_attempts,
            next_request_id: None,
            previous_max_price_wei: None,
            last_retry_reason: None,
        }
    }

    pub(super) const fn attempt(&self) -> u64 {
        self.attempt
    }

    pub(super) const fn next_request_id(&self) -> Option<U256> {
        self.next_request_id
    }

    pub(super) const fn previous_max_price_wei(&self) -> Option<U256> {
        self.previous_max_price_wei
    }

    pub(super) fn ensure_submission_budget(&self) -> RaikoResult<()> {
        if !exceeds_submission_budget(self.attempt, self.max_attempts) {
            return Ok(());
        }

        let detail = self
            .last_retry_reason
            .as_deref()
            .map(|reason| format!("; last attempt: {reason}"))
            .unwrap_or_default();
        Err(RaikoError::Guest(format!(
            "Exhausted Boundless submission budget of {} attempts \
             (rebid_max_attempts = {}){detail}",
            u64::from(self.max_attempts) + 1,
            self.max_attempts,
        )))
    }

    pub(super) fn record_retry(
        &mut self,
        submission: &Submission,
        directive: RetryDirective,
        reason: impl Into<String>,
    ) {
        self.next_request_id = match directive {
            RetryDirective::ReuseRequestId => Some(submission.market_request_id),
            RetryDirective::RotateRequestId => None,
        };
        self.previous_max_price_wei = Some(submission.max_price_wei);
        self.last_retry_reason = Some(reason.into());
        self.attempt = self.attempt.max(submission.attempt).saturating_add(1);
    }

    pub(super) fn no_lock_timeout(
        &self,
        rebid_timeout_ms: u64,
        price_pinned_at_ceiling: bool,
    ) -> NoLockTimeout {
        let mut timeout =
            no_lock_timeout_for_attempt(self.attempt, rebid_timeout_ms, self.max_attempts);
        if price_pinned_at_ceiling {
            // A ceiling-pinned bid cannot be raised by any later rung: the previous-max floor
            // keeps rebid prices at or above this bid and the ceiling clamps them back to exactly
            // it. Rebidding would resubmit the identical price while restarting the offer's ramp
            // (temporarily offering the market less than the live request already does) and, on
            // the onchain path, pay gas per rung. Treat the rung as final instead: wait out the
            // payable window, then give up.
            timeout.action = NoLockTimeoutAction::Abort;
        }
        timeout
    }
}

/// Whether `max_price_wei` already sits at the configured absolute ceiling, meaning no later
/// rung can bid higher. Zero (a legacy resume record whose exact bid is unknown) is never
/// pinned, and a flat ladder (`step_bps == 0`) is excluded: identical-price rebids are its
/// explicit, opted-into design.
pub(super) fn price_pinned_at_ceiling(
    max_price_wei: U256,
    ceiling_wei: Option<U256>,
    step_bps: u32,
) -> bool {
    step_bps > 0
        && !max_price_wei.is_zero()
        && ceiling_wei.is_some_and(|ceiling| max_price_wei >= ceiling)
}

/// Retry directive for a failure the market has NOT confirmed (poll timeout, read error, RPC
/// failure). Always reuses the request id: the market pays at most once per id, so reuse keeps
/// double-pay impossible even when the request's true state is unknown. The clock and expiry are
/// accepted — and deliberately ignored — to document that this decision must never regress to a
/// local-time heuristic: only a successful market read proving `MarketExpired`/`LockExpired`
/// rotates, and local `now >= expires_at` is exactly the read-lag shortcut that used to reopen
/// the double-pay window.
pub(super) const fn retry_directive_for_unconfirmed_failure(
    _now_secs: u64,
    _expires_at: u64,
) -> RetryDirective {
    RetryDirective::ReuseRequestId
}

/// Number of rebid rungs applied at 1-based `attempt`, capped at `max_attempts`.
pub(super) fn escalation_rungs(attempt: u64, max_attempts: u32) -> u64 {
    attempt.saturating_sub(1).min(u64::from(max_attempts))
}

/// Compound `base` by `step_bps` basis points per rebid rung, applied iteratively in U256 so the
/// magnitude stays bounded. `step_bps == 0` is a flat (no-escalation) ladder.
pub(super) fn escalated_price(
    base: U256,
    attempt: u64,
    step_bps: u32,
    max_attempts: u32,
) -> RaikoResult<U256> {
    let rungs = escalation_rungs(attempt, max_attempts);
    let numer = U256::from(10_000u64 + u64::from(step_bps));
    let denom = U256::from(10_000u64);
    let mut price = base;
    for _ in 0..rungs {
        price = price
            .checked_mul(numer)
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig(format!(
                    "Boundless offer price {base} wei overflows escalating {step_bps} bps over {rungs} rebid rungs"
                ))
            })?
            / denom;
    }
    Ok(price)
}

/// Floored effective multiplier at `attempt`, for progress/metadata display only. Never used to
/// price an offer — pricing goes through [`escalated_price`] on the real base.
pub(super) fn effective_price_multiplier(attempt: u64, step_bps: u32, max_attempts: u32) -> u32 {
    const SCALE: u64 = 1_000_000;
    let scaled = escalated_price(U256::from(SCALE), attempt, step_bps, max_attempts)
        .unwrap_or(U256::from(SCALE));
    u32::try_from(scaled / U256::from(SCALE)).unwrap_or(u32::MAX)
}

/// Attempt number to resume a stored submission at. Records carry the real 1-based attempt.
///
/// Records written before `rebid_attempt` was persisted deserialize with `attempt == 0`. The first
/// release carrying that field is also the first to resume such records, so on that one upgrade
/// every in-flight submission would otherwise reset to attempt 1 — re-granting the full rebid budget
/// and rebidding below prices the market already declined. The reconstruction inverts the old
/// `max_price_multiplier = base_multiplier^(attempt - 1)` ladder as `1 + log2(max_price_multiplier)`,
/// which is exact only for the ×2 default ladder (`rebid_price_multiplier = 2`) — the multiplier
/// every known deployment ran, as the live k8s fleet never set a custom value. A legacy record from a
/// non-default ladder (multiplier 3, 4, …) resolves to a conservative upper bound on the attempt:
/// `log2` over-counts the rungs, so the resumed attempt is no smaller than the real one, leaving
/// fewer remaining rebids at higher prices — never more. Post-upgrade records always carry a real
/// `attempt`, so this branch is dead after one deploy.
pub(super) fn resume_attempt(submission: &Submission) -> u64 {
    if submission.attempt > 0 {
        return submission.attempt;
    }
    // TODO: delete this legacy branch, its tests, and this comment after one release carrying
    // rebid_attempt has shipped — records with attempt == 0 cannot exist beyond that deploy.
    1 + u64::from(submission.max_price_multiplier.max(1).ilog2())
}

pub(super) const fn should_rebid_unlocked_request(attempt: u64, max_attempts: u32) -> bool {
    attempt > 0 && attempt <= max_attempts as u64
}

/// Total market submissions allowed per proof task: the initial attempt plus `max_attempts`
/// rebids. The no-lock path already enforces this bound through [`NoLockTimeoutAction::Abort`];
/// this check extends the same budget to the `Expired` and poll-timeout retry paths, which would
/// otherwise mint replacement requests without limit.
pub(super) const fn exceeds_submission_budget(attempt: u64, max_attempts: u32) -> bool {
    attempt > max_attempts as u64 + 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NoLockTimeoutAction {
    Rebid,
    Abort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NoLockTimeout {
    pub(super) delay: Duration,
    pub(super) action: NoLockTimeoutAction,
}

pub(super) fn no_lock_timeout_for_attempt(
    attempt: u64,
    rebid_timeout_ms: u64,
    max_attempts: u32,
) -> NoLockTimeout {
    let action = if should_rebid_unlocked_request(attempt, max_attempts) {
        NoLockTimeoutAction::Rebid
    } else {
        NoLockTimeoutAction::Abort
    };
    NoLockTimeout {
        delay: Duration::from_millis(rebid_timeout_ms.max(MIN_REBID_TIMEOUT_MS)),
        action,
    }
}

/// Wall-clock deadline after which an unlocked submission is given up on.
///
/// Rebid attempts use the configured rebid delay: the request is abandoned early in favor of a
/// higher-priced resubmission. The final attempt (`Abort`) instead waits out the offer's lock
/// deadline when known: the market pays nothing for fulfillments past `lock_expires_at`, so that
/// is the exact end of the payable window — aborting sooner walks away from a live request the
/// client could still be charged for, and waiting longer cannot yield a paid fulfillment.
pub(super) const fn no_lock_deadline(
    submitted_at: u64,
    lock_expires_at: u64,
    timeout: NoLockTimeout,
) -> u64 {
    let rebid_deadline = submitted_at.saturating_add(timeout.delay.as_secs());
    match timeout.action {
        NoLockTimeoutAction::Rebid => rebid_deadline,
        // `lock_expires_at == 0` means a legacy resume record; keep the old rebid-delay behavior.
        NoLockTimeoutAction::Abort => {
            if lock_expires_at > 0 {
                lock_expires_at
            } else {
                rebid_deadline
            }
        }
    }
}

#[cfg(test)]
pub(super) const fn no_lock_deadline_elapsed(
    submitted_at: u64,
    lock_expires_at: u64,
    timeout: NoLockTimeout,
    now_secs: u64,
) -> bool {
    now_secs >= no_lock_deadline(submitted_at, lock_expires_at, timeout)
}

/// Whether the overall poll timeout must be deferred for the current submission.
///
/// On the final attempt (`Abort`) the request stays payable until its lock deadline, so the overall
/// timeout only takes effect once the payable window has closed. The deferral is bounded by
/// `expires_at` so a corrupt stored record cannot keep the poll loop open forever.
#[cfg(test)]
pub(super) const fn defer_poll_timeout_while_payable(
    submitted_at: u64,
    lock_expires_at: u64,
    expires_at: u64,
    timeout: NoLockTimeout,
    now_secs: u64,
) -> bool {
    matches!(timeout.action, NoLockTimeoutAction::Abort)
        && now_secs < expires_at
        && !no_lock_deadline_elapsed(submitted_at, lock_expires_at, timeout, now_secs)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;

    use super::{BiddingSession, RetryDirective, retry_directive_for_unconfirmed_failure};
    use crate::boundless::Submission;

    fn test_submission(attempt: u64, max_price_wei: u64) -> Submission {
        Submission {
            market_request_id: U256::from(1u64),
            provider_request_id: "0x1".to_string(),
            remote_tx_hash: None,
            expires_at: 1,
            lock_expires_at: 1,
            submitted_at: 1,
            max_price_multiplier: 1,
            max_price_wei: U256::from(max_price_wei),
            attempt,
        }
    }

    #[test]
    fn retry_reuses_request_id_until_market_proves_it_dead() {
        let mut session = BiddingSession::new(4);
        let submitted = test_submission(1, 100);
        session.record_retry(&submitted, RetryDirective::ReuseRequestId, "unlocked");
        assert_eq!(session.next_request_id(), Some(submitted.market_request_id));
        assert_eq!(
            session.previous_max_price_wei(),
            Some(submitted.max_price_wei)
        );
        assert_eq!(session.attempt(), 2);
    }

    #[test]
    fn retry_rotates_only_for_dead_request() {
        let mut session = BiddingSession::new(4);
        let submitted = test_submission(1, 100);
        session.record_retry(&submitted, RetryDirective::RotateRequestId, "expired");
        assert_eq!(session.next_request_id(), None);
        assert_eq!(session.attempt(), 2);
    }

    #[test]
    fn timeout_and_read_errors_reuse_request_id_after_local_expiry() {
        assert_eq!(
            retry_directive_for_unconfirmed_failure(101, 100),
            RetryDirective::ReuseRequestId
        );
    }

    #[test]
    fn exhausted_session_rejects_another_submission() {
        let session = BiddingSession::at_attempt(2, 0);
        assert!(session.ensure_submission_budget().is_err());
    }

    #[test]
    fn pinned_price_requires_ceiling_and_escalating_ladder() {
        use super::price_pinned_at_ceiling;

        assert!(!price_pinned_at_ceiling(U256::from(100), None, 5_000));
        // A legacy resume record stores zero for "exact bid unknown"; never pinned.
        assert!(!price_pinned_at_ceiling(
            U256::ZERO,
            Some(U256::from(100)),
            5_000
        ));
        assert!(!price_pinned_at_ceiling(
            U256::from(99),
            Some(U256::from(100)),
            5_000
        ));
        // Flat ladders rebid the same price by design.
        assert!(!price_pinned_at_ceiling(
            U256::from(100),
            Some(U256::from(100)),
            0
        ));
        assert!(price_pinned_at_ceiling(
            U256::from(100),
            Some(U256::from(100)),
            5_000
        ));
        assert!(price_pinned_at_ceiling(
            U256::from(101),
            Some(U256::from(100)),
            5_000
        ));
    }

    #[test]
    fn pinned_price_forces_final_attempt_semantics() {
        use super::NoLockTimeoutAction;

        let session = BiddingSession::at_attempt(2, 4);
        assert_eq!(
            session.no_lock_timeout(300_000, false).action,
            NoLockTimeoutAction::Rebid
        );
        assert_eq!(
            session.no_lock_timeout(300_000, true).action,
            NoLockTimeoutAction::Abort
        );
    }
}
