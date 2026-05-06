use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy::primitives::{Address, B256, address, hex};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use raiko2_guests::{
    DEFAULT_GUEST_ELF_DIR, Risc0ShastaGuestElves, Sp1ShastaGuestElves,
    load_risc0_shasta_guest_elves_from_dir, load_sp1_shasta_guest_elves_from_dir,
};
use risc0_zkvm::compute_image_id;
use serde::{Deserialize, Serialize};
use sp1_sdk::{HashableKey, Prover as _, ProverClient};
use xtask_build_guest::Backend;

use crate::util;

const DEFAULT_RPC_URL_HOODI_SHASTA: &str = "http://34.46.244.179:8545";
const DEFAULT_PRIVATE_KEY_ENV: &str = "PRIVATE_KEY";
const TX_TIMEOUT: Duration = Duration::from_secs(180);
const HOODI_NETWORK: &str = "hoodi";
const HOODI_CHAIN_ID: u64 = 560_048;

sol! {
    #[sol(rpc)]
    contract Risc0ImageVerifierContract {
        function setImageIdTrusted(bytes32 imageId, bool trusted) external;
        function isImageTrusted(bytes32 imageId) external view returns (bool trusted);
    }

    #[sol(rpc)]
    contract Sp1ImageVerifierContract {
        function setProgramTrusted(bytes32 program, bool trusted) external;
        function isProgramTrusted(bytes32 program) external view returns (bool trusted);
    }
}

#[derive(Args)]
pub(crate) struct RegisterImageArgs {
    /// Built-in verifier/rpc profile.
    #[arg(long, value_enum, default_value_t = RegisterImageProfile::HoodiShasta)]
    profile: RegisterImageProfile,
    /// Backend subset to register.
    #[arg(long, value_enum, default_value_t = Backend::All)]
    backend: Backend,
    /// RPC URL override.
    #[arg(long)]
    rpc_url: Option<String>,
    /// RISC0 verifier override.
    #[arg(long)]
    risc0_verifier: Option<Address>,
    /// SP1 verifier override.
    #[arg(long)]
    sp1_verifier: Option<Address>,
    /// Environment variable that stores the broadcast private key.
    #[arg(long, default_value = DEFAULT_PRIVATE_KEY_ENV)]
    private_key_env: String,
    /// Receipt output directory.
    #[arg(long)]
    output_dir: Option<PathBuf>,
    /// Broadcast transactions instead of generating a dry-run summary.
    #[arg(long)]
    apply: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RegisterImageProfile {
    HoodiShasta,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ContractKind {
    Risc0,
    Sp1,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Proposal,
    Aggregation,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum DigestSource {
    ImageId,
    VkBn254,
    VkHashBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PlannedAction {
    Register,
    SkipAlreadyTrusted,
}

#[derive(Clone, Debug)]
struct ResolvedProfile {
    profile: RegisterImageProfile,
    network: &'static str,
    expected_chain_id: u64,
    rpc_url: String,
    risc0_verifier: Address,
    sp1_verifier: Address,
}

#[derive(Clone, Debug, Serialize)]
struct PlannedRegistration {
    sequence: usize,
    registration_key: String,
    object_name: String,
    contract_kind: ContractKind,
    stage: Stage,
    digest_source: DigestSource,
    digest: String,
    contract: String,
    method: &'static str,
    trusted: bool,
    already_trusted: bool,
    needs_registration: bool,
    planned_action: PlannedAction,
}

#[derive(Clone, Debug)]
struct RegistrationCall {
    registration_key: String,
    object_name: String,
    contract_kind: ContractKind,
    stage: Stage,
    digest_source: DigestSource,
    digest: B256,
    contract: Address,
}

#[derive(Clone, Debug)]
struct CheckedRegistration {
    call: RegistrationCall,
    already_trusted: bool,
}

#[derive(Clone, Debug, Serialize)]
struct RegistrationReceipt {
    sequence: usize,
    network: &'static str,
    chain_id: u64,
    registration_key: String,
    object_name: String,
    contract_kind: ContractKind,
    stage: Stage,
    digest_source: DigestSource,
    digest: String,
    contract: String,
    method: &'static str,
    sender: String,
    tx_hash: String,
    block_number: u64,
    status: bool,
    gas_used: u64,
    readback_trusted: bool,
    readback_block_number: u64,
}

#[derive(Debug, Serialize)]
struct SummaryFile {
    mode: &'static str,
    profile: &'static str,
    network: &'static str,
    chain_id: u64,
    backend: &'static str,
    rpc_url: String,
    output_dir: String,
    sender: Option<String>,
    created_at_unix: u64,
    registrations: Vec<PlannedRegistration>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    receipt_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tdx_guest_data: Option<TdxGuestData>,
}

/// TDX bootstrap data — what the on-chain TDX verifier needs to register the prover.
///
/// Read from `~/.config/raiko2/tdx/bootstrap.json`, which is written by the TDX
/// prover the first time it bootstraps (i.e. the first time the server starts
/// with `--features tdx` and a reachable attestation socket).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TdxGuestData {
    issuer_type: String,
    public_key: String,
    quote: String,
    nonce: String,
    metadata: serde_json::Value,
}

pub(crate) async fn run(root: &Path, args: RegisterImageArgs) -> Result<()> {
    let config = resolve_profile(&args);
    let output_dir = resolve_output_dir(root, args.output_dir.as_deref())?;
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create output dir {}", output_dir.display()))?;
    let summary_path = output_dir.join("summary.json");

    if matches!(args.backend, Backend::Tdx) {
        return run_tdx_only(args, &config, &output_dir, &summary_path);
    }

    let tdx_guest_data = if matches!(args.backend, Backend::All) {
        match read_tdx_guest_data() {
            Ok(data) => Some(data),
            Err(err) => {
                println!(
                    "[WARN] Skipping TDX guest data: {err}. Run the server with --features tdx once to bootstrap, then re-run."
                );
                None
            }
        }
    } else {
        None
    };

    let plan = build_plan(args.backend, &config, root)?;
    let read_provider = ProviderBuilder::new().connect_http(
        config
            .rpc_url
            .parse()
            .with_context(|| format!("invalid rpc url {}", config.rpc_url))?,
    );
    let chain_id = read_provider
        .get_chain_id()
        .await
        .context("failed to fetch verifier chain id")?;
    ensure_profile_chain_id(&config, chain_id)?;
    let checked_plan = check_plan(&read_provider, &plan).await?;

    if !args.apply {
        let already_trusted = checked_plan
            .iter()
            .filter(|registration| registration.already_trusted)
            .count();
        let pending = checked_plan.len().saturating_sub(already_trusted);
        let summary = SummaryFile {
            mode: "dry-run",
            profile: profile_name(config.profile),
            network: config.network,
            chain_id,
            backend: backend_name(args.backend),
            rpc_url: config.rpc_url.clone(),
            output_dir: output_dir.display().to_string(),
            sender: None,
            created_at_unix: unix_timestamp(),
            registrations: materialize_checked_plan(&checked_plan),
            receipt_files: Vec::new(),
            tdx_guest_data,
        };
        write_json(&summary_path, &summary)?;
        println!(
            "Dry-run checked {} registrations: {} already trusted, {} need registration. Summary: {}",
            summary.registrations.len(),
            already_trusted,
            pending,
            summary_path.display(),
        );
        if summary.tdx_guest_data.is_some() {
            println!(
                "TDX guest data attached to summary (on-chain TDX registration is not implemented yet)."
            );
        }
        return Ok(());
    }

    let private_key = env::var(&args.private_key_env).with_context(|| {
        format!(
            "{} must be set when using --apply",
            args.private_key_env.trim()
        )
    })?;
    let signer: PrivateKeySigner = private_key.parse().with_context(|| {
        format!(
            "{} does not contain a valid private key",
            args.private_key_env
        )
    })?;
    let sender = signer.address();
    let write_provider = ProviderBuilder::new().wallet(signer).connect_http(
        config
            .rpc_url
            .parse()
            .with_context(|| format!("invalid rpc url {}", config.rpc_url))?,
    );

    let pending_count = checked_plan
        .iter()
        .filter(|registration| !registration.already_trusted)
        .count();
    let mut receipt_files = Vec::with_capacity(pending_count);
    for (index, registration) in checked_plan.iter().enumerate() {
        if registration.already_trusted {
            continue;
        }

        let call = &registration.call;
        let receipt = apply_one(
            &write_provider,
            call,
            index + 1,
            sender,
            config.network,
            chain_id,
        )
        .await?;
        let file_name = format!("{:02}-{}.json", index + 1, call.registration_key);
        let receipt_path = output_dir.join(&file_name);
        write_json(&receipt_path, &receipt)?;
        receipt_files.push(file_name);
    }

    if tdx_guest_data.is_some() {
        println!(
            "[WARN] On-chain TDX prover registration is not implemented yet; TDX guest data is attached to the summary file for manual use."
        );
    }

    let summary = SummaryFile {
        mode: "apply",
        profile: profile_name(config.profile),
        network: config.network,
        chain_id,
        backend: backend_name(args.backend),
        rpc_url: config.rpc_url,
        output_dir: output_dir.display().to_string(),
        sender: Some(address_hex(sender)),
        created_at_unix: unix_timestamp(),
        registrations: materialize_checked_plan(&checked_plan),
        receipt_files,
        tdx_guest_data,
    };
    write_json(&summary_path, &summary)?;
    println!(
        "Applied {} registrations, skipped {} already trusted. Summary: {}",
        summary.receipt_files.len(),
        summary
            .registrations
            .len()
            .saturating_sub(summary.receipt_files.len()),
        summary_path.display()
    );
    Ok(())
}

/// Handle `--backend tdx`: skip RPC/chain interaction entirely. TDX has no
/// per-image registration call yet, so we just dump the bootstrap data into
/// summary.json so it can be used for manual on-chain registration.
fn run_tdx_only(
    args: RegisterImageArgs,
    config: &ResolvedProfile,
    output_dir: &Path,
    summary_path: &Path,
) -> Result<()> {
    if args.apply {
        bail!(
            "--apply is not supported for backend=tdx: on-chain TDX prover registration is not implemented yet. Run with --backend tdx (without --apply) to dump the bootstrap values into summary.json."
        );
    }

    let tdx_guest_data = read_tdx_guest_data().with_context(|| {
        format!(
            "failed to read TDX bootstrap data from {}",
            tdx_bootstrap_path().display()
        )
    })?;

    let summary = SummaryFile {
        mode: "dry-run",
        profile: profile_name(config.profile),
        network: config.network,
        chain_id: 0,
        backend: backend_name(args.backend),
        rpc_url: String::new(),
        output_dir: output_dir.display().to_string(),
        sender: None,
        created_at_unix: unix_timestamp(),
        registrations: Vec::new(),
        receipt_files: Vec::new(),
        tdx_guest_data: Some(tdx_guest_data),
    };
    write_json(summary_path, &summary)?;
    println!(
        "TDX guest data dumped to {} (on-chain TDX registration is not implemented yet).",
        summary_path.display()
    );
    Ok(())
}

fn tdx_bootstrap_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config")
        .join("raiko2")
        .join("tdx")
        .join("bootstrap.json")
}

fn read_tdx_guest_data() -> Result<TdxGuestData> {
    let path = tdx_bootstrap_path();
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("TDX bootstrap file not found at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse TDX bootstrap file {}", path.display()))
}

fn resolve_profile(args: &RegisterImageArgs) -> ResolvedProfile {
    let (rpc_url, risc0_verifier, sp1_verifier) = match args.profile {
        RegisterImageProfile::HoodiShasta => (
            DEFAULT_RPC_URL_HOODI_SHASTA.to_string(),
            address!("fa0e7dAFe9785627df034c123A9B87497EB06b41"),
            address!("c42Ef1A7A606162e144F696A07A7D3Ad98bF4EE7"),
        ),
    };

    ResolvedProfile {
        profile: args.profile,
        network: HOODI_NETWORK,
        expected_chain_id: HOODI_CHAIN_ID,
        rpc_url: args.rpc_url.clone().unwrap_or(rpc_url),
        risc0_verifier: args.risc0_verifier.unwrap_or(risc0_verifier),
        sp1_verifier: args.sp1_verifier.unwrap_or(sp1_verifier),
    }
}

fn resolve_output_dir(root: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        });
    }

    let run_id = unix_timestamp();
    Ok(util::target_root(root)
        .join("register-image")
        .join(format!("run-{run_id}")))
}

fn build_plan(
    backend: Backend,
    config: &ResolvedProfile,
    root: &Path,
) -> Result<Vec<RegistrationCall>> {
    let mut plan = Vec::new();
    let elf_dir = root.join(DEFAULT_GUEST_ELF_DIR);

    match backend {
        Backend::Risc0 => {
            let elves = load_risc0_shasta_guest_elves_from_dir(&elf_dir).with_context(|| {
                format!("failed to load RISC0 guest ELFs from {}", elf_dir.display())
            })?;
            plan.extend(build_risc0_calls(config, &elves)?);
        }
        Backend::Sp1 => {
            let elves = load_sp1_shasta_guest_elves_from_dir(&elf_dir).with_context(|| {
                format!("failed to load SP1 guest ELFs from {}", elf_dir.display())
            })?;
            plan.extend(build_sp1_calls(config, &elves)?);
        }
        Backend::Tdx => {
            // TDX has no on-chain image registration call yet; the bootstrap data
            // is gathered separately via `read_tdx_guest_data` in `run`.
        }
        Backend::All => {
            let risc0_elves =
                load_risc0_shasta_guest_elves_from_dir(&elf_dir).with_context(|| {
                    format!("failed to load RISC0 guest ELFs from {}", elf_dir.display())
                })?;
            let sp1_elves = load_sp1_shasta_guest_elves_from_dir(&elf_dir).with_context(|| {
                format!("failed to load SP1 guest ELFs from {}", elf_dir.display())
            })?;
            plan.extend(build_risc0_calls(config, &risc0_elves)?);
            plan.extend(build_sp1_calls(config, &sp1_elves)?);
        }
    }

    if plan.is_empty() {
        bail!("no registrations generated");
    }

    Ok(plan)
}

fn build_risc0_calls(
    config: &ResolvedProfile,
    elves: &Risc0ShastaGuestElves,
) -> Result<Vec<RegistrationCall>> {
    Ok(vec![
        risc0_call(
            "risc0_shasta_proposal",
            Stage::Proposal,
            elves.proposal.as_ref(),
            config.risc0_verifier,
        )?,
        risc0_call(
            "risc0_shasta_aggregation",
            Stage::Aggregation,
            elves.aggregation.as_ref(),
            config.risc0_verifier,
        )?,
        risc0_call(
            "risc0_shasta_boundless_aggregation",
            Stage::Aggregation,
            elves.boundless_aggregation.as_ref(),
            config.risc0_verifier,
        )?,
    ])
}

fn build_sp1_calls(
    config: &ResolvedProfile,
    elves: &Sp1ShastaGuestElves,
) -> Result<Vec<RegistrationCall>> {
    let client = ProverClient::builder().cpu().build();
    let proposal_vk = client.setup(elves.proposal.as_ref()).1;
    let aggregation_vk = client.setup(elves.aggregation.as_ref()).1;

    Ok(vec![
        sp1_call(
            "sp1_shasta_proposal",
            Stage::Proposal,
            DigestSource::VkBn254,
            &proposal_vk.bytes32(),
            config.sp1_verifier,
        )?,
        sp1_call(
            "sp1_shasta_proposal",
            Stage::Proposal,
            DigestSource::VkHashBytes,
            &hex::encode_prefixed(proposal_vk.hash_bytes()),
            config.sp1_verifier,
        )?,
        sp1_call(
            "sp1_shasta_aggregation",
            Stage::Aggregation,
            DigestSource::VkBn254,
            &aggregation_vk.bytes32(),
            config.sp1_verifier,
        )?,
        sp1_call(
            "sp1_shasta_aggregation",
            Stage::Aggregation,
            DigestSource::VkHashBytes,
            &hex::encode_prefixed(aggregation_vk.hash_bytes()),
            config.sp1_verifier,
        )?,
    ])
}

fn risc0_call(
    object_name: &str,
    stage: Stage,
    elf: &[u8],
    contract: Address,
) -> Result<RegistrationCall> {
    let image_id = compute_image_id(elf)
        .with_context(|| format!("failed to compute RISC0 image id for {object_name}"))?;
    let digest = B256::from_slice(image_id.as_bytes());
    Ok(RegistrationCall {
        registration_key: format!("{object_name}-image-id"),
        object_name: object_name.to_string(),
        contract_kind: ContractKind::Risc0,
        stage,
        digest_source: DigestSource::ImageId,
        digest,
        contract,
    })
}

fn sp1_call(
    object_name: &str,
    stage: Stage,
    digest_source: DigestSource,
    digest: &str,
    contract: Address,
) -> Result<RegistrationCall> {
    Ok(RegistrationCall {
        registration_key: format!("{object_name}-{}", digest_source_suffix(digest_source)),
        object_name: object_name.to_string(),
        contract_kind: ContractKind::Sp1,
        stage,
        digest_source,
        digest: B256::from_str(digest)
            .with_context(|| format!("invalid SP1 digest for {object_name}: {digest}"))?,
        contract,
    })
}

async fn apply_one<P>(
    provider: P,
    call: &RegistrationCall,
    sequence: usize,
    sender: Address,
    network: &'static str,
    chain_id: u64,
) -> Result<RegistrationReceipt>
where
    P: Provider + Clone,
{
    let method = call.method_name();
    let (tx_hash, block_number, status, gas_used) = match call.contract_kind {
        ContractKind::Risc0 => {
            let contract = Risc0ImageVerifierContract::new(call.contract, provider.clone());
            let pending = contract
                .setImageIdTrusted(call.digest, true)
                .send()
                .await
                .with_context(|| format!("failed to send {}", call.registration_key))?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending
                .with_timeout(Some(TX_TIMEOUT))
                .get_receipt()
                .await
                .with_context(|| format!("failed to confirm {}", call.registration_key))?;
            (
                tx_hash,
                receipt.block_number.unwrap_or_default(),
                receipt.status(),
                receipt.gas_used,
            )
        }
        ContractKind::Sp1 => {
            let contract = Sp1ImageVerifierContract::new(call.contract, provider.clone());
            let pending = contract
                .setProgramTrusted(call.digest, true)
                .send()
                .await
                .with_context(|| format!("failed to send {}", call.registration_key))?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending
                .with_timeout(Some(TX_TIMEOUT))
                .get_receipt()
                .await
                .with_context(|| format!("failed to confirm {}", call.registration_key))?;
            (
                tx_hash,
                receipt.block_number.unwrap_or_default(),
                receipt.status(),
                receipt.gas_used,
            )
        }
    };
    let readback_trusted = registration_trusted(provider.clone(), call).await?;

    if !readback_trusted {
        bail!("{} did not read back as trusted", call.registration_key);
    }

    if !status {
        bail!("{} transaction reverted", call.registration_key);
    }

    let readback_block_number = provider
        .get_block_number()
        .await
        .with_context(|| format!("failed to fetch block number for {}", call.registration_key))?;

    Ok(RegistrationReceipt {
        sequence,
        network,
        chain_id,
        registration_key: call.registration_key.clone(),
        object_name: call.object_name.clone(),
        contract_kind: call.contract_kind,
        stage: call.stage,
        digest_source: call.digest_source,
        digest: b256_hex(call.digest),
        contract: address_hex(call.contract),
        method,
        sender: address_hex(sender),
        tx_hash: b256_hex(tx_hash),
        block_number,
        status,
        gas_used,
        readback_trusted,
        readback_block_number,
    })
}

async fn check_plan<P>(provider: P, plan: &[RegistrationCall]) -> Result<Vec<CheckedRegistration>>
where
    P: Provider + Clone,
{
    let mut checked = Vec::with_capacity(plan.len());
    for call in plan {
        let already_trusted = registration_trusted(provider.clone(), call).await?;
        checked.push(CheckedRegistration {
            call: call.clone(),
            already_trusted,
        });
    }
    Ok(checked)
}

async fn registration_trusted<P>(provider: P, call: &RegistrationCall) -> Result<bool>
where
    P: Provider + Clone,
{
    match call.contract_kind {
        ContractKind::Risc0 => {
            let contract = Risc0ImageVerifierContract::new(call.contract, provider);
            contract
                .isImageTrusted(call.digest)
                .call()
                .await
                .with_context(|| format!("failed to read back {}", call.registration_key))
        }
        ContractKind::Sp1 => {
            let contract = Sp1ImageVerifierContract::new(call.contract, provider);
            contract
                .isProgramTrusted(call.digest)
                .call()
                .await
                .with_context(|| format!("failed to read back {}", call.registration_key))
        }
    }
}

fn materialize_checked_plan(checked_plan: &[CheckedRegistration]) -> Vec<PlannedRegistration> {
    checked_plan
        .iter()
        .enumerate()
        .map(|(index, registration)| {
            let call = &registration.call;
            let needs_registration = !registration.already_trusted;
            PlannedRegistration {
                sequence: index + 1,
                registration_key: call.registration_key.clone(),
                object_name: call.object_name.clone(),
                contract_kind: call.contract_kind,
                stage: call.stage,
                digest_source: call.digest_source,
                digest: b256_hex(call.digest),
                contract: address_hex(call.contract),
                method: call.method_name(),
                trusted: true,
                already_trusted: registration.already_trusted,
                needs_registration,
                planned_action: if needs_registration {
                    PlannedAction::Register
                } else {
                    PlannedAction::SkipAlreadyTrusted
                },
            }
        })
        .collect()
}

fn ensure_profile_chain_id(config: &ResolvedProfile, chain_id: u64) -> Result<()> {
    if chain_id != config.expected_chain_id {
        bail!(
            "profile {} expects {} chain id {}, got {}",
            profile_name(config.profile),
            config.network,
            config.expected_chain_id,
            chain_id
        );
    }
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).context("failed to serialize json")?;
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn address_hex(value: Address) -> String {
    format!("{value:#x}")
}

fn b256_hex(value: B256) -> String {
    format!("{value:#x}")
}

fn profile_name(profile: RegisterImageProfile) -> &'static str {
    match profile {
        RegisterImageProfile::HoodiShasta => "hoodi-shasta",
    }
}

const fn backend_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Risc0 => "risc0",
        Backend::Sp1 => "sp1",
        Backend::Tdx => "tdx",
        Backend::All => "all",
    }
}

fn digest_source_suffix(source: DigestSource) -> &'static str {
    match source {
        DigestSource::ImageId => "image-id",
        DigestSource::VkBn254 => "vk-bn254",
        DigestSource::VkHashBytes => "vk-hash-bytes",
    }
}

impl RegistrationCall {
    const fn method_name(&self) -> &'static str {
        match self.contract_kind {
            ContractKind::Risc0 => "setImageIdTrusted(bytes32,bool)",
            ContractKind::Sp1 => "setProgramTrusted(bytes32,bool)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CheckedRegistration, ContractKind, DigestSource, HOODI_CHAIN_ID, HOODI_NETWORK,
        PlannedAction, RegisterImageArgs, RegisterImageProfile, RegistrationCall, Stage,
        backend_name, build_risc0_calls, digest_source_suffix, ensure_profile_chain_id,
        materialize_checked_plan, resolve_profile,
    };
    use alloy::primitives::{Address, B256, address};
    use raiko2_guests::load_risc0_shasta_guest_elves;
    use std::collections::BTreeSet;
    use xtask_build_guest::Backend;

    #[test]
    fn backend_names_match_cli_values() {
        assert_eq!(backend_name(Backend::Risc0), "risc0");
        assert_eq!(backend_name(Backend::Sp1), "sp1");
        assert_eq!(backend_name(Backend::All), "all");
    }

    #[test]
    fn profile_defaults_can_be_overridden() {
        let args = RegisterImageArgs {
            profile: RegisterImageProfile::HoodiShasta,
            backend: Backend::All,
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
            risc0_verifier: Some(address!("1111111111111111111111111111111111111111")),
            sp1_verifier: Some(address!("2222222222222222222222222222222222222222")),
            private_key_env: "PRIVATE_KEY".to_string(),
            output_dir: None,
            apply: false,
        };

        let resolved = resolve_profile(&args);
        assert_eq!(resolved.network, HOODI_NETWORK);
        assert_eq!(resolved.expected_chain_id, HOODI_CHAIN_ID);
        assert_eq!(resolved.rpc_url, "http://127.0.0.1:8545");
        assert_eq!(
            resolved.risc0_verifier,
            address!("1111111111111111111111111111111111111111")
        );
        assert_eq!(
            resolved.sp1_verifier,
            address!("2222222222222222222222222222222222222222")
        );
    }

    #[test]
    fn digest_source_suffixes_are_stable() {
        assert_eq!(digest_source_suffix(DigestSource::ImageId), "image-id");
        assert_eq!(digest_source_suffix(DigestSource::VkBn254), "vk-bn254");
        assert_eq!(
            digest_source_suffix(DigestSource::VkHashBytes),
            "vk-hash-bytes"
        );
    }

    #[test]
    fn risc0_plan_includes_boundless_aggregation_registration() {
        let args = RegisterImageArgs {
            profile: RegisterImageProfile::HoodiShasta,
            backend: Backend::Risc0,
            rpc_url: None,
            risc0_verifier: None,
            sp1_verifier: None,
            private_key_env: "PRIVATE_KEY".to_string(),
            output_dir: None,
            apply: false,
        };

        let resolved = resolve_profile(&args);
        let elves = load_risc0_shasta_guest_elves().expect("load RISC0 Shasta guest ELFs");
        let calls = build_risc0_calls(&resolved, &elves).expect("build risc0 calls");
        let keys = calls
            .iter()
            .map(|call| call.registration_key.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(calls.len(), 3);
        assert!(keys.contains("risc0_shasta_proposal-image-id"));
        assert!(keys.contains("risc0_shasta_aggregation-image-id"));
        assert!(keys.contains("risc0_shasta_boundless_aggregation-image-id"));
    }

    #[test]
    fn hoodi_profile_rejects_wrong_chain_id() {
        let args = RegisterImageArgs {
            profile: RegisterImageProfile::HoodiShasta,
            backend: Backend::All,
            rpc_url: None,
            risc0_verifier: None,
            sp1_verifier: None,
            private_key_env: "PRIVATE_KEY".to_string(),
            output_dir: None,
            apply: false,
        };
        let resolved = resolve_profile(&args);

        ensure_profile_chain_id(&resolved, HOODI_CHAIN_ID).expect("hoodi chain id should match");
        let err = ensure_profile_chain_id(&resolved, HOODI_CHAIN_ID + 1)
            .expect_err("wrong chain id should be rejected");

        assert!(err.to_string().contains("expects hoodi chain id"));
    }

    #[test]
    fn checked_plan_marks_already_trusted_registrations_as_skipped() {
        let checked = vec![
            CheckedRegistration {
                call: sample_call("already_trusted"),
                already_trusted: true,
            },
            CheckedRegistration {
                call: sample_call("needs_registration"),
                already_trusted: false,
            },
        ];

        let registrations = materialize_checked_plan(&checked);

        assert_eq!(
            registrations[0].planned_action,
            PlannedAction::SkipAlreadyTrusted
        );
        assert!(registrations[0].already_trusted);
        assert!(!registrations[0].needs_registration);
        assert_eq!(registrations[1].planned_action, PlannedAction::Register);
        assert!(!registrations[1].already_trusted);
        assert!(registrations[1].needs_registration);
    }

    fn sample_call(name: &str) -> RegistrationCall {
        RegistrationCall {
            registration_key: format!("{name}-image-id"),
            object_name: name.to_string(),
            contract_kind: ContractKind::Risc0,
            stage: Stage::Proposal,
            digest_source: DigestSource::ImageId,
            digest: B256::ZERO,
            contract: Address::ZERO,
        }
    }
}
