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

/// NVIDIA devices the enabled local GPU provers need. Default when no local GPU proving is enabled.
///
/// In `cuda` builds RISC0 local proving runs in-process on the first device CUDA exposes to raiko2
/// (after `CUDA_VISIBLE_DEVICES`). SP1 `prover = "local"` with `mode = "prove"` runs
/// `sp1-gpu-server` with `CUDA_VISIBLE_DEVICES=<sp1.cuda_device_id>` (default 0), which overrides
/// the parent's setting, so that id indexes the host's devices. Execute-only SP1 stays on the CPU.
#[derive(Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
struct GpuRequirements {
    risc0: bool,
    sp1_device: Option<u32>,
}

#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
fn gpu_requirements(config: &Config) -> GpuRequirements {
    let risc0 = matches!(
        config.prover.runner(ProofType::Risc0),
        Some(RunnerKind::Local)
    ) && !config.prover.risc0.mock;
    let sp1 = &config.prover.sp1;
    let sp1_gpu = config.prover.is_enabled(ProofType::Sp1)
        && sp1.prover == raiko2_prover::sp1_config::ProverMode::Local
        && matches!(sp1.mode, Sp1ExecutionMode::Prove);
    GpuRequirements {
        risc0,
        sp1_device: sp1_gpu.then(|| sp1.cuda_device_id.unwrap_or(0)),
    }
}

/// One `nvidia-smi --query-gpu=index,uuid --format=csv,noheader` row.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
struct NvidiaGpu {
    index: u32,
    uuid: String,
}

#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
fn parse_nvidia_gpus(stdout: &str) -> Result<Vec<NvidiaGpu>> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (index, uuid) = line
                .split_once(',')
                .ok_or_else(|| anyhow!("unexpected nvidia-smi row {line:?}"))?;
            Ok(NvidiaGpu {
                index: index
                    .trim()
                    .parse()
                    .with_context(|| format!("unexpected nvidia-smi index in {line:?}"))?,
                uuid: uuid.trim().to_string(),
            })
        })
        .collect()
}

/// Devices CUDA exposes to this process after `CUDA_VISIBLE_DEVICES`.
///
/// Follows CUDA's rules: unset exposes every device; entries are indices or (prefixes of)
/// `GPU-`/`MIG-` UUIDs; the first invalid or repeated entry hides it and every later entry, so
/// `-1` or an empty value exposes none.
#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
fn cuda_visible_device_count(gpus: &[NvidiaGpu], cuda_visible_devices: Option<&str>) -> usize {
    let Some(visible) = cuda_visible_devices else {
        return gpus.len();
    };
    let mut selected: Vec<u32> = Vec::new();
    for entry in visible.split(',').map(str::trim) {
        let gpu = if entry.starts_with("GPU-") || entry.starts_with("MIG-") {
            gpus.iter().find(|gpu| gpu.uuid.starts_with(entry))
        } else {
            entry
                .parse::<u32>()
                .ok()
                .and_then(|index| gpus.iter().find(|gpu| gpu.index == index))
        };
        match gpu {
            Some(gpu) if !selected.contains(&gpu.index) => selected.push(gpu.index),
            _ => break,
        }
    }
    selected.len()
}

#[cfg_attr(not(feature = "cuda"), allow(dead_code))]
fn check_gpu_requirements(
    requirements: &GpuRequirements,
    gpus: &[NvidiaGpu],
    cuda_visible_devices: Option<&str>,
) -> Result<()> {
    if requirements.risc0 && cuda_visible_device_count(gpus, cuda_visible_devices) == 0 {
        bail!(
            "RISC0 local GPU proving sees no CUDA device ({} on the host, CUDA_VISIBLE_DEVICES={})",
            gpus.len(),
            cuda_visible_devices.unwrap_or("<unset>")
        );
    }
    if let Some(device_id) = requirements.sp1_device
        && !gpus.iter().any(|gpu| gpu.index == device_id)
    {
        bail!(
            "SP1 local GPU proving uses CUDA device {device_id}, but nvidia-smi reports {} device(s)",
            gpus.len()
        );
    }
    Ok(())
}

#[cfg(feature = "cuda")]
async fn check_local_cuda_devices(config: &Config) -> Result<()> {
    const NVIDIA_SMI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    let requirements = gpu_requirements(config);
    if requirements == GpuRequirements::default() {
        return Ok(());
    }
    let output = tokio::time::timeout(
        NVIDIA_SMI_TIMEOUT,
        tokio::process::Command::new("nvidia-smi")
            .args(["--query-gpu=index,uuid", "--format=csv,noheader"])
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
    let gpus = parse_nvidia_gpus(&String::from_utf8_lossy(&output.stdout))?;
    let cuda_visible_devices = std::env::var("CUDA_VISIBLE_DEVICES").ok();
    check_gpu_requirements(&requirements, &gpus, cuda_visible_devices.as_deref())?;
    // An sp1-gpu-server that failed to start or died stays unready until it is replaced.
    if requirements.sp1_device.is_some()
        && let Some(message) = describe_cuda_failures(&raiko2_prover::sp1::cuda_device_failures())
    {
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
        GpuRequirements, NvidiaGpu, ReadyCheck, Sp1RemoteVerifyConfig, check_gpu_requirements,
        check_prover, cuda_visible_device_count, describe_cuda_failures, format_error_chain,
        gpu_requirements, parse_nvidia_gpus, readiness_response, sp1_effective_pair_config,
        sp1_network_credentials_required,
    };
    use crate::config::Config;
    use raiko2_prover::sp1_config::{
        ExecutionMode as Sp1ExecutionMode, ProverMode as Sp1ProverMode,
    };

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

    fn gpus(count: u32) -> Vec<NvidiaGpu> {
        (0..count)
            .map(|index| NvidiaGpu {
                index,
                uuid: format!("GPU-{index}aaaaaaa-0000-0000-0000-000000000000"),
            })
            .collect()
    }

    #[test]
    fn gpus_not_required_without_local_zk_proving() {
        let mut config = Config::default();
        set_routes(&mut config, "sp1/network,sgx/remote");

        assert_eq!(gpu_requirements(&config), GpuRequirements::default());
    }

    #[test]
    fn gpus_not_required_for_mock_risc0_or_execute_only_sp1() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/local,sp1/local");
        config.prover.risc0.mock = true;
        config.prover.sp1.prover = Sp1ProverMode::Local;
        config.prover.sp1.mode = Sp1ExecutionMode::Execute;
        config.prover.sp1.cuda_device_id = Some(1);

        assert_eq!(gpu_requirements(&config), GpuRequirements::default());
    }

    #[test]
    fn gpu_requirements_cover_risc0_and_configured_sp1_device() {
        let mut config = Config::default();
        set_routes(&mut config, "risc0/local,sp1/local");
        config.prover.risc0.mock = false;
        config.prover.sp1.prover = Sp1ProverMode::Local;
        config.prover.sp1.mode = Sp1ExecutionMode::Prove;
        config.prover.sp1.cuda_device_id = Some(1);

        assert_eq!(
            gpu_requirements(&config),
            GpuRequirements {
                risc0: true,
                sp1_device: Some(1),
            }
        );
    }

    #[test]
    fn nvidia_smi_rows_parse_index_and_uuid() {
        let parsed = parse_nvidia_gpus("0, GPU-abc\n1, GPU-def\n\n").expect("valid rows");

        assert_eq!(
            parsed,
            vec![
                NvidiaGpu {
                    index: 0,
                    uuid: "GPU-abc".to_string()
                },
                NvidiaGpu {
                    index: 1,
                    uuid: "GPU-def".to_string()
                },
            ]
        );
        assert!(parse_nvidia_gpus("garbage").is_err());
    }

    #[test]
    fn cuda_visible_devices_follow_cuda_rules() {
        let host = gpus(2);

        assert_eq!(cuda_visible_device_count(&host, None), 2);
        assert_eq!(cuda_visible_device_count(&host, Some("1")), 1);
        assert_eq!(cuda_visible_device_count(&host, Some("1,0")), 2);
        assert_eq!(cuda_visible_device_count(&host, Some("GPU-1aaa")), 1);
        assert_eq!(cuda_visible_device_count(&host, Some("-1")), 0);
        assert_eq!(cuda_visible_device_count(&host, Some("")), 0);
        assert_eq!(cuda_visible_device_count(&host, Some("2,0")), 0);
        assert_eq!(cuda_visible_device_count(&host, Some("0,7,1")), 1);
        assert_eq!(cuda_visible_device_count(&host, Some("0,0")), 1);
    }

    #[test]
    fn gpu_check_rejects_hidden_risc0_and_missing_sp1_device() {
        let both = GpuRequirements {
            risc0: true,
            sp1_device: Some(1),
        };

        assert!(check_gpu_requirements(&both, &gpus(2), None).is_ok());
        let hidden = check_gpu_requirements(&both, &gpus(2), Some("-1"))
            .expect_err("RISC0 needs a visible device")
            .to_string();
        assert!(hidden.contains("RISC0"), "{hidden}");
        let missing = check_gpu_requirements(&both, &gpus(1), None)
            .expect_err("SP1 device 1 needs two devices")
            .to_string();
        assert!(missing.contains("SP1"), "{missing}");
        // SP1's server ignores the parent's CUDA_VISIBLE_DEVICES, so only RISC0 is affected.
        let sp1_only = GpuRequirements {
            risc0: false,
            sp1_device: Some(1),
        };
        assert!(check_gpu_requirements(&sp1_only, &gpus(2), Some("-1")).is_ok());
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
