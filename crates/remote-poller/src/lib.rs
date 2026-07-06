#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::redundant_pub_crate)]

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{StreamExt, stream::FuturesUnordered};
use raiko2_primitives::ProofType;
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot},
    time::{MissedTickBehavior, interval, timeout},
};
use uuid::Uuid;

const MIN_POLL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct RemoteStatusTracker {
    command_tx: mpsc::UnboundedSender<TrackerCommand>,
}

impl RemoteStatusTracker {
    #[must_use]
    pub fn spawn(
        config: RemotePollerConfig,
        sources: HashMap<ProofType, Arc<dyn RemoteStatusSource>>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let actor = TrackerActor::new(config, sources, command_rx);
        tokio::spawn(actor.run());
        Self { command_tx }
    }

    pub fn register(
        &self,
        submission: RemoteSubmission,
        terminal_tx: oneshot::Sender<RemoteTerminalResult>,
    ) -> Result<(), RegisterError> {
        self.command_tx
            .send(TrackerCommand::Register {
                submission,
                terminal_tx,
            })
            .map_err(|_| RegisterError::Closed)
    }

    pub fn untrack(&self, submission_id: RemoteSubmissionId) {
        let _ = self
            .command_tx
            .send(TrackerCommand::Untrack { submission_id });
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RemotePollerConfig {
    pub poll_interval: Duration,
    pub poll_timeout: Duration,
}

impl RemotePollerConfig {
    #[must_use]
    pub fn new(poll_interval: Duration) -> Self {
        Self {
            poll_interval,
            poll_timeout: poll_interval.saturating_mul(2).max(MIN_POLL_TIMEOUT),
        }
    }
}

#[async_trait::async_trait]
pub trait RemoteStatusSource: Send + Sync + 'static {
    async fn poll(
        &self,
        proof_type: ProofType,
        submissions: Vec<RemoteSubmission>,
    ) -> Result<Vec<RemoteSubmissionStatus>, RemotePollError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSubmission {
    pub id: RemoteSubmissionId,
    pub proof_type: ProofType,
    pub provider_request_id: String,
    pub timeout_at: Option<Instant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSubmissionStatus {
    pub submission_id: RemoteSubmissionId,
    pub status: RemoteStatus,
    pub reason: Option<RemoteStatusReason>,
    pub observed_unix_secs: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RemoteSubmissionId(Uuid);

impl RemoteSubmissionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RemoteSubmissionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RemoteSubmissionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteStatus {
    Pending,
    Locked,
    Fulfilled,
    Failed,
    Expired,
    Unrecoverable,
}

impl RemoteStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Fulfilled | Self::Failed | Self::Expired | Self::Unrecoverable
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteTerminalResult {
    Fulfilled {
        submission_id: RemoteSubmissionId,
        observed_unix_secs: u64,
    },
    Failed {
        submission_id: RemoteSubmissionId,
        reason: RemoteStatusReason,
        observed_unix_secs: u64,
    },
    Expired {
        submission_id: RemoteSubmissionId,
        reason: RemoteStatusReason,
        observed_unix_secs: u64,
    },
    TimedOut {
        submission_id: RemoteSubmissionId,
        reason: RemoteStatusReason,
    },
    Unrecoverable {
        submission_id: RemoteSubmissionId,
        reason: RemoteStatusReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteStatusReason {
    pub message: String,
}

impl RemoteStatusReason {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum RemotePollError {
    #[error("remote status source transient error: {0}")]
    Transient(String),
    #[error("remote status source unavailable: {0}")]
    SourceUnavailable(String),
}

#[derive(Debug, Error)]
pub enum RegisterError {
    #[error("remote status tracker is closed")]
    Closed,
}

type PollFuture = Pin<Box<dyn Future<Output = PollCompletion> + Send>>;

enum TrackerCommand {
    Register {
        submission: RemoteSubmission,
        terminal_tx: oneshot::Sender<RemoteTerminalResult>,
    },
    Untrack {
        submission_id: RemoteSubmissionId,
    },
}

struct TrackedSubmission {
    submission: RemoteSubmission,
    terminal_tx: oneshot::Sender<RemoteTerminalResult>,
}

struct PollCompletion {
    proof_type: ProofType,
    selected_ids: BTreeSet<RemoteSubmissionId>,
    result: Result<Vec<RemoteSubmissionStatus>, RemotePollError>,
}

struct TrackerActor {
    config: RemotePollerConfig,
    sources: HashMap<ProofType, Arc<dyn RemoteStatusSource>>,
    command_rx: mpsc::UnboundedReceiver<TrackerCommand>,
    active: BTreeMap<RemoteSubmissionId, TrackedSubmission>,
    buckets: HashMap<ProofType, BTreeSet<RemoteSubmissionId>>,
    in_flight_proof_types: BTreeSet<ProofType>,
    in_flight: FuturesUnordered<PollFuture>,
}

impl TrackerActor {
    fn new(
        config: RemotePollerConfig,
        sources: HashMap<ProofType, Arc<dyn RemoteStatusSource>>,
        command_rx: mpsc::UnboundedReceiver<TrackerCommand>,
    ) -> Self {
        Self {
            config,
            sources,
            command_rx,
            active: BTreeMap::new(),
            buckets: HashMap::new(),
            in_flight_proof_types: BTreeSet::new(),
            in_flight: FuturesUnordered::new(),
        }
    }

    async fn run(mut self) {
        let mut poll_interval = interval(self.config.poll_interval);
        poll_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut commands_open = true;

        loop {
            if !commands_open && self.active.is_empty() && self.in_flight.is_empty() {
                break;
            }
            tokio::select! {
                command = self.command_rx.recv(), if commands_open => {
                    match command {
                        Some(command) => self.handle_command(command),
                        None => commands_open = false,
                    }
                }
                _ = poll_interval.tick(), if !self.active.is_empty() => {
                    let now = Instant::now();
                    self.prune_closed_and_timed_out(now);
                    self.schedule_polls(now);
                }
                Some(completion) = self.in_flight.next(), if !self.in_flight.is_empty() => {
                    self.handle_poll_completion(completion);
                }
            }
        }
    }

    fn handle_command(&mut self, command: TrackerCommand) {
        match command {
            TrackerCommand::Register {
                submission,
                terminal_tx,
            } => self.register(submission, terminal_tx),
            TrackerCommand::Untrack { submission_id } => self.remove_submission(submission_id),
        }
    }

    fn register(
        &mut self,
        submission: RemoteSubmission,
        terminal_tx: oneshot::Sender<RemoteTerminalResult>,
    ) {
        if self.active.contains_key(&submission.id) {
            let _ = terminal_tx.send(RemoteTerminalResult::Unrecoverable {
                submission_id: submission.id,
                reason: RemoteStatusReason::new("duplicate remote submission id"),
            });
            return;
        }

        if !self.sources.contains_key(&submission.proof_type) {
            let _ = terminal_tx.send(RemoteTerminalResult::Unrecoverable {
                submission_id: submission.id,
                reason: RemoteStatusReason::new(format!(
                    "no remote status source registered for proof type {}",
                    submission.proof_type
                )),
            });
            return;
        }

        let submission_id = submission.id;
        let proof_type = submission.proof_type;
        self.active.insert(
            submission_id,
            TrackedSubmission {
                submission,
                terminal_tx,
            },
        );
        self.buckets
            .entry(proof_type)
            .or_default()
            .insert(submission_id);
    }

    fn schedule_polls(&mut self, now: Instant) {
        let proof_types = self.buckets.keys().copied().collect::<Vec<_>>();
        for proof_type in proof_types {
            if self.in_flight_proof_types.contains(&proof_type) {
                continue;
            }

            let submissions = self.collect_bucket_submissions(proof_type, now);
            if submissions.is_empty() {
                if self.buckets.get(&proof_type).is_none_or(BTreeSet::is_empty) {
                    self.buckets.remove(&proof_type);
                }
                continue;
            }

            let Some(source) = self.sources.get(&proof_type).cloned() else {
                self.fail_submissions_unrecoverable(
                    submissions.iter().map(|submission| submission.id),
                    "remote status source disappeared",
                );
                continue;
            };

            let selected_ids = submissions
                .iter()
                .map(|submission| submission.id)
                .collect::<BTreeSet<_>>();
            let poll_timeout = self.config.poll_timeout;
            self.in_flight_proof_types.insert(proof_type);
            let mut poll_task =
                tokio::spawn(async move { source.poll(proof_type, submissions).await });
            self.in_flight.push(Box::pin(async move {
                let result = match timeout(poll_timeout, &mut poll_task).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(err)) if err.is_panic() => Err(RemotePollError::SourceUnavailable(
                        format!("remote status poll panicked: {err}"),
                    )),
                    Ok(Err(err)) => Err(RemotePollError::SourceUnavailable(format!(
                        "remote status poll task failed: {err}"
                    ))),
                    Err(_) => {
                        poll_task.abort();
                        Err(RemotePollError::SourceUnavailable(format!(
                            "remote status poll exceeded timeout {poll_timeout:?}"
                        )))
                    }
                };
                PollCompletion {
                    proof_type,
                    selected_ids,
                    result,
                }
            }));
        }
    }

    fn collect_bucket_submissions(
        &mut self,
        proof_type: ProofType,
        now: Instant,
    ) -> Vec<RemoteSubmission> {
        let ids = self
            .buckets
            .get(&proof_type)
            .into_iter()
            .flat_map(|ids| ids.iter().copied())
            .collect::<Vec<_>>();
        let mut submissions = Vec::new();

        for submission_id in ids {
            let Some(tracked) = self.active.get(&submission_id) else {
                self.remove_from_bucket(proof_type, submission_id);
                continue;
            };

            if tracked.terminal_tx.is_closed() {
                self.remove_submission(submission_id);
                continue;
            }

            if tracked
                .submission
                .timeout_at
                .is_some_and(|timeout_at| now >= timeout_at)
            {
                let terminal = RemoteTerminalResult::TimedOut {
                    submission_id,
                    reason: RemoteStatusReason::new("remote submission timed out"),
                };
                self.send_terminal(submission_id, terminal);
                continue;
            }

            submissions.push(tracked.submission.clone());
        }

        submissions
    }

    fn prune_closed_and_timed_out(&mut self, now: Instant) {
        let submission_ids = self.active.keys().copied().collect::<Vec<_>>();
        for submission_id in submission_ids {
            let Some(tracked) = self.active.get(&submission_id) else {
                continue;
            };
            if tracked.terminal_tx.is_closed() {
                self.remove_submission(submission_id);
                continue;
            }
            if tracked
                .submission
                .timeout_at
                .is_some_and(|timeout_at| now >= timeout_at)
            {
                let terminal = RemoteTerminalResult::TimedOut {
                    submission_id,
                    reason: RemoteStatusReason::new("remote submission timed out"),
                };
                self.send_terminal(submission_id, terminal);
            }
        }
    }

    fn handle_poll_completion(&mut self, completion: PollCompletion) {
        self.in_flight_proof_types.remove(&completion.proof_type);
        match completion.result {
            Ok(statuses) => {
                self.handle_statuses(completion.proof_type, &completion.selected_ids, statuses);
            }
            Err(RemotePollError::Transient(error)) => {
                tracing::warn!(
                    proof_type = %completion.proof_type,
                    error,
                    "remote status poll failed; keeping submissions active"
                );
            }
            Err(RemotePollError::SourceUnavailable(error)) => {
                tracing::error!(
                    proof_type = %completion.proof_type,
                    error,
                    "remote status source unavailable; failing selected submissions"
                );
                self.fail_submissions_unrecoverable(completion.selected_ids, error);
            }
        }
    }

    fn handle_statuses(
        &mut self,
        proof_type: ProofType,
        selected_ids: &BTreeSet<RemoteSubmissionId>,
        statuses: Vec<RemoteSubmissionStatus>,
    ) {
        if !valid_status_set(selected_ids, &statuses) {
            tracing::error!(
                proof_type = %proof_type,
                selected = selected_ids.len(),
                returned = statuses.len(),
                "remote status source violated selected submission result contract"
            );
            self.fail_submissions_unrecoverable(
                selected_ids.iter().copied(),
                "remote status source returned a mismatched status set",
            );
            return;
        }

        let mut counts = StatusCounts::default();
        for status in statuses {
            counts.record(status.status);
            if let Some(terminal) = terminal_result(status) {
                self.send_terminal(terminal.submission_id(), terminal);
            }
        }
        tracing::debug!(
            proof_type = %proof_type,
            pending = counts.pending,
            locked = counts.locked,
            fulfilled = counts.fulfilled,
            failed = counts.failed,
            expired = counts.expired,
            unrecoverable = counts.unrecoverable,
            "remote status poll completed"
        );
    }

    fn send_terminal(&mut self, submission_id: RemoteSubmissionId, terminal: RemoteTerminalResult) {
        if let Some(tracked) = self.active.remove(&submission_id) {
            self.remove_from_bucket(tracked.submission.proof_type, submission_id);
            let _ = tracked.terminal_tx.send(terminal);
        }
    }

    fn fail_submissions_unrecoverable(
        &mut self,
        submission_ids: impl IntoIterator<Item = RemoteSubmissionId>,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        for submission_id in submission_ids {
            self.send_terminal(
                submission_id,
                RemoteTerminalResult::Unrecoverable {
                    submission_id,
                    reason: RemoteStatusReason::new(reason.clone()),
                },
            );
        }
    }

    fn remove_submission(&mut self, submission_id: RemoteSubmissionId) {
        if let Some(tracked) = self.active.remove(&submission_id) {
            self.remove_from_bucket(tracked.submission.proof_type, submission_id);
        }
    }

    fn remove_from_bucket(&mut self, proof_type: ProofType, submission_id: RemoteSubmissionId) {
        if let Some(bucket) = self.buckets.get_mut(&proof_type) {
            bucket.remove(&submission_id);
            if bucket.is_empty() {
                self.buckets.remove(&proof_type);
            }
        }
    }
}

impl RemoteTerminalResult {
    #[must_use]
    pub const fn submission_id(&self) -> RemoteSubmissionId {
        match self {
            Self::Fulfilled { submission_id, .. }
            | Self::Failed { submission_id, .. }
            | Self::Expired { submission_id, .. }
            | Self::TimedOut { submission_id, .. }
            | Self::Unrecoverable { submission_id, .. } => *submission_id,
        }
    }
}

fn terminal_result(status: RemoteSubmissionStatus) -> Option<RemoteTerminalResult> {
    match status.status {
        RemoteStatus::Pending | RemoteStatus::Locked => None,
        RemoteStatus::Fulfilled => Some(RemoteTerminalResult::Fulfilled {
            submission_id: status.submission_id,
            observed_unix_secs: status.observed_unix_secs,
        }),
        RemoteStatus::Failed => Some(RemoteTerminalResult::Failed {
            submission_id: status.submission_id,
            reason: status
                .reason
                .unwrap_or_else(|| RemoteStatusReason::new("remote submission failed")),
            observed_unix_secs: status.observed_unix_secs,
        }),
        RemoteStatus::Expired => Some(RemoteTerminalResult::Expired {
            submission_id: status.submission_id,
            reason: status
                .reason
                .unwrap_or_else(|| RemoteStatusReason::new("remote submission expired")),
            observed_unix_secs: status.observed_unix_secs,
        }),
        RemoteStatus::Unrecoverable => Some(RemoteTerminalResult::Unrecoverable {
            submission_id: status.submission_id,
            reason: status
                .reason
                .unwrap_or_else(|| RemoteStatusReason::new("remote submission is unrecoverable")),
        }),
    }
}

fn valid_status_set(
    selected_ids: &BTreeSet<RemoteSubmissionId>,
    statuses: &[RemoteSubmissionStatus],
) -> bool {
    let returned_ids = statuses
        .iter()
        .map(|status| status.submission_id)
        .collect::<BTreeSet<_>>();
    returned_ids.len() == statuses.len() && &returned_ids == selected_ids
}

#[derive(Default)]
struct StatusCounts {
    pending: usize,
    locked: usize,
    fulfilled: usize,
    failed: usize,
    expired: usize,
    unrecoverable: usize,
}

impl StatusCounts {
    const fn record(&mut self, status: RemoteStatus) {
        match status {
            RemoteStatus::Pending => self.pending += 1,
            RemoteStatus::Locked => self.locked += 1,
            RemoteStatus::Fulfilled => self.fulfilled += 1,
            RemoteStatus::Failed => self.failed += 1,
            RemoteStatus::Expired => self.expired += 1,
            RemoteStatus::Unrecoverable => self.unrecoverable += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use tokio::sync::{Mutex, oneshot};

    use super::*;

    #[derive(Default)]
    struct FakeSource {
        calls: AtomicUsize,
        statuses: Mutex<Vec<Vec<RemoteSubmissionStatus>>>,
        delay: Option<Duration>,
        delay_once: Mutex<Option<Duration>>,
        error_once: Mutex<Option<RemotePollError>>,
        panic_once: AtomicBool,
    }

    impl FakeSource {
        fn with_statuses(statuses: Vec<Vec<RemoteSubmissionStatus>>) -> Self {
            Self {
                statuses: Mutex::new(statuses),
                ..Self::default()
            }
        }

        fn with_delay(delay: Duration) -> Self {
            Self {
                delay: Some(delay),
                ..Self::default()
            }
        }

        fn with_delay_once(delay: Duration, statuses: Vec<Vec<RemoteSubmissionStatus>>) -> Self {
            Self {
                delay_once: Mutex::new(Some(delay)),
                statuses: Mutex::new(statuses),
                ..Self::default()
            }
        }

        fn with_panic_once(statuses: Vec<Vec<RemoteSubmissionStatus>>) -> Self {
            Self {
                statuses: Mutex::new(statuses),
                panic_once: AtomicBool::new(true),
                ..Self::default()
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl RemoteStatusSource for FakeSource {
        async fn poll(
            &self,
            _proof_type: ProofType,
            submissions: Vec<RemoteSubmission>,
        ) -> Result<Vec<RemoteSubmissionStatus>, RemotePollError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.panic_once.swap(false, Ordering::SeqCst) {
                panic!("remote status source panic");
            }
            if let Some(delay) = self.delay_once.lock().await.take() {
                tokio::time::sleep(delay).await;
            }
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            if let Some(error) = self.error_once.lock().await.take() {
                return Err(error);
            }
            let mut statuses = self.statuses.lock().await;
            if statuses.is_empty() {
                return Ok(submissions
                    .into_iter()
                    .map(|submission| pending_status(submission.id))
                    .collect());
            }
            Ok(statuses.remove(0))
        }
    }

    fn submission(proof_type: ProofType) -> RemoteSubmission {
        RemoteSubmission {
            id: RemoteSubmissionId::new(),
            proof_type,
            provider_request_id: "request".to_string(),
            timeout_at: None,
        }
    }

    fn pending_status(submission_id: RemoteSubmissionId) -> RemoteSubmissionStatus {
        RemoteSubmissionStatus {
            submission_id,
            status: RemoteStatus::Pending,
            reason: None,
            observed_unix_secs: 1,
        }
    }

    fn fulfilled_status(submission_id: RemoteSubmissionId) -> RemoteSubmissionStatus {
        RemoteSubmissionStatus {
            submission_id,
            status: RemoteStatus::Fulfilled,
            reason: None,
            observed_unix_secs: 1,
        }
    }

    fn tracker_with_source(
        source: Arc<FakeSource>,
        poll_interval: Duration,
    ) -> RemoteStatusTracker {
        tracker_with_config(source, RemotePollerConfig::new(poll_interval))
    }

    fn tracker_with_config(
        source: Arc<FakeSource>,
        config: RemotePollerConfig,
    ) -> RemoteStatusTracker {
        let mut sources: HashMap<ProofType, Arc<dyn RemoteStatusSource>> = HashMap::new();
        sources.insert(ProofType::Sp1, source);
        RemoteStatusTracker::spawn(config, sources)
    }

    #[tokio::test]
    async fn same_proof_type_submissions_share_one_poll_call() {
        let first = submission(ProofType::Sp1);
        let second = submission(ProofType::Sp1);
        let source = Arc::new(FakeSource::with_statuses(vec![vec![
            fulfilled_status(first.id),
            fulfilled_status(second.id),
        ]]));
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();

        tracker.register(first, first_tx).expect("register first");
        tracker
            .register(second, second_tx)
            .expect("register second");

        let first_terminal = tokio::time::timeout(Duration::from_secs(1), first_rx)
            .await
            .expect("first terminal")
            .expect("first sender");
        let second_terminal = tokio::time::timeout(Duration::from_secs(1), second_rx)
            .await
            .expect("second terminal")
            .expect("second sender");

        assert!(matches!(
            first_terminal,
            RemoteTerminalResult::Fulfilled { .. }
        ));
        assert!(matches!(
            second_terminal,
            RemoteTerminalResult::Fulfilled { .. }
        ));
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn transient_poll_error_keeps_submission_active() {
        let submission = submission(ProofType::Sp1);
        let source = Arc::new(FakeSource::with_statuses(vec![vec![fulfilled_status(
            submission.id,
        )]]));
        *source.error_once.lock().await = Some(RemotePollError::Transient("rpc 429".to_string()));
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (tx, rx) = oneshot::channel();

        tracker.register(submission, tx).expect("register");

        let terminal = tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .expect("terminal")
            .expect("sender");

        assert!(matches!(terminal, RemoteTerminalResult::Fulfilled { .. }));
        assert!(source.calls() >= 2);
    }

    #[tokio::test]
    async fn poll_panic_fails_selected_batch_without_killing_actor() {
        let first = submission(ProofType::Sp1);
        let second = submission(ProofType::Sp1);
        let source = Arc::new(FakeSource::with_panic_once(vec![vec![fulfilled_status(
            second.id,
        )]]));
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();

        tracker.register(first, first_tx).expect("register first");
        let first_terminal = tokio::time::timeout(Duration::from_secs(1), first_rx)
            .await
            .expect("first terminal")
            .expect("first sender");
        tracker
            .register(second, second_tx)
            .expect("register second after panic");
        let second_terminal = tokio::time::timeout(Duration::from_secs(1), second_rx)
            .await
            .expect("second terminal")
            .expect("second sender");

        assert!(matches!(
            first_terminal,
            RemoteTerminalResult::Unrecoverable { .. }
        ));
        assert!(matches!(
            second_terminal,
            RemoteTerminalResult::Fulfilled { .. }
        ));
        assert!(source.calls() >= 2);
    }

    #[tokio::test]
    async fn invalid_status_set_fails_selected_submissions() {
        let first = submission(ProofType::Sp1);
        let second = submission(ProofType::Sp1);
        let source = Arc::new(FakeSource::with_statuses(vec![vec![fulfilled_status(
            first.id,
        )]]));
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();

        tracker.register(first, first_tx).expect("register first");
        tracker
            .register(second, second_tx)
            .expect("register second");

        let first_terminal = tokio::time::timeout(Duration::from_secs(1), first_rx)
            .await
            .expect("first terminal")
            .expect("first sender");
        let second_terminal = tokio::time::timeout(Duration::from_secs(1), second_rx)
            .await
            .expect("second terminal")
            .expect("second sender");

        assert!(matches!(
            first_terminal,
            RemoteTerminalResult::Unrecoverable { .. }
        ));
        assert!(matches!(
            second_terminal,
            RemoteTerminalResult::Unrecoverable { .. }
        ));
    }

    #[tokio::test]
    async fn poll_timeout_fails_selected_batch_without_sticking_proof_type() {
        let first = submission(ProofType::Sp1);
        let second = submission(ProofType::Sp1);
        let source = Arc::new(FakeSource::with_delay_once(
            Duration::from_millis(200),
            vec![vec![fulfilled_status(second.id)]],
        ));
        let tracker = tracker_with_config(
            Arc::clone(&source),
            RemotePollerConfig {
                poll_interval: Duration::from_millis(10),
                poll_timeout: Duration::from_millis(30),
            },
        );
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();

        tracker.register(first, first_tx).expect("register first");
        let first_terminal = tokio::time::timeout(Duration::from_secs(1), first_rx)
            .await
            .expect("first terminal")
            .expect("first sender");
        tracker
            .register(second, second_tx)
            .expect("register second after timeout");
        let second_terminal = tokio::time::timeout(Duration::from_secs(1), second_rx)
            .await
            .expect("second terminal")
            .expect("second sender");

        assert!(matches!(
            first_terminal,
            RemoteTerminalResult::Unrecoverable { .. }
        ));
        assert!(matches!(
            second_terminal,
            RemoteTerminalResult::Fulfilled { .. }
        ));
        assert!(source.calls() >= 2);
    }

    #[tokio::test]
    async fn timeout_is_checked_before_polling() {
        let mut submission = submission(ProofType::Sp1);
        submission.timeout_at = Some(Instant::now() - Duration::from_secs(1));
        let source = Arc::new(FakeSource::default());
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (tx, rx) = oneshot::channel();

        tracker.register(submission, tx).expect("register");

        let terminal = tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .expect("terminal")
            .expect("sender");

        assert!(matches!(terminal, RemoteTerminalResult::TimedOut { .. }));
        assert_eq!(source.calls(), 0);
    }

    #[tokio::test]
    async fn in_flight_poll_blocks_duplicate_poll_for_same_proof_type() {
        let submission = submission(ProofType::Sp1);
        let source = Arc::new(FakeSource::with_delay(Duration::from_millis(80)));
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (tx, _rx) = oneshot::channel();

        tracker.register(submission, tx).expect("register");
        tokio::time::sleep(Duration::from_millis(35)).await;

        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn in_flight_poll_does_not_block_submission_timeout() {
        let mut submission = submission(ProofType::Sp1);
        submission.timeout_at = Some(Instant::now() + Duration::from_millis(30));
        let source = Arc::new(FakeSource::with_delay(Duration::from_millis(300)));
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (tx, rx) = oneshot::channel();

        tracker.register(submission, tx).expect("register");

        let terminal = tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .expect("terminal")
            .expect("sender");

        assert!(matches!(terminal, RemoteTerminalResult::TimedOut { .. }));
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn dropped_receiver_is_pruned() {
        let submission = submission(ProofType::Sp1);
        let source = Arc::new(FakeSource::default());
        let tracker = tracker_with_source(Arc::clone(&source), Duration::from_millis(10));
        let (tx, rx) = oneshot::channel();

        tracker.register(submission, tx).expect("register");
        drop(rx);
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert_eq!(source.calls(), 0);
    }
}
