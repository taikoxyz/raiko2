//! Startup summary logging helpers for the dedicated SGX provider.

use serde::Serialize;
use tracing::info;

use crate::config::{GlobalOpts, RuntimeMode, ServiceConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StartupSummary {
    mode: &'static str,
    listen: String,
    fork: String,
    instance_id: u32,
    config_dir: String,
    secret_dir: String,
}

pub(crate) fn build_startup_summary(
    global_opts: &GlobalOpts,
    service_config: &ServiceConfig,
) -> StartupSummary {
    StartupSummary {
        mode: runtime_mode_name(global_opts.mode),
        listen: service_config.listen_addr.clone(),
        fork: service_config.fork.clone(),
        instance_id: service_config.instance_id,
        config_dir: global_opts.config_dir.display().to_string(),
        secret_dir: global_opts.secret_dir.display().to_string(),
    }
}

pub(crate) fn log_startup_summary(global_opts: &GlobalOpts, service_config: &ServiceConfig) {
    let summary = build_startup_summary(global_opts, service_config);
    info!(
        mode = summary.mode,
        listen = %summary.listen,
        fork = %summary.fork,
        instance_id = summary.instance_id,
        config_dir = %summary.config_dir,
        secret_dir = %summary.secret_dir,
        "starting raiko2 sgx provider"
    );
}

const fn runtime_mode_name(mode: RuntimeMode) -> &'static str {
    match mode {
        RuntimeMode::Tee => "tee",
        RuntimeMode::Native => "native",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::build_startup_summary;
    use crate::config::{GlobalOpts, RuntimeMode, ServiceConfig};

    fn summary_json(global_opts: &GlobalOpts, service_config: &ServiceConfig) -> Value {
        serde_json::to_value(build_startup_summary(global_opts, service_config))
            .expect("serialize startup summary")
    }

    fn sample_global_opts() -> GlobalOpts {
        GlobalOpts {
            mode: RuntimeMode::Tee,
            config_dir: PathBuf::from("/var/lib/raiko2/sgx/config"),
            secret_dir: PathBuf::from("/var/lib/raiko2/sgx/secrets"),
        }
    }

    fn sample_service_config() -> ServiceConfig {
        ServiceConfig {
            listen_addr: "0.0.0.0:8080".to_string(),
            fork: "shasta".to_string(),
            instance_id: 14,
        }
    }

    #[test]
    fn startup_summary_includes_core_runtime_fields() {
        let summary = summary_json(&sample_global_opts(), &sample_service_config());

        assert_eq!(summary["mode"], "tee");
        assert_eq!(summary["listen"], "0.0.0.0:8080");
        assert_eq!(summary["fork"], "shasta");
        assert_eq!(summary["instance_id"], 14);
        assert_eq!(summary["config_dir"], "/var/lib/raiko2/sgx/config");
        assert_eq!(summary["secret_dir"], "/var/lib/raiko2/sgx/secrets");
    }

    #[test]
    fn startup_summary_tracks_native_mode() {
        let mut global_opts = sample_global_opts();
        global_opts.mode = RuntimeMode::Native;

        let summary = summary_json(&global_opts, &sample_service_config());

        assert_eq!(summary["mode"], "native");
    }

    #[test]
    fn startup_summary_does_not_expose_bootstrap_payload_fields() {
        let summary = summary_json(&sample_global_opts(), &sample_service_config()).to_string();

        assert!(!summary.contains("quote"));
        assert!(!summary.contains("public_key"));
        assert!(!summary.contains("new_instance"));
        assert!(!summary.contains("priv.key"));
    }
}
