use super::net;
use crate::config::{Config, NetworkPairConfig};
use raiko2_primitives::ProofType;
use raiko2_prover::redact::sanitize_url_for_log;
use serde::Serialize;
use tracing::info;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StartupSummary {
    listen: String,
    routes: Vec<String>,
    pairs: Vec<String>,
    environment: String,
    namespace: String,
    runtime_store: String,
    queue_workers: usize,
    json_logs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_sgx_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_sgx_sgxgeth_base_url: Option<String>,
}

pub(crate) fn build_startup_summary(config: &Config, json_logs: bool) -> StartupSummary {
    let remote_sgx_base_url = (config.prover.is_enabled(ProofType::Sgx)
        && !config.prover.sgx.base_url.trim().is_empty())
    .then(|| sanitize_url_for_log(&config.prover.sgx.base_url));
    let remote_sgx_sgxgeth_base_url = (config.prover.is_enabled(ProofType::SgxGeth)
        && !config.prover.sgxgeth.base_url.trim().is_empty())
    .then(|| sanitize_url_for_log(&config.prover.sgxgeth.base_url));

    StartupSummary {
        listen: net::bind_addr(config),
        routes: config
            .prover
            .iter_routes()
            .map(|(proof_type, runner)| format!("{proof_type}/{runner}"))
            .collect(),
        pairs: config
            .rpc
            .pairs
            .iter()
            .map(NetworkPairConfig::key)
            .collect(),
        environment: config.runtime.environment.clone(),
        namespace: config.runtime.namespace.clone(),
        runtime_store: match config.runtime.store.backend {
            crate::config::RuntimeStoreBackend::Memory => "memory",
            crate::config::RuntimeStoreBackend::Gcs => "gcs",
        }
        .to_string(),
        queue_workers: config.queue.workers,
        json_logs,
        remote_sgx_base_url,
        remote_sgx_sgxgeth_base_url,
    }
}

pub(crate) fn log_startup_summary(config: &Config, json_logs: bool) {
    log_summary(
        "starting raiko2 host",
        &build_startup_summary(config, json_logs),
    );
}

pub(crate) fn log_startup_readiness_passed(config: &Config, json_logs: bool) {
    log_summary(
        "startup readiness passed",
        &build_startup_summary(config, json_logs),
    );
}

fn log_summary(message: &'static str, summary: &StartupSummary) {
    match (
        summary.remote_sgx_base_url.as_deref(),
        summary.remote_sgx_sgxgeth_base_url.as_deref(),
    ) {
        (Some(base_url), Some(sgxgeth_base_url)) => info!(
            listen = %summary.listen,
            routes = ?summary.routes,
            pairs = ?summary.pairs,
            environment = %summary.environment,
            namespace = %summary.namespace,
            runtime_store = %summary.runtime_store,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            remote_sgx_base_url = %base_url,
            remote_sgx_sgxgeth_base_url = %sgxgeth_base_url,
            "{}",
            message
        ),
        (Some(base_url), None) => info!(
            listen = %summary.listen,
            routes = ?summary.routes,
            pairs = ?summary.pairs,
            environment = %summary.environment,
            namespace = %summary.namespace,
            runtime_store = %summary.runtime_store,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            remote_sgx_base_url = %base_url,
            "{}",
            message
        ),
        (None, Some(sgxgeth_base_url)) => info!(
            listen = %summary.listen,
            routes = ?summary.routes,
            pairs = ?summary.pairs,
            environment = %summary.environment,
            namespace = %summary.namespace,
            runtime_store = %summary.runtime_store,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            remote_sgx_sgxgeth_base_url = %sgxgeth_base_url,
            "{}",
            message
        ),
        (None, None) => info!(
            listen = %summary.listen,
            routes = ?summary.routes,
            pairs = ?summary.pairs,
            environment = %summary.environment,
            namespace = %summary.namespace,
            runtime_store = %summary.runtime_store,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            "{}",
            message
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::build_startup_summary;
    use crate::config::{Config, ServerAclFeature, ServerAclKey};
    use serde_json::Value;

    fn summary_json(config: &Config, json_logs: bool) -> Value {
        serde_json::to_value(build_startup_summary(config, json_logs)).expect("serialize summary")
    }

    fn set_routes(config: &mut Config, routes: &str) {
        config
            .prover
            .apply_routes_override(&routes.parse().expect("parse routes"));
    }

    fn sample_config() -> Config {
        let mut config = Config::default();
        config.server.host = "127.0.0.1".to_string();
        config.server.port = 8088;
        config.server.acl.keys = vec![ServerAclKey {
            id: "ops-admin".to_string(),
            key: "secret-admin-key".to_string(),
            allow: vec![
                ServerAclFeature::AdminBallotRead,
                ServerAclFeature::AdminBallotWrite,
            ],
            rate_limit_per_minute: None,
        }];
        set_routes(
            &mut config,
            "sgxgeth/remote,sgx/remote,native/local,sp1/network,risc0/network",
        );
        config.prover.sgx.base_url.clear();
        config.prover.sgxgeth.base_url.clear();
        config.runtime.environment = "test".to_string();
        config.runtime.namespace = "raiko2-test".to_string();
        config.queue.workers = 9;
        config.rpc.pairs[0].network = "taiko_hoodi".to_string();
        config.rpc.pairs[0].l1_network = "hoodi".to_string();
        config
    }

    #[test]
    fn startup_summary_includes_core_fields() {
        let summary = summary_json(&sample_config(), false);

        assert_eq!(summary["listen"], "127.0.0.1:8088");
        assert_eq!(
            summary["routes"],
            serde_json::json!([
                "risc0/network",
                "sp1/network",
                "native/local",
                "sgx/remote",
                "sgxgeth/remote"
            ])
        );
        assert!(summary.get("route").is_none());
        assert_eq!(summary["pairs"], serde_json::json!(["taiko_hoodi/hoodi"]));
        assert_eq!(summary["environment"], "test");
        assert_eq!(summary["namespace"], "raiko2-test");
        assert_eq!(summary["runtime_store"], "memory");
        assert_eq!(summary["queue_workers"], 9);
        assert_eq!(summary["json_logs"], false);
        assert!(summary.get("remote_sgx_base_url").is_none());
        assert!(summary.get("remote_sgx_sgxgeth_base_url").is_none());
    }

    #[test]
    fn startup_summary_includes_only_remote_sgx_url_when_sgx_is_enabled() {
        let mut config = sample_config();
        set_routes(&mut config, "sgx/remote");
        config.prover.sgx.base_url = "http://example.com:9090".to_string();
        config.prover.sgxgeth.base_url = "http://example.com:8090".to_string();

        let summary = summary_json(&config, true);

        assert_eq!(summary["routes"], serde_json::json!(["sgx/remote"]));
        assert_eq!(summary["remote_sgx_base_url"], "http://example.com:9090");
        assert!(summary.get("remote_sgx_sgxgeth_base_url").is_none());
        assert_eq!(summary["json_logs"], true);
    }

    #[test]
    fn startup_summary_includes_only_sgxgeth_url_when_sgxgeth_is_enabled() {
        let mut config = sample_config();
        set_routes(&mut config, "sgxgeth/remote");
        config.prover.sgx.base_url = "http://example.com:9090".to_string();
        config.prover.sgxgeth.base_url = "http://example.com:8090".to_string();

        let summary = summary_json(&config, false);

        assert_eq!(summary["routes"], serde_json::json!(["sgxgeth/remote"]));
        assert!(summary.get("remote_sgx_base_url").is_none());
        assert_eq!(
            summary["remote_sgx_sgxgeth_base_url"],
            "http://example.com:8090"
        );
    }

    #[test]
    fn startup_summary_does_not_expose_secret_like_fields() {
        let summary = summary_json(&sample_config(), false).to_string();

        assert!(!summary.contains("secret-admin-key"));
        assert!(!summary.contains("signer_key"));
        assert!(!summary.contains("acl"));
    }

    #[test]
    fn startup_summary_sanitizes_remote_urls_before_logging() {
        let mut config = sample_config();
        set_routes(&mut config, "sgx/remote,sgxgeth/remote");
        config.prover.sgx.base_url =
            "https://user:secret@example.com:9090/prove?token=abc#frag".to_string();
        config.prover.sgxgeth.base_url =
            "https://other:credential@example.net:8090/prove?key=value#section".to_string();

        let summary = summary_json(&config, false);

        assert_eq!(
            summary["remote_sgx_base_url"],
            "https://example.com:9090/prove"
        );
        assert_eq!(
            summary["remote_sgx_sgxgeth_base_url"],
            "https://example.net:8090/prove"
        );
        let serialized = summary.to_string();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("credential"));
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("key=value"));
    }
}
