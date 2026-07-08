//! Minimal Prometheus telemetry for the hosted `raiko2` API.

use prometheus::{
    Encoder, HistogramVec, IntCounterVec, IntGaugeVec, TextEncoder, histogram_opts,
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
};
use raiko2_primitives::ProofType;
use std::sync::LazyLock;

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
