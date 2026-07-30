//! Minimal Prometheus telemetry for the hosted `raiko2` API.

use prometheus::{
    Encoder, HistogramVec, IntCounterVec, IntGaugeVec, TextEncoder, histogram_opts,
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
};
use raiko2_pipeline::forks::shasta::preflight_cache::{
    PreflightCacheResult, PreflightCacheStage, PreflightObserver, PreflightSingleFlightEvent,
    PreflightSingleFlightPhase,
};
use raiko2_primitives::ProofType;
use std::{sync::LazyLock, time::Duration};

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
        assert!(!metrics.contains("proposal_id="));
        assert!(!metrics.contains("key_hash="));
        assert!(!metrics.contains("verifier="));
    }
}
