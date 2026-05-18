use super::net;
use crate::config::{Config, QueueBackend};
use serde::Serialize;
use tracing::info;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StartupSummary {
    listen: String,
    route: String,
    pairs: Vec<String>,
    runtime_root: String,
    queue_backend: String,
    queue_workers: usize,
    json_logs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_sgx_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_sgx_sgxgeth_base_url: Option<String>,
}

pub(crate) fn build_startup_summary(config: &Config, json_logs: bool) -> StartupSummary {
    let (remote_sgx_base_url, remote_sgx_sgxgeth_base_url) = if config.prover.is_remote_sgx_route()
    {
        (
            (!config.prover.remote_sgx.base_url.trim().is_empty())
                .then(|| sanitize_url_for_log(&config.prover.remote_sgx.base_url)),
            (!config.prover.remote_sgx.sgxgeth_base_url.trim().is_empty())
                .then(|| sanitize_url_for_log(&config.prover.remote_sgx.sgxgeth_base_url)),
        )
    } else {
        (None, None)
    };

    StartupSummary {
        listen: net::bind_addr(config).to_string(),
        route: config.prover.route().to_string(),
        pairs: config.rpc.pairs.iter().map(|pair| pair.key()).collect(),
        runtime_root: config.runtime.root.display().to_string(),
        queue_backend: queue_backend_name(config.queue.backend).to_string(),
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
            route = %summary.route,
            pairs = ?summary.pairs,
            runtime_root = %summary.runtime_root,
            queue_backend = %summary.queue_backend,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            remote_sgx_base_url = %base_url,
            remote_sgx_sgxgeth_base_url = %sgxgeth_base_url,
            "{}",
            message
        ),
        (Some(base_url), None) => info!(
            listen = %summary.listen,
            route = %summary.route,
            pairs = ?summary.pairs,
            runtime_root = %summary.runtime_root,
            queue_backend = %summary.queue_backend,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            remote_sgx_base_url = %base_url,
            "{}",
            message
        ),
        (None, Some(sgxgeth_base_url)) => info!(
            listen = %summary.listen,
            route = %summary.route,
            pairs = ?summary.pairs,
            runtime_root = %summary.runtime_root,
            queue_backend = %summary.queue_backend,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            remote_sgx_sgxgeth_base_url = %sgxgeth_base_url,
            "{}",
            message
        ),
        (None, None) => info!(
            listen = %summary.listen,
            route = %summary.route,
            pairs = ?summary.pairs,
            runtime_root = %summary.runtime_root,
            queue_backend = %summary.queue_backend,
            queue_workers = summary.queue_workers,
            json_logs = summary.json_logs,
            "{}",
            message
        ),
    }
}

const fn queue_backend_name(backend: QueueBackend) -> &'static str {
    match backend {
        QueueBackend::Memory => "memory",
        QueueBackend::Redis => "redis",
    }
}

fn sanitize_url_for_log(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        if raw.contains('@') || raw.contains('?') || raw.contains('#') {
            return "<redacted-url>".to_string();
        }
        return raw.to_string();
    };

    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let mut sanitized = url.to_string();
    if url.path() == "/" && !raw.ends_with('/') && sanitized.ends_with('/') {
        sanitized.pop();
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::build_startup_summary;
    use crate::config::{Config, GuestSystem, QueueBackend, RunnerKind};
    use serde_json::Value;
    use std::path::PathBuf;

    fn summary_json(config: &Config, json_logs: bool) -> Value {
        serde_json::to_value(build_startup_summary(config, json_logs)).expect("serialize summary")
    }

    fn sample_config() -> Config {
        let mut config = Config::default();
        config.server.host = "127.0.0.1".to_string();
        config.server.port = 8088;
        config.server.admin_api_key = Some("secret-admin-key".to_string());
        config.prover.guest_system = GuestSystem::Native;
        config.prover.runner = RunnerKind::Local;
        config.runtime.root = PathBuf::from("/tmp/raiko2-runtime");
        config.queue.backend = QueueBackend::Memory;
        config.queue.workers = 9;
        config.rpc.pairs[0].network = "taiko_hoodi".to_string();
        config.rpc.pairs[0].l1_network = "hoodi".to_string();
        config
    }

    #[test]
    fn startup_summary_includes_core_fields() {
        let summary = summary_json(&sample_config(), false);

        assert_eq!(summary["listen"], "127.0.0.1:8088");
        assert_eq!(summary["route"], "native/local");
        assert_eq!(summary["pairs"], serde_json::json!(["taiko_hoodi/hoodi"]));
        assert_eq!(summary["runtime_root"], "/tmp/raiko2-runtime");
        assert_eq!(summary["queue_backend"], "memory");
        assert_eq!(summary["queue_workers"], 9);
        assert_eq!(summary["json_logs"], false);
        assert!(summary.get("remote_sgx_base_url").is_none());
        assert!(summary.get("remote_sgx_sgxgeth_base_url").is_none());
    }

    #[test]
    fn startup_summary_includes_remote_sgx_urls_for_remote_sgx_route() {
        let mut config = sample_config();
        config.prover.guest_system = GuestSystem::Sgx;
        config.prover.runner = RunnerKind::Remote;
        config.prover.remote_sgx.base_url = "http://43.153.195.212:9090".to_string();
        config.prover.remote_sgx.sgxgeth_base_url = "http://43.153.195.212:8090".to_string();

        let summary = summary_json(&config, true);

        assert_eq!(summary["route"], "sgx/remote");
        assert_eq!(summary["remote_sgx_base_url"], "http://43.153.195.212:9090");
        assert_eq!(
            summary["remote_sgx_sgxgeth_base_url"],
            "http://43.153.195.212:8090"
        );
        assert_eq!(summary["json_logs"], true);
    }

    #[test]
    fn startup_summary_does_not_expose_secret_like_fields() {
        let summary = summary_json(&sample_config(), false).to_string();

        assert!(!summary.contains("secret-admin-key"));
        assert!(!summary.contains("signer_key"));
        assert!(!summary.contains("admin_api_key"));
    }

    #[test]
    fn startup_summary_sanitizes_remote_urls_before_logging() {
        let mut config = sample_config();
        config.prover.guest_system = GuestSystem::Sgx;
        config.prover.runner = RunnerKind::Remote;
        config.prover.remote_sgx.base_url =
            "https://user:secret@example.com:9090/prove?token=abc#frag".to_string();

        let summary = summary_json(&config, false);

        assert_eq!(
            summary["remote_sgx_base_url"],
            "https://example.com:9090/prove"
        );
    }
}
