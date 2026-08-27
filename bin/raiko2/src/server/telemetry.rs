//! Minimal Prometheus telemetry for the hosted `raiko2` API.

use prometheus::{
    Encoder, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, TextEncoder, histogram_opts,
    register_histogram_vec, register_int_counter_vec, register_int_gauge, register_int_gauge_vec,
};
use raiko2_pipeline::proposal::preflight_cache::{
    PreflightCacheRecoveryEvent, PreflightCacheResult, PreflightCacheStage, PreflightObserver,
    PreflightSingleFlightEvent, PreflightSingleFlightPhase,
};
use raiko2_primitives::ProofType;
use raiko2_runtime::{
    RuntimeArtifactDeleteOutcome, RuntimeLifecycleObserver, RuntimeStateStats,
    StartupCleanupReport, StartupCleanupScope,
};
use std::{sync::Arc, sync::LazyLock, time::Duration};

static REQUEST_REGISTRATIONS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_request_registrations_total",
        "Total root proof requests accepted by the API",
        &["route", "proof_type", "pair", "aggregate"]
    )
    .expect("register raiko2_request_registrations_total")
});

static STAGE_TASKS_INFLIGHT: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "raiko2_stage_tasks_inflight",
        "Current number of in-flight stage tasks",
        &["route", "proof_type", "pair", "aggregate", "stage"]
    )
    .expect("register raiko2_stage_tasks_inflight")
});

static STAGE_TASK_STARTED_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_stage_task_started_total",
        "Total stage-task starts observed by the engine",
        &["route", "proof_type", "pair", "aggregate", "stage"]
    )
    .expect("register raiko2_stage_task_started_total")
});

static STAGE_TASK_TERMINAL_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_stage_task_terminal_total",
        "Total terminal stage-task outcomes observed by the engine",
        &[
            "route",
            "proof_type",
            "pair",
            "aggregate",
            "stage",
            "status"
        ]
    )
    .expect("register raiko2_stage_task_terminal_total")
});

static STAGE_TASK_FAILURES_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_stage_task_failures_total",
        "Total failed stage tasks grouped by a bounded error kind",
        &[
            "route",
            "proof_type",
            "pair",
            "aggregate",
            "stage",
            "error_kind"
        ]
    )
    .expect("register raiko2_stage_task_failures_total")
});

static STAGE_TASK_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        histogram_opts!(
            "raiko2_stage_task_duration_seconds",
            "Observed stage-task durations in seconds",
            vec![
                0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1_800.0,
                3_600.0, 7_200.0, 14_400.0,
            ]
        ),
        &[
            "route",
            "proof_type",
            "pair",
            "aggregate",
            "stage",
            "status"
        ]
    )
    .expect("register raiko2_stage_task_duration_seconds")
});

static EXTERNAL_SUBMISSION_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_external_submission_total",
        "Total external prover submissions registered by the engine",
        &[
            "route",
            "proof_type",
            "pair",
            "aggregate",
            "stage",
            "provider"
        ]
    )
    .expect("register raiko2_external_submission_total")
});

static DUPLICATE_REQUESTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_duplicate_requests_total",
        "Total duplicate root proof requests observed by the API",
        &["route", "proof_type", "pair", "aggregate", "runner_status"]
    )
    .expect("register raiko2_duplicate_requests_total")
});

static PREFLIGHT_CACHE_REQUESTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_preflight_cache_requests_total",
        "Total canonical preflight cache outcomes",
        &["pair", "result"]
    )
    .expect("register raiko2_preflight_cache_requests_total")
});

static PREFLIGHT_CACHE_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        histogram_opts!(
            "raiko2_preflight_cache_duration_seconds",
            "Canonical preflight cache stage durations in seconds",
            vec![
                0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 15.0, 30.0, 60.0, 300.0
            ]
        ),
        &["pair", "stage"]
    )
    .expect("register raiko2_preflight_cache_duration_seconds")
});

static PREFLIGHT_CACHE_SERIALIZED_BYTES: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        histogram_opts!(
            "raiko2_preflight_cache_serialized_bytes",
            "Serialized canonical preflight core size in bytes",
            vec![
                1_024.0,
                16_384.0,
                65_536.0,
                262_144.0,
                1_048_576.0,
                4_194_304.0,
                16_777_216.0,
                67_108_864.0,
                268_435_456.0,
                1_073_741_824.0,
            ]
        ),
        &["pair"]
    )
    .expect("register raiko2_preflight_cache_serialized_bytes")
});

static PREFLIGHT_SINGLEFLIGHT_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_preflight_singleflight_total",
        "Total canonical preflight single-flight leaders and waiters",
        &["pair", "phase", "role"]
    )
    .expect("register raiko2_preflight_singleflight_total")
});

static PREFLIGHT_SINGLEFLIGHT_WAITERS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "raiko2_preflight_singleflight_waiters",
        "Current canonical preflight single-flight waiters",
        &["pair", "phase"]
    )
    .expect("register raiko2_preflight_singleflight_waiters")
});

static PREFLIGHT_CACHE_RECOVERY_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_preflight_cache_recovery_total",
        "Canonical preflight invalid-cache recovery outcomes",
        &["pair", "outcome"]
    )
    .expect("register raiko2_preflight_cache_recovery_total")
});

static STARTUP_CLEANUP_OBJECTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_startup_cleanup_objects_total",
        "Startup cleanup objects grouped by bounded scope and outcome",
        &["scope", "outcome"]
    )
    .expect("register raiko2_startup_cleanup_objects_total")
});

static STARTUP_CLEANUP_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        histogram_opts!(
            "raiko2_startup_cleanup_duration_seconds",
            "Startup cleanup duration in seconds",
            vec![0.01, 0.1, 0.5, 1.0, 5.0, 15.0, 30.0, 60.0, 300.0, 900.0]
        ),
        &["scope"]
    )
    .expect("register raiko2_startup_cleanup_duration_seconds")
});

static STARTUP_RECONCILIATION_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_startup_reconciliation_total",
        "Startup invalidated-proof reconciliation attempts",
        &["outcome"]
    )
    .expect("register raiko2_startup_reconciliation_total")
});

static STARTUP_RECONCILIATION_ARTIFACTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_startup_reconciliation_artifacts_total",
        "Invalidated proof artifacts finalized during startup reconciliation",
        &["outcome"]
    )
    .expect("register raiko2_startup_reconciliation_artifacts_total")
});

static STARTUP_RECONCILIATION_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        histogram_opts!(
            "raiko2_startup_reconciliation_duration_seconds",
            "Startup invalidated-proof reconciliation duration in seconds",
            vec![0.01, 0.1, 0.5, 1.0, 5.0, 30.0, 120.0, 300.0]
        ),
        &["outcome"]
    )
    .expect("register raiko2_startup_reconciliation_duration_seconds")
});

static RUNTIME_STATE_SERIALIZED_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "raiko2_runtime_state_serialized_bytes",
        "Current serialized authoritative runtime-state size in bytes"
    )
    .expect("register raiko2_runtime_state_serialized_bytes")
});

static RUNTIME_STATE_RECORDS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "raiko2_runtime_state_records",
        "Current authoritative runtime-state records grouped by bounded kind",
        &["kind"]
    )
    .expect("register raiko2_runtime_state_records")
});

static RUNTIME_RETENTION_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_runtime_retention_total",
        "Runtime retention outcomes grouped by bounded outcome",
        &["outcome"]
    )
    .expect("register raiko2_runtime_retention_total")
});

static RUNTIME_RETENTION_RETRY_QUEUE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "raiko2_runtime_retention_retry_queue",
        "Current process-local runtime retention retry identities by lane",
        &["lane"]
    )
    .expect("register raiko2_runtime_retention_retry_queue")
});

static RUNTIME_RETENTION_BLOCKED: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "raiko2_runtime_retention_blocked",
        "Whether the most recent runtime retention pass for a lane was blocked",
        &["lane"]
    )
    .expect("register raiko2_runtime_retention_blocked")
});

static RUNTIME_RETENTION_ATTEMPTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_runtime_retention_attempts_total",
        "Runtime retention attempts by lane and scheduler source",
        &["lane", "source"]
    )
    .expect("register raiko2_runtime_retention_attempts_total")
});

static RUNTIME_RETENTION_OUTCOMES_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_runtime_retention_outcomes_total",
        "Runtime retention outcomes by lane",
        &["lane", "outcome"]
    )
    .expect("register raiko2_runtime_retention_outcomes_total")
});

static ARTIFACT_LIFECYCLE_LOCK_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        histogram_opts!(
            "raiko2_artifact_lifecycle_lock_duration_seconds",
            "Artifact lifecycle keyed-lock wait and hold durations in seconds",
            vec![0.000_1, 0.001, 0.01, 0.1, 1.0, 5.0, 30.0, 120.0]
        ),
        &["phase"]
    )
    .expect("register raiko2_artifact_lifecycle_lock_duration_seconds")
});

static ARTIFACT_LIFECYCLE_LOCK_REGISTRY: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "raiko2_artifact_lifecycle_lock_registry",
        "Artifact lifecycle keyed-lock registry entries",
        &["state"]
    )
    .expect("register raiko2_artifact_lifecycle_lock_registry")
});

static ARTIFACT_LIFECYCLE_LOCK_SWEPT_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_artifact_lifecycle_lock_swept_total",
        "Dead artifact lifecycle keyed-lock entries swept",
        &[]
    )
    .expect("register raiko2_artifact_lifecycle_lock_swept_total")
});

static PROOF_EXACT_DELETE_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_proof_exact_delete_total",
        "Proof manifest exact-delete outcomes",
        &["outcome"]
    )
    .expect("register raiko2_proof_exact_delete_total")
});

static PROOF_PUBLICATION_CLEANUP_PENDING_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "raiko2_proof_publication_cleanup_pending_total",
        "Proof publications rejected while exact cleanup is pending",
        &[]
    )
    .expect("register raiko2_proof_publication_cleanup_pending_total")
});

static RUNTIME_INVALIDATED_ARTIFACTS: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "raiko2_runtime_invalidated_artifacts",
        "Current proof artifacts in the durable Invalidated lifecycle"
    )
    .expect("register raiko2_runtime_invalidated_artifacts")
});

#[derive(Debug)]
struct RuntimeLifecycleMetricsObserver;

impl RuntimeLifecycleObserver for RuntimeLifecycleMetricsObserver {
    fn record_lock_duration(&self, phase: &'static str, duration: Duration) {
        ARTIFACT_LIFECYCLE_LOCK_DURATION_SECONDS
            .with_label_values(&[phase])
            .observe(duration.as_secs_f64());
    }

    fn record_lock_registry(&self, live: usize, dead: usize, swept: usize) {
        for (state, count) in [("live", live), ("dead", dead)] {
            ARTIFACT_LIFECYCLE_LOCK_REGISTRY
                .with_label_values(&[state])
                .set(i64::try_from(count).unwrap_or(i64::MAX));
        }
        ARTIFACT_LIFECYCLE_LOCK_SWEPT_TOTAL
            .with_label_values(&[])
            .inc_by(u64::try_from(swept).unwrap_or(u64::MAX));
    }

    fn record_exact_delete(&self, outcome: RuntimeArtifactDeleteOutcome) {
        let outcome = match outcome {
            RuntimeArtifactDeleteOutcome::Removed => "removed",
            RuntimeArtifactDeleteOutcome::Missing => "missing",
            RuntimeArtifactDeleteOutcome::Stale => "stale",
            RuntimeArtifactDeleteOutcome::Failure => "failure",
        };
        PROOF_EXACT_DELETE_TOTAL.with_label_values(&[outcome]).inc();
    }

    fn record_cleanup_pending(&self) {
        PROOF_PUBLICATION_CLEANUP_PENDING_TOTAL
            .with_label_values(&[])
            .inc();
    }
}

pub(crate) fn runtime_lifecycle_observer() -> Arc<dyn RuntimeLifecycleObserver> {
    Arc::new(RuntimeLifecycleMetricsObserver)
}

#[derive(Debug)]
pub(crate) struct PreflightCacheMetricsObserver {
    pair: String,
}

impl PreflightCacheMetricsObserver {
    pub(crate) const fn new(pair: String) -> Self {
        Self { pair }
    }
}

impl PreflightObserver for PreflightCacheMetricsObserver {
    fn record_cache_result(&self, result: PreflightCacheResult) {
        PREFLIGHT_CACHE_REQUESTS_TOTAL
            .with_label_values(&[self.pair.as_str(), result.as_str()])
            .inc();
    }

    fn record_stage_duration(&self, stage: PreflightCacheStage, duration: Duration) {
        PREFLIGHT_CACHE_DURATION_SECONDS
            .with_label_values(&[self.pair.as_str(), stage.as_str()])
            .observe(duration.as_secs_f64());
    }

    fn record_recovery(&self, event: PreflightCacheRecoveryEvent) {
        PREFLIGHT_CACHE_RECOVERY_TOTAL
            .with_label_values(&[self.pair.as_str(), event.as_str()])
            .inc();
    }

    fn record_serialized_size(&self, bytes: usize) {
        let bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
        PREFLIGHT_CACHE_SERIALIZED_BYTES
            .with_label_values(&[self.pair.as_str()])
            .observe(f64::from(bytes));
    }

    fn record_single_flight(
        &self,
        phase: PreflightSingleFlightPhase,
        event: PreflightSingleFlightEvent,
    ) {
        let phase = phase.as_str();
        match event {
            PreflightSingleFlightEvent::LeaderStarted => {
                PREFLIGHT_SINGLEFLIGHT_TOTAL
                    .with_label_values(&[self.pair.as_str(), phase, "leader"])
                    .inc();
            }
            PreflightSingleFlightEvent::WaiterStarted => {
                PREFLIGHT_SINGLEFLIGHT_TOTAL
                    .with_label_values(&[self.pair.as_str(), phase, "waiter"])
                    .inc();
                PREFLIGHT_SINGLEFLIGHT_WAITERS
                    .with_label_values(&[self.pair.as_str(), phase])
                    .inc();
            }
            PreflightSingleFlightEvent::WaiterFinished => {
                PREFLIGHT_SINGLEFLIGHT_WAITERS
                    .with_label_values(&[self.pair.as_str(), phase])
                    .dec();
            }
        }
    }
}

pub(crate) fn record_startup_cleanup_report(report: &StartupCleanupReport) {
    for entry in &report.scopes {
        let scope = entry.scope.as_str();
        for (outcome, value) in [
            ("matched", entry.matched),
            ("removed", entry.removed),
            ("failed", entry.failed),
        ] {
            STARTUP_CLEANUP_OBJECTS_TOTAL
                .with_label_values(&[scope, outcome])
                .inc_by(u64::try_from(value).unwrap_or(u64::MAX));
        }
        STARTUP_CLEANUP_DURATION_SECONDS
            .with_label_values(&[scope])
            .observe(entry.duration.as_secs_f64());
    }
}

pub(crate) fn record_startup_cleanup_failure(scope: StartupCleanupScope) {
    STARTUP_CLEANUP_OBJECTS_TOTAL
        .with_label_values(&[scope.as_str(), "failed"])
        .inc();
}

pub(crate) fn record_startup_reconciliation(
    outcome: &'static str,
    reconciled: usize,
    duration: Duration,
) {
    STARTUP_RECONCILIATION_TOTAL
        .with_label_values(&[outcome])
        .inc();
    STARTUP_RECONCILIATION_ARTIFACTS_TOTAL
        .with_label_values(&[outcome])
        .inc_by(u64::try_from(reconciled).unwrap_or(u64::MAX));
    STARTUP_RECONCILIATION_DURATION_SECONDS
        .with_label_values(&[outcome])
        .observe(duration.as_secs_f64());
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeStateMetricValues {
    serialized_bytes: i64,
    records: [(&'static str, i64); 3],
}

fn runtime_state_metric_values(stats: RuntimeStateStats) -> RuntimeStateMetricValues {
    RuntimeStateMetricValues {
        serialized_bytes: i64::try_from(stats.serialized_bytes).unwrap_or(i64::MAX),
        records: [
            ("tasks", i64::try_from(stats.tasks).unwrap_or(i64::MAX)),
            (
                "artifacts",
                i64::try_from(stats.artifacts).unwrap_or(i64::MAX),
            ),
            (
                "pending_publications",
                i64::try_from(stats.pending_publications).unwrap_or(i64::MAX),
            ),
        ],
    }
}

pub(crate) fn record_runtime_state_stats(stats: RuntimeStateStats) {
    let values = runtime_state_metric_values(stats);
    RUNTIME_STATE_SERIALIZED_BYTES.set(values.serialized_bytes);
    for (kind, count) in values.records {
        RUNTIME_STATE_RECORDS.with_label_values(&[kind]).set(count);
    }
    RUNTIME_INVALIDATED_ARTIFACTS
        .set(i64::try_from(stats.invalidated_artifacts).unwrap_or(i64::MAX));
}

pub(crate) fn record_runtime_cleanup_stats(
    stats: &crate::server::task_cleanup::RuntimeCleanupStats,
) {
    for (outcome, count) in [
        ("selected_tasks", stats.expired),
        ("retired_tasks", stats.retired_roots),
        ("skipped_tasks", stats.skipped_roots),
        ("removed_tasks", stats.removed_roots),
        ("retained_task_failures", stats.retained_failures),
        ("invalidated_artifacts", stats.invalidated_artifacts),
        ("removed_artifacts", stats.removed_artifacts),
        (
            "retained_artifact_failures",
            stats.retained_artifact_failures,
        ),
        (
            "removed_pending_publications",
            stats.removed_pending_publications,
        ),
        (
            "retained_pending_publication_failures",
            stats.retained_pending_publication_failures,
        ),
        ("orphaned_tasks_cancelled", stats.orphaned_cancelled),
        ("overdue_active_tasks", stats.overdue_active_warnings),
    ] {
        RUNTIME_RETENTION_TOTAL
            .with_label_values(&[outcome])
            .inc_by(u64::try_from(count).unwrap_or(u64::MAX));
    }
}

pub(crate) fn record_runtime_cleanup_pass(outcome: &'static str) {
    RUNTIME_RETENTION_TOTAL
        .with_label_values(&[match outcome {
            "success" => "cleanup_pass_success",
            _ => "cleanup_pass_failure",
        }])
        .inc();
}

pub(crate) fn record_runtime_retention_blocked(lane: &'static str, blocked: bool) {
    RUNTIME_RETENTION_BLOCKED
        .with_label_values(&[lane])
        .set(i64::from(blocked));
}

pub(crate) fn record_runtime_cleanup_scheduler_lane(
    lane: &'static str,
    retry_queue_len: usize,
    fresh_attempts: usize,
    retry_attempts: usize,
    successes: usize,
    failures: usize,
    stale: usize,
) {
    RUNTIME_RETENTION_RETRY_QUEUE
        .with_label_values(&[lane])
        .set(i64::try_from(retry_queue_len).unwrap_or(i64::MAX));
    for (source, attempts) in [("fresh", fresh_attempts), ("retry", retry_attempts)] {
        RUNTIME_RETENTION_ATTEMPTS_TOTAL
            .with_label_values(&[lane, source])
            .inc_by(u64::try_from(attempts).unwrap_or(u64::MAX));
    }
    for (outcome, count) in [
        ("success", successes),
        ("failure", failures),
        ("stale", stale),
    ] {
        RUNTIME_RETENTION_OUTCOMES_TOTAL
            .with_label_values(&[lane, outcome])
            .inc_by(u64::try_from(count).unwrap_or(u64::MAX));
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MetricContext {
    route: String,
    proof_type: ProofType,
    pair: String,
    aggregate: bool,
}

impl MetricContext {
    pub(crate) const fn new(
        route: String,
        proof_type: ProofType,
        pair: String,
        aggregate: bool,
    ) -> Self {
        Self {
            route,
            proof_type,
            pair,
            aggregate,
        }
    }

    fn proof_type_label(&self) -> String {
        self.proof_type.to_string()
    }

    const fn aggregate_label(&self) -> &'static str {
        if self.aggregate { "true" } else { "false" }
    }
}

pub(crate) fn record_request_registered(context: &MetricContext, aggregate: bool) {
    let proof_type = context.proof_type_label();
    let aggregate_label = if aggregate { "true" } else { "false" };
    REQUEST_REGISTRATIONS_TOTAL
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            aggregate_label,
        ])
        .inc();
}

pub(crate) fn record_stage_task_started(context: &MetricContext, stage: &str) {
    let proof_type = context.proof_type_label();
    STAGE_TASK_STARTED_TOTAL
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            context.aggregate_label(),
            stage,
        ])
        .inc();
    STAGE_TASKS_INFLIGHT
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            context.aggregate_label(),
            stage,
        ])
        .inc();
}

pub(crate) fn record_stage_task_terminal(
    context: &MetricContext,
    stage: &str,
    status: &str,
    decrement_inflight: bool,
) {
    let proof_type = context.proof_type_label();
    STAGE_TASK_TERMINAL_TOTAL
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            context.aggregate_label(),
            stage,
            status,
        ])
        .inc();
    if decrement_inflight {
        STAGE_TASKS_INFLIGHT
            .with_label_values(&[
                context.route.as_str(),
                proof_type.as_str(),
                context.pair.as_str(),
                context.aggregate_label(),
                stage,
            ])
            .dec();
    }
}

pub(crate) fn record_stage_task_failure(context: &MetricContext, stage: &str, error: &str) {
    let proof_type = context.proof_type_label();
    STAGE_TASK_FAILURES_TOTAL
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            context.aggregate_label(),
            stage,
            failure_error_kind(error),
        ])
        .inc();
}

pub(crate) fn record_stage_task_duration(
    context: &MetricContext,
    stage: &str,
    status: &str,
    duration_seconds: f64,
) {
    let proof_type = context.proof_type_label();
    STAGE_TASK_DURATION_SECONDS
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            context.aggregate_label(),
            stage,
            status,
        ])
        .observe(duration_seconds.max(0.0));
}

pub(crate) fn record_duplicate_request(context: &MetricContext, runner_status: &str) {
    let proof_type = context.proof_type_label();
    DUPLICATE_REQUESTS_TOTAL
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            context.aggregate_label(),
            runner_status,
        ])
        .inc();
}

pub(crate) fn record_external_submission(
    context: &MetricContext,
    stage: &str,
    provider: &'static str,
) {
    let proof_type = context.proof_type_label();
    EXTERNAL_SUBMISSION_TOTAL
        .with_label_values(&[
            context.route.as_str(),
            proof_type.as_str(),
            context.pair.as_str(),
            context.aggregate_label(),
            stage,
            provider,
        ])
        .inc();
}

fn failure_error_kind(error: &str) -> &'static str {
    let error = error.to_ascii_lowercase();
    if error.contains("instance id mismatch") {
        "instance_id_mismatch"
    } else if error.contains("verifier mismatch") {
        "verifier_mismatch"
    } else if error.contains("dependency_not_ready") || error.contains("dependency not ready") {
        "dependency_not_ready"
    } else if error.contains("persist proof output")
        || error.contains("publish proof artifact")
        || error.contains("serialize proof output")
        || error.contains("write proof artifact")
        || error.contains("register proof artifact")
    {
        "proof_persistence"
    } else if error.contains("proof artifact")
        || error.contains("missing completed proposal proof")
        || error.contains("missing completed aggregate proof")
    {
        "stale_artifact"
    } else if error.contains("missing trie node")
        || error.contains("witness state error")
        || error.contains("witness")
    {
        "witness_error"
    } else if error.contains("rpc")
        || error.contains("eth_getlogs")
        || error.contains("block not found")
        || error.contains("beacon")
        || error.contains("sidecar")
        || error.contains("transport")
        || error.contains("connection")
        || error.contains("timeout")
    {
        "rpc_error"
    } else if error.contains("rate limit") || error.contains("rate_limited") {
        "rate_limited"
    } else if error.contains("invalid_request") || error.contains("invalid request") {
        "invalid_request"
    } else if error.contains("prover_error") || error.contains("prover error") {
        "remote_prover_error"
    } else if error.contains("panic") {
        "panic"
    } else if error.contains("cancelled") || error.contains("canceled") {
        "cancelled"
    } else {
        "internal"
    }
}

pub(crate) fn render() -> Result<(String, Vec<u8>), prometheus::Error> {
    let encoder = TextEncoder::new();
    let metrics = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metrics, &mut buffer)?;
    Ok((encoder.format_type().to_string(), buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::task_cleanup::RuntimeCleanupStats;
    use raiko2_runtime::RuntimeStateStats;

    #[test]
    fn preflight_metrics_use_only_bounded_dimensions() {
        let observer = PreflightCacheMetricsObserver::new("metrics_test/l1".to_string());
        for result in [
            PreflightCacheResult::Hit,
            PreflightCacheResult::Miss,
            PreflightCacheResult::Bypass,
            PreflightCacheResult::Error,
        ] {
            observer.record_cache_result(result);
        }
        for stage in [
            PreflightCacheStage::Load,
            PreflightCacheStage::Build,
            PreflightCacheStage::Validate,
        ] {
            observer.record_stage_duration(stage, Duration::from_millis(5));
        }
        observer.record_serialized_size(4_096);
        observer.record_single_flight(
            PreflightSingleFlightPhase::Core,
            PreflightSingleFlightEvent::LeaderStarted,
        );
        observer.record_single_flight(
            PreflightSingleFlightPhase::Core,
            PreflightSingleFlightEvent::WaiterStarted,
        );
        observer.record_single_flight(
            PreflightSingleFlightPhase::Core,
            PreflightSingleFlightEvent::WaiterFinished,
        );
        for event in [
            PreflightCacheRecoveryEvent::InvalidEntry,
            PreflightCacheRecoveryEvent::Rebuild,
            PreflightCacheRecoveryEvent::ExactDeleteRemoved,
            PreflightCacheRecoveryEvent::ExactDeleteMissing,
            PreflightCacheRecoveryEvent::ExactDeleteStale,
            PreflightCacheRecoveryEvent::ExactDeleteFailure,
            PreflightCacheRecoveryEvent::UncachedFallback,
        ] {
            observer.record_recovery(event);
        }

        let (_, metrics) = render().expect("render metrics");
        let metrics = String::from_utf8(metrics).expect("metrics are UTF-8");
        for result in ["hit", "miss", "bypass", "error"] {
            assert!(metrics.contains(&format!(
                "raiko2_preflight_cache_requests_total{{pair=\"metrics_test/l1\",result=\"{result}\"}}"
            )));
        }
        for stage in ["load", "build", "validate"] {
            assert!(metrics.contains(&format!(
                "raiko2_preflight_cache_duration_seconds_count{{pair=\"metrics_test/l1\",stage=\"{stage}\"}}"
            )));
        }
        assert!(metrics.contains(
            "raiko2_preflight_singleflight_waiters{pair=\"metrics_test/l1\",phase=\"core\"} 0"
        ));
        for outcome in [
            "invalid_entry",
            "rebuild",
            "exact_delete_removed",
            "exact_delete_missing",
            "exact_delete_stale",
            "exact_delete_failure",
            "uncached_fallback",
        ] {
            assert!(metrics.contains(&format!(
                "raiko2_preflight_cache_recovery_total{{outcome=\"{outcome}\",pair=\"metrics_test/l1\"}}"
            )));
        }
        assert!(!metrics.contains("proposal_id="));
        assert!(!metrics.contains("key_hash="));
        assert!(!metrics.contains("verifier="));
    }

    #[test]
    fn startup_cleanup_metrics_use_scope_and_outcome_only() {
        let report = StartupCleanupReport {
            scopes: vec![raiko2_runtime::StartupCleanupScopeReport {
                scope: StartupCleanupScope::Proof,
                matched: 3,
                removed: 2,
                failed: 1,
                duration: Duration::from_millis(25),
            }],
        };
        record_startup_cleanup_report(&report);
        record_startup_cleanup_failure(StartupCleanupScope::Preflight);

        let (_, metrics) = render().expect("render metrics");
        let metrics = String::from_utf8(metrics).expect("metrics are UTF-8");
        for outcome in ["matched", "removed", "failed"] {
            assert!(metrics.contains(&format!(
                "raiko2_startup_cleanup_objects_total{{outcome=\"{outcome}\",scope=\"proof\"}}"
            )));
        }
        assert!(metrics.contains(
            "raiko2_startup_cleanup_objects_total{outcome=\"failed\",scope=\"preflight\"}"
        ));
        assert!(metrics.contains("raiko2_startup_cleanup_duration_seconds_count{scope=\"proof\"}"));
    }

    #[test]
    fn startup_reconciliation_metrics_use_only_bounded_outcomes() {
        record_startup_reconciliation("success", 2, Duration::from_millis(25));

        let (_, metrics) = render().expect("render metrics");
        let metrics = String::from_utf8(metrics).expect("metrics are UTF-8");
        assert!(metrics.contains("raiko2_startup_reconciliation_total{outcome=\"success\"}"));
        assert!(
            metrics.contains(
                "raiko2_startup_reconciliation_duration_seconds_count{outcome=\"success\"}"
            )
        );
        assert!(
            metrics.contains("raiko2_startup_reconciliation_artifacts_total{outcome=\"success\"}")
        );
        assert!(!metrics.contains("proof_ref="));
    }

    #[test]
    fn runtime_retention_metrics_use_only_bounded_dimensions() {
        let state_stats = RuntimeStateStats {
            serialized_bytes: 12_345,
            tasks: 7,
            artifacts: 5,
            invalidated_artifacts: 2,
            pending_publications: 2,
        };
        assert_eq!(
            runtime_state_metric_values(state_stats),
            RuntimeStateMetricValues {
                serialized_bytes: 12_345,
                records: [("tasks", 7), ("artifacts", 5), ("pending_publications", 2)],
            }
        );
        record_runtime_state_stats(state_stats);
        record_runtime_cleanup_stats(&RuntimeCleanupStats {
            scanned: 4,
            expired: 4,
            retired_roots: 3,
            skipped_roots: 1,
            removed_roots: 2,
            skipped_shared_children: 1,
            retained_failures: 1,
            invalidated_artifacts: 2,
            removed_artifacts: 1,
            retained_artifact_failures: 1,
            removed_pending_publications: 1,
            retained_pending_publication_failures: 1,
            orphaned_cancelled: 0,
            overdue_active_warnings: 1,
        });
        record_runtime_cleanup_pass("success");
        record_runtime_cleanup_pass("failure");

        let (_, metrics) = render().expect("render metrics");
        let metrics = String::from_utf8(metrics).expect("metrics are UTF-8");
        assert!(metrics.contains("raiko2_runtime_state_serialized_bytes "));
        assert!(metrics.contains("raiko2_runtime_invalidated_artifacts 2"));
        for kind in ["tasks", "artifacts", "pending_publications"] {
            assert!(metrics.contains(&format!("raiko2_runtime_state_records{{kind=\"{kind}\"}}")));
        }
        for outcome in [
            "selected_tasks",
            "retired_tasks",
            "removed_tasks",
            "invalidated_artifacts",
            "removed_artifacts",
            "removed_pending_publications",
            "cleanup_pass_success",
            "cleanup_pass_failure",
        ] {
            assert!(metrics.contains(&format!(
                "raiko2_runtime_retention_total{{outcome=\"{outcome}\"}}"
            )));
        }
        assert!(!metrics.contains("task_id="));
        assert!(!metrics.contains("proof_ref="));
    }

    #[test]
    fn runtime_retention_scheduler_metrics_use_only_fixed_lane_labels() {
        record_runtime_cleanup_scheduler_lane("root", 2, 3, 1, 2, 1, 1);

        let (_, metrics) = render().expect("render metrics");
        let metrics = String::from_utf8(metrics).expect("metrics are UTF-8");
        assert!(metrics.contains("raiko2_runtime_retention_retry_queue{lane=\"root\"}"));
        assert!(
            metrics.contains(
                "raiko2_runtime_retention_attempts_total{lane=\"root\",source=\"fresh\"}"
            )
        );
        assert!(
            metrics.contains(
                "raiko2_runtime_retention_attempts_total{lane=\"root\",source=\"retry\"}"
            )
        );
        for outcome in ["success", "failure", "stale"] {
            assert!(metrics.contains(&format!(
                "raiko2_runtime_retention_outcomes_total{{lane=\"root\",outcome=\"{outcome}\"}}"
            )));
        }
        assert!(!metrics.contains("task_id="));
        assert!(!metrics.contains("proof_ref="));
    }

    #[test]
    fn runtime_retention_blocked_gauge_uses_only_a_fixed_lane_label() {
        record_runtime_retention_blocked("orphan", true);

        let (_, metrics) = render().expect("render metrics");
        let metrics = String::from_utf8(metrics).expect("metrics are UTF-8");
        assert!(metrics.contains("raiko2_runtime_retention_blocked{lane=\"orphan\"}"));
        assert!(!metrics.contains("task_id="));
    }

    #[test]
    fn artifact_lifecycle_metrics_use_only_bounded_outcomes() {
        let observer = RuntimeLifecycleMetricsObserver;
        observer.record_lock_duration("wait", Duration::from_millis(1));
        observer.record_lock_duration("hold", Duration::from_millis(2));
        observer.record_lock_registry(3, 2, 2);
        for outcome in [
            RuntimeArtifactDeleteOutcome::Removed,
            RuntimeArtifactDeleteOutcome::Missing,
            RuntimeArtifactDeleteOutcome::Stale,
            RuntimeArtifactDeleteOutcome::Failure,
        ] {
            observer.record_exact_delete(outcome);
        }
        observer.record_cleanup_pending();

        let (_, metrics) = render().expect("render metrics");
        let metrics = String::from_utf8(metrics).expect("metrics are UTF-8");
        for phase in ["wait", "hold"] {
            assert!(metrics.contains(&format!(
                "raiko2_artifact_lifecycle_lock_duration_seconds_count{{phase=\"{phase}\"}}"
            )));
        }
        for state in ["live", "dead"] {
            assert!(metrics.contains(&format!(
                "raiko2_artifact_lifecycle_lock_registry{{state=\"{state}\"}}"
            )));
        }
        for outcome in ["removed", "missing", "stale", "failure"] {
            assert!(metrics.contains(&format!(
                "raiko2_proof_exact_delete_total{{outcome=\"{outcome}\"}}"
            )));
        }
        assert!(metrics.contains("raiko2_artifact_lifecycle_lock_swept_total"));
        assert!(metrics.contains("raiko2_proof_publication_cleanup_pending_total"));
        assert!(!metrics.contains("proof_ref="));
        assert!(!metrics.contains("generation="));
    }
}
