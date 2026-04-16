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
    risc0::shasta::{
        AGGREGATION_ELF as RISC0_SHASTA_AGGREGATION_ELF,
        BOUNDLESS_AGGREGATION_ELF as RISC0_SHASTA_BOUNDLESS_AGGREGATION_ELF,
        PROPOSAL_ELF as RISC0_SHASTA_PROPOSAL_ELF,
    },
    sp1::shasta::{
        AGGREGATION_ELF as SP1_SHASTA_AGGREGATION_ELF, PROPOSAL_ELF as SP1_SHASTA_PROPOSAL_ELF,
    },
};
use risc0_zkvm::compute_image_id;
use serde::Serialize;
use sp1_sdk::{HashableKey, Prover as _, ProverClient};

use crate::Backend;
use crate::util;

const DEFAULT_RPC_URL_HOODI_SHASTA: &str = "http://34.46.244.179:8545";
const DEFAULT_PRIVATE_KEY_ENV: &str = "PRIVATE_KEY";
const TX_TIMEOUT: Duration = Duration::from_secs(180);

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

#[derive(Clone, Debug)]
struct ResolvedProfile {
    profile: RegisterImageProfile,
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

#[derive(Clone, Debug, Serialize)]
struct RegistrationReceipt {
    sequence: usize,
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
    backend: &'static str,
    rpc_url: String,
    output_dir: String,
    sender: Option<String>,
    created_at_unix: u64,
    registrations: Vec<PlannedRegistration>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    receipt_files: Vec<String>,
}

pub(crate) async fn run(root: &Path, args: RegisterImageArgs) -> Result<()> {
    let config = resolve_profile(&args);
    let output_dir = resolve_output_dir(root, args.output_dir.as_deref())?;
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create output dir {}", output_dir.display()))?;

    let plan = build_plan(args.backend, &config)?;
    let summary_path = output_dir.join("summary.json");

    if !args.apply {
        let summary = SummaryFile {
            mode: "dry-run",
            profile: profile_name(config.profile),
            backend: backend_name(args.backend),
            rpc_url: config.rpc_url.clone(),
            output_dir: output_dir.display().to_string(),
            sender: None,
            created_at_unix: unix_timestamp(),
            registrations: materialize_plan(&plan),
            receipt_files: Vec::new(),
        };
        write_json(&summary_path, &summary)?;
        println!(
            "Dry-run plan written to {} ({} registrations)",
            summary_path.display(),
            summary.registrations.len()
        );
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
    let provider = ProviderBuilder::new().wallet(signer).connect_http(
        config
            .rpc_url
            .parse()
            .with_context(|| format!("invalid rpc url {}", config.rpc_url))?,
    );

    let mut receipt_files = Vec::with_capacity(plan.len());
    for (index, call) in plan.iter().enumerate() {
        let receipt = apply_one(&provider, call, index + 1, sender).await?;
        let file_name = format!("{:02}-{}.json", index + 1, call.registration_key);
        let receipt_path = output_dir.join(&file_name);
        write_json(&receipt_path, &receipt)?;
        receipt_files.push(file_name);
    }

    let summary = SummaryFile {
        mode: "apply",
        profile: profile_name(config.profile),
        backend: backend_name(args.backend),
        rpc_url: config.rpc_url,
        output_dir: output_dir.display().to_string(),
        sender: Some(address_hex(sender)),
        created_at_unix: unix_timestamp(),
        registrations: materialize_plan(&plan),
        receipt_files,
    };
    write_json(&summary_path, &summary)?;
    println!(
        "Applied {} registrations. Summary: {}",
        summary.registrations.len(),
        summary_path.display()
    );
    Ok(())
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

fn build_plan(backend: Backend, config: &ResolvedProfile) -> Result<Vec<RegistrationCall>> {
    let mut plan = Vec::new();

    match backend {
        Backend::Risc0 => {
            plan.extend(build_risc0_calls(config)?);
        }
        Backend::Sp1 => {
            plan.extend(build_sp1_calls(config)?);
        }
        Backend::All => {
            plan.extend(build_risc0_calls(config)?);
            plan.extend(build_sp1_calls(config)?);
        }
    }

    if plan.is_empty() {
        bail!("no registrations generated");
    }

    Ok(plan)
}

fn build_risc0_calls(config: &ResolvedProfile) -> Result<Vec<RegistrationCall>> {
    Ok(vec![
        risc0_call(
            "risc0_shasta_proposal",
            Stage::Proposal,
            RISC0_SHASTA_PROPOSAL_ELF,
            config.risc0_verifier,
        )?,
        risc0_call(
            "risc0_shasta_aggregation",
            Stage::Aggregation,
            RISC0_SHASTA_AGGREGATION_ELF,
            config.risc0_verifier,
        )?,
        risc0_call(
            "risc0_shasta_boundless_aggregation",
            Stage::Aggregation,
            RISC0_SHASTA_BOUNDLESS_AGGREGATION_ELF,
            config.risc0_verifier,
        )?,
    ])
}

fn build_sp1_calls(config: &ResolvedProfile) -> Result<Vec<RegistrationCall>> {
    let client = ProverClient::builder().cpu().build();
    let proposal_vk = client.setup(SP1_SHASTA_PROPOSAL_ELF).1;
    let aggregation_vk = client.setup(SP1_SHASTA_AGGREGATION_ELF).1;

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
) -> Result<RegistrationReceipt>
where
    P: Provider + Clone,
{
    let method = call.method_name();
    let (tx_hash, block_number, status, gas_used, readback_trusted) = match call.contract_kind {
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
            let readback = contract
                .isImageTrusted(call.digest)
                .call()
                .await
                .with_context(|| format!("failed to read back {}", call.registration_key))?;
            (
                tx_hash,
                receipt.block_number.unwrap_or_default(),
                receipt.status(),
                receipt.gas_used,
                readback,
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
            let readback = contract
                .isProgramTrusted(call.digest)
                .call()
                .await
                .with_context(|| format!("failed to read back {}", call.registration_key))?;
            (
                tx_hash,
                receipt.block_number.unwrap_or_default(),
                receipt.status(),
                receipt.gas_used,
                readback,
            )
        }
    };

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

fn materialize_plan(plan: &[RegistrationCall]) -> Vec<PlannedRegistration> {
    plan.iter()
        .enumerate()
        .map(|(index, call)| PlannedRegistration {
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
        })
        .collect()
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

fn backend_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Risc0 => "risc0",
        Backend::Sp1 => "sp1",
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
        DigestSource, RegisterImageArgs, RegisterImageProfile, backend_name, build_risc0_calls,
        digest_source_suffix, resolve_profile,
    };
    use crate::Backend;
    use alloy::primitives::{Address, address};
    use std::collections::BTreeSet;

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
        assert_eq!(resolved.rpc_url, "http://127.0.0.1:8545");
        assert_eq!(
            resolved.risc0_verifier,
            Address::from(address!("1111111111111111111111111111111111111111"))
        );
        assert_eq!(
            resolved.sp1_verifier,
            Address::from(address!("2222222222222222222222222222222222222222"))
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
        let calls = build_risc0_calls(&resolved).expect("build risc0 calls");
        let keys = calls
            .iter()
            .map(|call| call.registration_key.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(calls.len(), 3);
        assert!(keys.contains("risc0_shasta_proposal-image-id"));
        assert!(keys.contains("risc0_shasta_aggregation-image-id"));
        assert!(keys.contains("risc0_shasta_boundless_aggregation-image-id"));
    }
}
