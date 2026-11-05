use crate::config::{Config, RunnerKind};
use crate::server::state::AppState;
use alloy::providers::{Provider as AlloyProvider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, anyhow, bail};
use raiko2_primitives::ProofType;
use raiko2_prover::gaiko2::{Gaiko2Config, Gaiko2Prover};
use raiko2_prover::sp1_config::{
    ExecutionMode as Sp1ExecutionMode, Sp1Config, Sp1RemoteVerifyConfig, Sp1SystemConfig,
};
use raiko2_provider::rpc::build_rpc_client;
use serde::Serialize;
use url::Url;

#[derive(Debug, Serialize)]
pub struct ReadyCheck {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ReadyCheck {
    const fn ok() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    fn err(err: &anyhow::Error) -> Self {
        Self {
            ok: false,
            error: Some(format_error_chain(err)),
        }
    }
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut chain = err.chain();
    let Some(root) = chain.next() else {
        return String::new();
    };

    let mut message = root.to_string();
    for source in chain {
        let source = source.to_string();
        if !source.is_empty() {
            message.push_str(": ");
            message.push_str(&source);
        }
    }
    message
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub status: &'static str,
    /// Legacy field name retained for compatibility; this covers configured L1/L2 RPC readiness.
    pub reth: ReadyCheck,
    pub runtime: ReadyCheck,
    pub queue: ReadyCheck,
    pub prover: ReadyCheck,
    /// Responsiveness of the accelerator proving backends: the remote SGX/SGXGETH TEE runtimes and,
    /// in `cuda` builds, the local NVIDIA GPUs used by RISC0/SP1 local proving.
    pub accelerator: ReadyCheck,
    /// Liveness of the proving worker pool.
    pub workers: ReadyCheck,
}

pub async fn evaluate_readiness(state: &AppState) -> ReadyResponse {
    let reth = match check_rpc_pairs(&state.config).await {
        Ok(()) => ReadyCheck::ok(),
        Err(err) => ReadyCheck::err(&err),
    };

    let runtime = match state.runtime.check_readiness().await {
        Ok(()) => ReadyCheck::ok(),
        Err(err) => ReadyCheck::err(&err),
    };

    let queue_max_age = std::time::Duration::from_millis(
        state
            .config
            .queue
            .maintenance_interval_ms
            .saturating_mul(3)
            .max(1_000),
    );
    let queue = if state.pipelines.queue_maintenance_ready(queue_max_age) {
        ReadyCheck::ok()
    } else {
        ReadyCheck::err(&anyhow::anyhow!(
            "queue maintenance has not completed within {}ms",
            queue_max_age.as_millis()
        ))
    };

    let prover = match check_prover(&state.config) {
        Ok(()) => ReadyCheck::ok(),
        Err(err) => ReadyCheck::err(&err),
    };

    let accelerator = match check_accelerator_health(&state.config).await {
        Ok(()) => ReadyCheck::ok(),
        Err(err) => ReadyCheck::err(&err),
    };

    let workers = if state.pipelines.workers_healthy() {
        ReadyCheck::ok()
    } else {
        ReadyCheck::err(&anyhow!("proving workers are not alive"))
    };

    readiness_response(reth, runtime, queue, prover, accelerator, workers)
}

const fn readiness_response(
    reth: ReadyCheck,
    runtime: ReadyCheck,
    queue: ReadyCheck,
    prover: ReadyCheck,
    accelerator: ReadyCheck,
    workers: ReadyCheck,
) -> ReadyResponse {
    let status = if reth.ok && runtime.ok && queue.ok && prover.ok && accelerator.ok && workers.ok {
        "ok"
    } else {
        "error"
    };
    ReadyResponse {
        status,
        reth,
        runtime,
        queue,
        prover,
        accelerator,
        workers,
    }
}

pub async fn ensure_startup_ready(config: &Config) -> Result<()> {
    check_rpc_pairs(config)
        .await
        .context("reth readiness failed")?;
    check_prover(config).context("prover readiness failed")?;
    Ok(())
}

async fn check_rpc_pairs(config: &Config) -> Result<()> {
    let client_config = config.rpc.provider_client_config();
    for pair in config.rpc.resolved_pairs()? {
        check_rpc_chain_id(
            "l1",
            &pair.key,
            &pair.l1_rpc,
            pair.l1_chain_id(),
            &client_config,
        )
        .await?;
        check_rpc_chain_id(
            "l2",
            &pair.key,
            &pair.l2_rpc,
            pair.l2_chain_id(),
            &client_config,
        )
        .await?;
        if pair.l2_witness_rpc != pair.l2_rpc {
            check_rpc_chain_id(
                "l2_witness",
                &pair.key,
                &pair.l2_witness_rpc,
                pair.l2_chain_id(),
                &client_config,
            )
            .await?;
        }
    }
    Ok(())
}

async fn check_rpc_chain_id(
    kind: &str,
    pair_key: &str,
    rpc_url: &str,
    expected_chain_id: u64,
    client_config: &raiko2_provider::RpcClientConfig,
) -> Result<()> {
    let rpc_client = build_rpc_client(rpc_url, client_config)
        .with_context(|| format!("failed to build {kind} RPC client for {pair_key}"))?;
    let provider = ProviderBuilder::new().connect_client(rpc_client);
    let chain_id = provider
        .get_chain_id()
        .await
        .with_context(|| format!("{kind} eth_chainId failed for {pair_key}"))?;
    if chain_id != expected_chain_id {
        bail!(
            "{kind} chain_id mismatch for {pair_key}: expected {expected_chain_id}, got {chain_id}"
        );
    }
    Ok(())
}

fn check_prover(config: &Config) -> Result<()> {
    config
        .prover
        .validate()
        .context("configured proving capabilities are invalid")?;

    if config.prover.is_enabled(ProofType::Sp1) {
        check_sp1_capability(config).context("sp1 capability is invalid")?;
    }

    Ok(())
}

/// Probe the responsiveness of every enabled GPU/accelerator proving backend.
///
/// For each enabled remote TEE prover (SGX/SGXGETH), reached over HTTP via its configured
/// `base_url`, this performs the `/health` probe that the TEE runtime already exposes. In `cuda`
/// builds it also checks that the NVIDIA devices used by local RISC0/SP1 proving are visible.
/// Backends that are not enabled are skipped, so a pure CPU/local configuration reports ready.
async fn check_accelerator_health(config: &Config) -> Result<()> {
    for (proof_type, remote) in [
        (ProofType::Sgx, &config.prover.sgx),
        (ProofType::SgxGeth, &config.prover.sgxgeth),
    ] {
        if !remote.enabled {
            continue;
        }
        let gaiko_config = Gaiko2Config {
            base_url: remote.base_url.clone(),
            timeout_ms: remote.timeout_ms,
        };
        let prover = Gaiko2Prover::new_for_proof_type(&gaiko_config, proof_type)
            .map_err(|err| anyhow!("failed to build {proof_type} health client: {err}"))?;
        prover
            .health_check()
            .await
            .with_context(|| format!("{proof_type} accelerator backend is not responsive"))?;
    }
    #[cfg(feature = "cuda")]
    check_local_cuda_devices(config)
        .await
        .context("local CUDA proving devices are not available")?;
    Ok(())
}

/// Number of CUDA devices local proving needs, or `None` when no local GPU proving is enabled.
///
/// In `cuda` builds RISC0 local proving runs in-process on CUDA device 0 (or the first device in
/// `CUDA_VISIBLE_DEVICES`), and SP1 `prover = "local"` runs `sp1-gpu-server` on
/// `sp1.cuda_device_id` (default 0).
#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
fn required_cuda_devices(config: &Config) -> Option<u32> {
    let risc0_gpu = matches!(
        config.prover.runner(ProofType::Risc0),
        Some(RunnerKind::Local)
    ) && !config.prover.risc0.mock;
    let sp1_gpu = config.prover.is_enabled(ProofType::Sp1)
        && config.prover.sp1.prover == raiko2_prover::sp1_config::ProverMode::Local;
    let sp1_required = sp1_gpu.then(|| config.prover.sp1.cuda_device_id.unwrap_or(0) + 1);
    let risc0_required = risc0_gpu.then_some(1);
    risc0_required.max(sp1_required)
}

/// Counts devices in `nvidia-smi --query-gpu=index --format=csv,noheader` output.
#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
fn parse_cuda_device_count(stdout: &str) -> u32 {
    let count = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

#[cfg(feature = "cuda")]
async fn check_local_cuda_devices(config: &Config) -> Result<()> {
    const NVIDIA_SMI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    let Some(required) = required_cuda_devices(config) else {
        return Ok(());
    };
    let output = tokio::time::timeout(
        NVIDIA_SMI_TIMEOUT,
        tokio::process::Command::new("nvidia-smi")
            .args(["--query-gpu=index", "--format=csv,noheader"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("nvidia-smi timed out")?
    .context("failed to run nvidia-smi; is the container started with `--gpus all`?")?;
    if !output.status.success() {
        bail!(
            "nvidia-smi exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let visible = parse_cuda_device_count(&String::from_utf8_lossy(&output.stdout));
    if visible < required {
        bail!("local GPU proving needs {required} CUDA device(s), nvidia-smi reports {visible}");
    }
    // An sp1-gpu-server that failed to start or died stays unready until it is replaced.
    let failures = raiko2_prover::sp1::cuda_device_failures();
    if let Some(message) = describe_cuda_failures(&failures) {
        bail!(message);
    }
    Ok(())
}

/// Summarizes SP1 CUDA servers that are down, or `None` when every started server is healthy.
#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
fn describe_cuda_failures(failures: &[(u32, String)]) -> Option<String> {
    if failures.is_empty() {
        return None;
    }
    let devices = failures
        .iter()
        .map(|(device_id, failure)| format!("device {device_id}: {failure}"))
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!("sp1-gpu-server unavailable ({devices})"))
}

const fn sp1_network_credentials_required(config: &Config) -> bool {
    matches!(
        config.prover.runner(ProofType::Sp1),
        Some(RunnerKind::Network)
    )
}

fn check_sp1_capability(config: &Config) -> Result<()> {
    check_sp1_prover(config)
}

fn check_sp1_prover(config: &Config) -> Result<()> {
    let sp1 = &config.prover.sp1;

    if sp1_network_credentials_required(config) {
        if matches!(sp1.mode, Sp1ExecutionMode::Prove) && sp1.verify {
            for pair in config.rpc.resolved_pairs()? {
                if sp1_effective_pair_config(sp1, &pair)
                    .remote_verify
                    .is_none()
                {
                    bail!(
                        "sp1 network verification is not enabled for network pair {}",
                        pair.key
                    );
                }
            }
        }
        let private_key = std::env::var("NETWORK_PRIVATE_KEY")
            .context("NETWORK_PRIVATE_KEY must be set for sp1.prover=network")?;
        let _: PrivateKeySigner = private_key
            .parse()
            .context("NETWORK_PRIVATE_KEY is invalid")?;
    }

    if let Some(rpc_url) = sp1.rpc_url.as_deref() {
        Url::parse(rpc_url).context("sp1.rpc_url is not a valid URL")?;
    }

    Ok(())
}

fn sp1_effective_pair_config(
    global: &Sp1Config,
    pair: &crate::config::ResolvedNetworkPair,
) -> Sp1Config {
    match (
        pair.sp1_verifier_rpc_url.as_ref(),
        pair.sp1_verifier_address.as_ref(),
    ) {
        (Some(rpc_url), Some(verifier_address)) => Sp1SystemConfig {
            remote_verify: Some(Sp1RemoteVerifyConfig {
                rpc_url: rpc_url.clone(),
                verifier_address: verifier_address.clone(),
            }),
        }
        .applied_to(global),
        _ => global.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ReadyCheck, Sp1RemoteVerifyConfig, check_prover, describe_cuda_failures,
        format_error_chain, parse_cuda_device_count, readiness_response, required_cuda_devices,
        sp1_effective_pair_config, sp1_network_credentials_required,
    };
    use crate::config::Config;
    use raiko2_prover::sp1_config::ProverMode as Sp1ProverMode;

    fn set_routes(config: &mut Config, routes: &str) {
        config
            .prover
            .apply_routes_override(&routes.parse().expect("valid prover routes"));
    }

    fn failed_check(message: &str) -> ReadyCheck {
        ReadyCheck {
            ok: false,
            error: Some(message.to_string()),
        }
    }

    #[test]
    fn readiness_ok_only_when_every_check_passes() {
        let response = readiness_response(
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
        );
        assert_eq!(response.status, "ok");
        assert!(response.accelerator.ok);
        assert!(response.workers.ok);
    }

    #[test]
    fn readiness_errors_when_accelerator_backend_is_down() {
        let response = readiness_response(
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            failed_check("sgx accelerator backend is not responsive"),
            ReadyCheck::ok(),
        );
        assert_eq!(response.status, "error");
        assert!(!response.accelerator.ok);
        assert!(response.workers.ok);
    }

    #[test]
    fn readiness_errors_when_workers_are_down() {
        let response = readiness_response(
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            ReadyCheck::ok(),
            failed_check("proving workers are not alive"),
        );
        assert_eq!(response.status, "error");
        assert!(response.accelerator.ok);
        assert!(!response.workers.ok);
    }

    #[test]
    fn sp1_effective_pair_config_honors_global_remote_verify() {
        let mut config = Config::default();
        config.prover.sp1.remote_verify = Some(Sp1RemoteVerifyConfig {
            rpc_url: "https://global-verifier.example.com".to_string(),
            verifier_address: "0x0000000000000000000000000000000000000001".to_string(),
        });
        let pair = config
            .rpc
            .resolved_pairs()
            .expect("resolve default pair")
            .remove(0);

        let effective = sp1_effective_pair_config(&config.prover.sp1, &pair);

        assert_eq!(
            effective
                .remote_verify
                .expect("global remote verifier")
                .rpc_url,
            "https://global-verifier.example.com"
        );
    }

    #[test]
    fn sp1_effective_pair_config_uses_pair_remote_verify_when_present() {
        let mut config = Config::default();
        config.prover.sp1.remote_verify = Some(Sp1RemoteVerifyConfig {
            rpc_url: "https://global-verifier.example.com".to_string(),
            verifier_address: "0x0000000000000000000000000000000000000001".to_string(),
        });
        let mut pair = config
            .rpc
            .resolved_pairs()
            .expect("resolve default pair")
            .remove(0);
        pair.sp1_verifier_rpc_url = Some("https://pair-verifier.example.com".to_string());
        pair.sp1_verifier_address = Some("0x0000000000000000000000000000000000000002".to_string());

        let effective = sp1_effective_pair_config(&config.prover.sp1, &pair);

        assert_eq!(
            effective
                .remote_verify
                .expect("pair remote verifier")
                .rpc_url,
            "https://pair-verifier.example.com"
        );
    }

    #[test]
    fn combined_sgx_and_sp1_still_checks_sp1() {
        let mut config = Config::default();
        set_routes(&mut config, "sp1/network,sgx/remote");
        config.prover.sgx.base_url = "http://127.0.0.1:9090".to_string();

        config
            .prover
            .validate()
            .expect("base prover validation must pass");

        let err = check_prover(&config)
            .expect_err("missing SP1 network verification must fail readiness");
        assert!(
            format_error_chain(&err)
                .contains("sp1 network verification is not enabled for network pair")
        );
    }

    #[test]
    fn sp1_local_skips_network_credentials() {
        let mut config = Config::default();
        set_routes(&mut config, "sp1/local");
        config.prover.sp1.prover = Sp1ProverMode::Local;

        assert!(!sp1_network_credentials_required(&config));
        check_prover(&config).expect("sp1/local must not require network credentials");
    }

    #[test]
    fn sp1_network_credentials_follow_sp1_prover_mode() {
        let mut config = Config::default();
        set_routes(&mut config, "native/local");
        config.prover.sp1.prover = Sp1ProverMode::Network;
        assert!(!sp1_network_credentials_required(&config));

        set_routes(&mut config, "sp1/local");
        assert!(!sp1_network_credentials_required(&config));

        set_routes(&mut config, "sp1/network");
        assert!(sp1_network_credentials_required(&config));
    }

    #[test]
    fn cuda_devices_not_required_without_local_zk_proving() {
        let mut config = Config::default();
        set_routes(&mut config, "sp1/network,sgx/remote");

        assert_eq!(required_cuda_devices(&config), None);
    }

    #[test]
    fn cuda_devices_not_required_for_mock_risc0() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/local");
        config.prover.risc0.mock = true;

        assert_eq!(required_cuda_devices(&config), None);
    }

    #[test]
    fn cuda_devices_cover_risc0_and_configured_sp1_device() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/local");
        config.prover.risc0.mock = false;
        assert_eq!(required_cuda_devices(&config), Some(1));

        set_routes(&mut config, "risc0/local,sp1/local");
        config.prover.sp1.prover = Sp1ProverMode::Local;
        config.prover.sp1.cuda_device_id = Some(1);
        assert_eq!(required_cuda_devices(&config), Some(2));
    }

    #[test]
    fn cuda_device_count_ignores_blank_lines() {
        assert_eq!(parse_cuda_device_count("0\n1\n"), 2);
        assert_eq!(parse_cuda_device_count("\n"), 0);
        assert_eq!(parse_cuda_device_count(""), 0);
    }

    #[test]
    fn cuda_failures_are_reported_per_device() {
        assert_eq!(describe_cuda_failures(&[]), None);
        assert_eq!(
            describe_cuda_failures(&[
                (0, "socket closed".to_string()),
                (1, "start failed".to_string()),
            ])
            .as_deref(),
            Some("sp1-gpu-server unavailable (device 0: socket closed; device 1: start failed)")
        );
    }
}
