//! File-only joins, explicit program compatibility, and immutable comparison runs.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use alethia_reth_evm::zk_gas::pricing::{
    BlockPricingInput, FIXED_POINT_SCALE, MptSelectors, PricingBreakdown, ProposalPricingInput,
    RUNTIME_SPEC_SHA256, ResourceSelectors, ZkGasModel,
};
use anyhow::{Context, Result, ensure};
use num_bigint::BigUint;
use num_traits::Zero;
use rustix::fs::OFlags;

use crate::{
    arithmetic::{ARITHMETIC_POLICY, ceil_mul_ratio, direct_metrics, ratio},
    contracts::*,
    identity::{canonical_json, content_id, sha256_hex},
    output::{CommitOutcome, DirectoryCommitLimits, commit_directory_bounded},
};

pub const MAX_COMPARE_FILES: usize = 128;
pub const MAX_COMPARE_FILE_BYTES: u64 = crate::quote::MAX_QUOTE_OUTPUT_BYTES;
pub const MAX_COMPARE_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_COMPARE_ROWS: usize = MAX_OBSERVATION_ROWS;
pub const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
pub const MAX_PROFILES: usize = 128;
pub const MAX_COMPARE_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
pub const EMBEDDED_PROFILES: &str = include_str!("../profiles/sp1-pricing-calibration-v1.toml");
const EMBEDDED_NORMALIZATION: &[u8] =
    include_bytes!("../profiles/artifacts/unzen-sp1-block-comparison-v1.json");

/// Hard ceilings apply to all callers, including library users. No CLI option can raise them.
#[derive(Clone, Debug, Default, clap::Parser)]
#[command(
    about = "Compare zkGas quotes with external workload observations without fetching or guest execution"
)]
#[command(
    after_help = "Hard ceilings: 128 input/config/artifact files; 64 MiB per quote/observation file; 256 MiB aggregate input; 10,000 total quote/observation rows; 1 MiB per profile/artifact; 128 profiles; 256 MiB serialized output. Files are preflighted before their contents are loaded."
)]
pub struct CompareArgs {
    #[arg(long, required = true)]
    pub quotes: Vec<PathBuf>,
    #[arg(long, required = true)]
    pub observations: Vec<PathBuf>,
    /// Replace the embedded compatibility profiles with this strict TOML config.
    #[arg(long)]
    pub profiles: Option<PathBuf>,
    /// Immutable store root; each run is committed beneath runs/<run-id>.
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Debug)]
pub struct CompareRun {
    pub manifest: ComparisonManifestV1,
    pub run_path: PathBuf,
    pub commit: CommitOutcome,
}
impl CompareRun {
    pub fn exit_code(&self) -> i32 {
        match self.manifest.status {
            RunStatus::Complete => 0,
            RunStatus::Partial => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileResolution {
    Matched(String),
    NoMatch,
    Ambiguous(Vec<String>),
}

fn checked_sum(values: impl IntoIterator<Item = i128>) -> Result<i128> {
    values
        .into_iter()
        .try_fold(0i128, |sum, value| sum.checked_add(value))
        .context("pricing scaled sum overflow")
}

fn ceil_scaled(value: i128) -> Result<u128> {
    ensure!(
        value >= 0 && FIXED_POINT_SCALE > 0,
        "invalid scaled prediction"
    );
    let quotient = value / FIXED_POINT_SCALE;
    let remainder = value % FIXED_POINT_SCALE;
    u128::try_from(quotient + i128::from(remainder != 0)).context("prediction exceeds u128")
}

fn validate_pricing_breakdown(
    pricing: &raiko2_zkgas_trace::PricingBreakdownSnapshot,
) -> Result<()> {
    ensure!(
        [
            pricing.fixed_scaled,
            pricing.operations_scaled,
            pricing.feature_error_scaled,
            pricing.resources_scaled,
            pricing.mpt_scaled,
            pricing.startup_scaled,
            pricing.transaction_payload_scaled,
            pricing.blob_source_scaled,
            pricing.parent_mpt_scaled,
            pricing.total_scaled,
        ]
        .into_iter()
        .all(|value| value >= 0),
        "negative pricing component"
    );
    ensure!(
        pricing.feature_error_scaled >= 0
            && pricing.feature_error_scaled <= pricing.operations_scaled,
        "feature_error attribution exceeds operations"
    );
    ensure!(
        pricing.total_scaled
            == checked_sum([
                pricing.fixed_scaled,
                pricing.operations_scaled,
                pricing.resources_scaled,
                pricing.mpt_scaled,
                pricing.startup_scaled,
                pricing.transaction_payload_scaled,
                pricing.blob_source_scaled,
                pricing.parent_mpt_scaled,
            ])?,
        "pricing breakdown total_scaled mismatch"
    );
    ensure!(
        pricing.total_pgu == ceil_scaled(pricing.total_scaled)?,
        "pricing breakdown total_pgu mismatch"
    );
    Ok(())
}

fn resource_selectors(
    selectors: raiko2_zkgas_trace::ResourceSelectorsSnapshot,
) -> ResourceSelectors {
    ResourceSelectors {
        unique_account_count: selectors.unique_account_count,
        unique_storage_slot_access_count: selectors.unique_storage_slot_access_count,
        unique_storage_slot_write_count: selectors.unique_storage_slot_write_count,
        unique_code_hash_count: selectors.unique_code_hash_count,
        unique_code_bytes: selectors.unique_code_bytes,
    }
}

fn mpt_selectors(selectors: raiko2_zkgas_trace::MptSelectorsSnapshot) -> MptSelectors {
    MptSelectors {
        path_node_visits: selectors.path_node_visits,
        materialization_work_units: selectors.materialization_work_units,
        hydration_hash_blocks: selectors.hydration_hash_blocks,
        dirty_encoded_nodes: selectors.dirty_encoded_nodes,
        dirty_hash_blocks: selectors.dirty_hash_blocks,
        structural_branch_extension_churn_nodes: selectors.structural_branch_extension_churn_nodes,
        dropped_trie_nodes: selectors.dropped_trie_nodes,
    }
}

fn exact_resources_cover_local(exact: ResourceSelectors, local: ResourceSelectors) -> bool {
    exact.unique_account_count >= local.unique_account_count
        && exact.unique_storage_slot_access_count >= local.unique_storage_slot_access_count
        && exact.unique_storage_slot_write_count >= local.unique_storage_slot_write_count
        && exact.unique_code_hash_count >= local.unique_code_hash_count
        && exact.unique_code_bytes >= local.unique_code_bytes
}

fn snapshot_matches_pricing(
    snapshot: &raiko2_zkgas_trace::PricingBreakdownSnapshot,
    pricing: &PricingBreakdown,
) -> bool {
    snapshot.fixed_scaled == pricing.fixed_scaled
        && snapshot.operations_scaled == pricing.operations_scaled
        && snapshot.feature_error_scaled == pricing.feature_error_scaled
        && snapshot.resources_scaled == pricing.resources_scaled
        && snapshot.mpt_scaled == pricing.mpt_scaled
        && snapshot.startup_scaled == pricing.startup_scaled
        && snapshot.transaction_payload_scaled == pricing.transaction_payload_scaled
        && snapshot.blob_source_scaled == pricing.blob_source_scaled
        && snapshot.parent_mpt_scaled == pricing.parent_mpt_scaled
        && snapshot.total_scaled == pricing.total_scaled
        && Some(snapshot.total_pgu) == pricing.total_pgu
}

fn validate_blob_selector_domain(
    selectors: &raiko2_zkgas_trace::ProposalBlobSelectors,
) -> Result<()> {
    ensure!(
        (1..=11).contains(&selectors.source_blob_counts.len()),
        "source blob count is outside the proposal domain"
    );
    let (normal, forced) = selectors
        .source_blob_counts
        .split_last()
        .context("source blob counts are empty")?;
    ensure!(
        forced.iter().all(|count| *count == 1) && (1..=6).contains(normal),
        "source blob topology is outside the proposal domain"
    );
    let total = selectors
        .source_blob_counts
        .iter()
        .try_fold(0u64, |sum, count| sum.checked_add(*count))
        .context("source blob total overflow")?;
    ensure!(
        (1..=16).contains(&total)
            && [
                selectors.proposal_blob_hash_count,
                selectors.provider_blob_count,
                selectors.provider_commitment_count,
                selectors.provider_proof_count,
            ]
            .into_iter()
            .all(|count| count == total),
        "provider blob topology mismatch"
    );
    Ok(())
}

fn validate_complete_quote(complete: &CompleteQuoteV1) -> Result<()> {
    let quote = &complete.quote;
    let proposal = &complete.proposal;
    let current_model = quote.model_id == RUNTIME_SPEC_SHA256;
    ensure!(
        quote.schema_version == 1,
        "embedded quote schema_version mismatch"
    );
    ensure!(
        complete.key.guest_input_sha256 == quote.guest_input_sha256,
        "guest input hash mismatch"
    );
    ensure!(
        usize::try_from(complete.guest_input_byte_length).ok()
            == Some(quote.guest_input_bincode_length),
        "guest input length mismatch"
    );
    ensure!(
        proposal.proof_carry_chain_id == proposal.chain.chain_id,
        "proof-carry chain identity mismatch"
    );
    ensure!(
        proposal.proof_carry_proposal_id == proposal.proposal_id,
        "proof-carry proposal identity mismatch"
    );
    ensure!(
        proposal.block_count > 0,
        "proposal block count must be positive"
    );
    let block_count = usize::try_from(proposal.block_count).context("block count exceeds usize")?;
    ensure!(
        quote.blocks.len() == block_count,
        "proposal and embedded quote block count mismatch"
    );
    ensure!(
        proposal.witness_chain_spec_sha256.len() == block_count,
        "witness chain-spec count mismatch"
    );
    ensure!(
        proposal.first_block.checked_add(proposal.block_count - 1) == Some(proposal.last_block),
        "proposal block range mismatch"
    );
    ensure!(
        proposal.public_output == quote.public_output.to_string(),
        "public output mismatch"
    );
    for (index, block) in quote.blocks.iter().enumerate() {
        ensure!(block.block_index == index, "embedded block index mismatch");
        ensure!(
            proposal.first_block.checked_add(index as u64) == Some(block.block_number),
            "embedded block number mismatch"
        );
        ensure!(
            block.prove.input_transaction_count > 0
                && block.prove.consumed_candidate_count > 0
                && block.prove.attempted_transaction_count <= block.prove.consumed_candidate_count
                && block.prove.consumed_candidate_count <= block.prove.input_transaction_count
                && block.prove.true_unattempted_transactions
                    == block.prove.input_transaction_count - block.prove.consumed_candidate_count,
            "embedded block transaction topology mismatch"
        );
        let node = &block.node.pricing;
        ensure!(
            [
                node.fixed_scaled,
                node.operations_scaled,
                node.feature_error_scaled,
                node.evm_local_resources_scaled,
                node.transaction_payload_scaled,
                node.known_subtotal_scaled,
            ]
            .into_iter()
            .all(|value| value >= 0),
            "negative node pricing component"
        );
        ensure!(
            node.feature_error_scaled >= 0 && node.feature_error_scaled <= node.operations_scaled,
            "feature_error attribution exceeds operations"
        );
        ensure!(
            node.known_subtotal_scaled
                == checked_sum([
                    node.fixed_scaled,
                    node.operations_scaled,
                    node.evm_local_resources_scaled,
                    node.transaction_payload_scaled,
                ])?,
            "node pricing subtotal mismatch"
        );
        ensure!(
            node.known_subtotal_pgu == ceil_scaled(node.known_subtotal_scaled)?,
            "node pricing pgu mismatch"
        );
        let prove = &block.prove.pricing;
        validate_pricing_breakdown(prove)?;
        let committed = block.prove.committed_payload;
        committed
            .residual_after_committed(Default::default())
            .context("invalid committed transaction payload domain")?;
        ensure!(
            committed.total_transaction_count > 0
                && committed.total_transaction_count <= block.prove.attempted_transaction_count
                && node.transaction_payload.canonical_transaction_bytes
                    == committed.canonical_transaction_bytes
                && node.transaction_payload.nonempty_payload_transactions
                    == committed.nonempty_payload_transactions,
            "committed transaction payload does not match node selectors"
        );
        ensure!(
            prove.transaction_payload_scaled == node.transaction_payload_scaled,
            "prove block payload price does not match committed node payload"
        );
        ensure!(
            prove.fixed_scaled >= node.fixed_scaled
                && prove.operations_scaled >= node.operations_scaled
                && prove.feature_error_scaled >= node.feature_error_scaled,
            "attempted prove pricing does not cover committed node pricing"
        );
        ensure!(
            exact_resources_cover_local(
                resource_selectors(block.prove.exact_resources),
                resource_selectors(node.evm_local_resources),
            ),
            "exact resources do not cover EVM-local resources"
        );
        if current_model {
            let local_pricing = ZkGasModel
                .price_block(&BlockPricingInput {
                    started_non_anchor_transactions: 0,
                    native_value_transfers: 0,
                    operation_routes: &[],
                    resources: resource_selectors(node.evm_local_resources),
                    transaction_payload: Default::default(),
                    mpt: MptSelectors::default(),
                })
                .context("EVM-local selector domain is invalid")?;
            ensure!(
                node.evm_local_resources_scaled == local_pricing.resources_scaled,
                "EVM-local resource price mismatch"
            );
            let exact_pricing = ZkGasModel
                .price_block(&BlockPricingInput {
                    started_non_anchor_transactions: 0,
                    native_value_transfers: 0,
                    operation_routes: &[],
                    resources: resource_selectors(block.prove.exact_resources),
                    transaction_payload: Default::default(),
                    mpt: mpt_selectors(block.prove.mpt),
                })
                .context("exact block selector domain is invalid")?;
            ensure!(
                prove.resources_scaled == exact_pricing.resources_scaled
                    && prove.mpt_scaled == exact_pricing.mpt_scaled,
                "exact block resource or MPT price mismatch"
            );
            let payload_pricing = ZkGasModel
                .price_transaction_payload(node.transaction_payload.into())
                .context("committed payload selector domain is invalid")?;
            ensure!(
                node.transaction_payload_scaled == payload_pricing.transaction_payload_scaled,
                "committed payload price mismatch"
            );
        }
        ensure!(
            prove.startup_scaled == 0
                && prove.blob_source_scaled == 0
                && prove.parent_mpt_scaled == 0,
            "block pricing contains proposal-only components"
        );
    }
    ensure!(!proposal.fork_segments.is_empty(), "fork envelope is empty");
    let mut next = proposal.first_block;
    for segment in &proposal.fork_segments {
        ensure!(!segment.fork.is_empty(), "fork name is empty");
        ensure!(
            segment.first_block == next && segment.first_block <= segment.last_block,
            "fork envelope is not ordered and contiguous"
        );
        next = segment
            .last_block
            .checked_add(1)
            .context("fork envelope end overflow")?;
    }
    ensure!(
        next == proposal
            .last_block
            .checked_add(1)
            .context("proposal range end overflow")?,
        "fork envelope does not cover proposal range"
    );

    ensure!(
        complete.prediction_surface == "prove" && complete.prediction_unit == "sp1_pgu",
        "unsupported prediction surface or unit"
    );
    ensure!(
        !quote.model_id.is_empty() && complete.calibration_profile == quote.model_id,
        "prediction model identity mismatch"
    );
    ensure!(
        complete.execution_profile == quote.execution_profile,
        "quote execution profile mismatch"
    );
    let unzen_sum = quote
        .blocks
        .iter()
        .try_fold(0u128, |sum, block| {
            sum.checked_add(u128::from(block.node.unzen_zkgas))
        })
        .context("unzen sum overflow")?;
    ensure!(
        quote.node.unzen_zkgas_sum == unzen_sum,
        "unzen proposal sum mismatch"
    );
    let known_scaled = checked_sum(
        quote
            .blocks
            .iter()
            .map(|b| b.node.pricing.known_subtotal_scaled),
    )?;
    ensure!(
        quote.node.block_local_known_scaled == known_scaled,
        "node scaled sum mismatch"
    );
    ensure!(
        quote.node.block_local_known_pgu == ceil_scaled(known_scaled)?,
        "node proposal pgu mismatch"
    );
    let known_pgu = quote
        .blocks
        .iter()
        .try_fold(0u128, |sum, block| {
            sum.checked_add(block.node.pricing.known_subtotal_pgu)
        })
        .context("node pgu sum overflow")?;
    ensure!(
        quote.node.sum_of_block_known_pgu == known_pgu,
        "node block pgu sum mismatch"
    );

    validate_pricing_breakdown(&quote.prove.proposal_once)?;
    let block_scaled = checked_sum(quote.blocks.iter().map(|b| b.prove.pricing.total_scaled))?;
    ensure!(
        quote.prove.block_local_scaled == block_scaled,
        "prove block scaled sum mismatch"
    );
    ensure!(
        quote.prove.block_local_pgu == ceil_scaled(block_scaled)?,
        "prove block pgu mismatch"
    );
    let block_pgu = quote
        .blocks
        .iter()
        .try_fold(0u128, |sum, block| {
            sum.checked_add(block.prove.pricing.total_pgu)
        })
        .context("prove block pgu sum overflow")?;
    ensure!(
        quote.prove.sum_of_block_local_pgu == block_pgu,
        "prove block-local pgu sum mismatch"
    );
    ensure!(
        quote.prove.proposal_once_pgu == quote.prove.proposal_once.total_pgu,
        "proposal-once pgu mismatch"
    );
    let total_scaled = block_scaled
        .checked_add(quote.prove.proposal_once.total_scaled)
        .context("prove total overflow")?;
    ensure!(
        quote.prove.total_scaled == total_scaled,
        "prove total_scaled mismatch"
    );
    ensure!(
        quote.prove.total_pgu == ceil_scaled(total_scaled)?,
        "prove total_pgu mismatch"
    );
    let input_transactions = quote
        .blocks
        .iter()
        .try_fold(0u64, |sum, block| {
            sum.checked_add(block.prove.input_transaction_count)
        })
        .context("transaction count overflow")?;
    let unattempted = quote
        .blocks
        .iter()
        .try_fold(0u64, |sum, block| {
            sum.checked_add(block.prove.true_unattempted_transactions)
        })
        .context("unattempted transaction count overflow")?;
    ensure!(
        quote.prove.selectors.payload.total_transaction_count == input_transactions,
        "payload transaction count mismatch"
    );
    ensure!(
        quote.prove.selectors.payload.canonical_transaction_bytes >= input_transactions,
        "canonical transaction bytes are below the transaction count"
    );
    ensure!(
        quote.prove.selectors.true_unattempted_transactions == unattempted,
        "unattempted transaction count mismatch"
    );
    ensure!(
        quote.prove.selectors.payload.nonempty_payload_transactions <= input_transactions,
        "nonempty payload transaction count mismatch"
    );
    let selectors = &quote.prove.selectors;
    let committed_payload = quote.blocks.iter().try_fold(
        raiko2_zkgas_trace::ProposalPayloadSelectors::default(),
        |mut total, block| -> Result<_> {
            let payload = block.prove.committed_payload;
            total.total_transaction_count = total
                .total_transaction_count
                .checked_add(payload.total_transaction_count)
                .context("committed transaction count overflow")?;
            total.canonical_transaction_bytes = total
                .canonical_transaction_bytes
                .checked_add(payload.canonical_transaction_bytes)
                .context("committed canonical bytes overflow")?;
            total.nonempty_payload_transactions = total
                .nonempty_payload_transactions
                .checked_add(payload.nonempty_payload_transactions)
                .context("committed nonempty transaction count overflow")?;
            Ok(total)
        },
    )?;
    let residual_payload = selectors
        .payload
        .residual_after_committed(committed_payload)
        .context("invalid residual transaction payload")?;
    validate_blob_selector_domain(&selectors.blobs)?;
    if current_model {
        let expected_proposal_once = ZkGasModel
            .price_proposal_once(&ProposalPricingInput {
                canonical_transaction_bytes: residual_payload.canonical_transaction_bytes,
                nonempty_payload_transactions: residual_payload.nonempty_payload_transactions,
                true_unattempted_transactions: selectors.true_unattempted_transactions,
                source_blob_counts: &selectors.blobs.source_blob_counts,
                proposal_blob_hash_count: selectors.blobs.proposal_blob_hash_count,
                provider_blob_count: selectors.blobs.provider_blob_count,
                provider_commitment_count: selectors.blobs.provider_commitment_count,
                provider_proof_count: selectors.blobs.provider_proof_count,
                parent_mpt: mpt_selectors(selectors.parent_mpt),
            })
            .context("proposal selector domain is invalid")?;
        ensure!(
            snapshot_matches_pricing(&quote.prove.proposal_once, &expected_proposal_once),
            "proposal-once pricing does not match sealed selectors"
        );
    }
    Ok(())
}

/// A profile constrains every algorithm it names. Additional observed algorithms are provenance.
fn program_matches(required: &ProgramIdentity, observed: &ProgramIdentity) -> bool {
    required.elf_sha256 == observed.elf_sha256
        && required
            .serialized_vk_sha256
            .as_ref()
            .is_none_or(|v| observed.serialized_vk_sha256.as_ref() == Some(v))
        && required
            .sp1_vkey
            .as_ref()
            .is_none_or(|v| observed.sp1_vkey.as_ref() == Some(v))
        && required
            .risc0_image_id
            .as_ref()
            .is_none_or(|v| observed.risc0_image_id.as_ref() == Some(v))
}

fn score_matches(quote: &CompleteQuoteV1, profile: &CompatibilityProfileV1) -> bool {
    quote.prediction_surface == "prove"
        && quote.prediction_unit == "sp1_pgu"
        && profile.prediction_unit == quote.prediction_unit
        && profile.model_id == quote.quote.model_id
        && quote.calibration_profile == quote.quote.model_id
        && profile
            .runtime_spec_sha256
            .as_ref()
            .is_none_or(|v| v == &quote.quote.model_id)
}

fn observation_matches(
    observation: &CompleteObservationV1,
    profile: &CompatibilityProfileV1,
) -> bool {
    profile.backend == observation.backend
        && profile.measurement_stage == observation.measurement_stage
        && profile
            .backend_version
            .as_ref()
            .is_none_or(|v| observation.backend_version.as_ref() == Some(v))
}

pub fn resolve_profile(
    quote: &CompleteQuoteV1,
    observation: &CompleteObservationV1,
    profiles: &CompatibilityProfilesV1,
) -> ProfileResolution {
    let ids: Vec<_> = profiles
        .profiles
        .iter()
        .filter(|profile| {
            score_matches(quote, profile)
                && observation_matches(observation, profile)
                && profile.backend == "sp1"
                && profile.observation_unit == "sp1_pgu"
                && profile.observation_unit == observation.workload.unit
                && program_matches(&profile.program, &observation.program)
        })
        .map(|p| p.profile_id.clone())
        .collect();
    match ids.as_slice() {
        [] => ProfileResolution::NoMatch,
        [id] => ProfileResolution::Matched(id.clone()),
        _ => ProfileResolution::Ambiguous(ids),
    }
}

fn decimal(value: &str) -> Result<BigUint> {
    ensure!(
        !value.is_empty() && value.len() <= 1024 && value.bytes().all(|b| b.is_ascii_digit()),
        "invalid bounded unsigned decimal"
    );
    Ok(value.parse()?)
}

/// Verify both the original raw bytes and its separate content address, excluding the identity field.
pub fn verify_normalization_artifact(normalization: &NormalizationV1, bytes: &[u8]) -> Result<()> {
    ensure!(
        sha256_hex(bytes) == normalization.artifact_sha256,
        "normalization raw artifact hash mismatch"
    );
    let mut document: serde_json::Value = serde_json::from_slice(bytes)?;
    let claimed = document
        .as_object_mut()
        .context("normalization artifact must be an object")?
        .remove("artifact_sha256")
        .context("normalization artifact identity missing")?;
    ensure!(
        claimed.as_str() == Some(normalization.artifact_identity.as_str()),
        "normalization artifact identity mismatch"
    );
    ensure!(
        sha256_hex(&canonical_json(&document)?) == normalization.artifact_identity,
        "normalization internal content address mismatch"
    );
    // The wrapper rational is the lossless decimal interpretation, never a binary float.
    let scalar = document
        .get("kappa_unzen")
        .and_then(|v| v.as_str())
        .context("normalization kappa_unzen missing")?;
    let (integer, fraction) = scalar.split_once('.').unwrap_or((scalar, ""));
    let scalar_numerator = decimal(&format!("{integer}{fraction}"))?;
    let scalar_denominator = BigUint::from(10u32).pow(fraction.len().try_into()?);
    let numerator = decimal(&normalization.numerator)?;
    let denominator = decimal(&normalization.denominator)?;
    ensure!(!denominator.is_zero(), "zero normalization denominator");
    ensure!(
        numerator * scalar_denominator == scalar_numerator * denominator,
        "normalization rational differs from artifact scalar"
    );
    ensure!(
        normalization.diagnostic_only,
        "normalization must be diagnostic-only"
    );
    Ok(())
}

fn validate_profiles(profiles: &CompatibilityProfilesV1) -> Result<()> {
    ensure!(
        profiles.profiles.len() <= MAX_PROFILES,
        "profile count limit exceeded"
    );
    let mut ids = BTreeSet::new();
    for profile in &profiles.profiles {
        ensure!(
            !profile.profile_id.is_empty() && ids.insert(&profile.profile_id),
            "duplicate or empty profile ID"
        );
        if let Some(n) = &profile.normalization {
            ensure!(n.diagnostic_only, "normalization must be diagnostic-only");
            decimal(&n.numerator)?;
            ensure!(
                !decimal(&n.denominator)?.is_zero(),
                "zero normalization denominator"
            );
        }
    }
    Ok(())
}

pub fn embedded_profiles() -> Result<CompatibilityProfilesV1> {
    let profiles: CompatibilityProfilesV1 = toml::from_str(EMBEDDED_PROFILES)?;
    validate_profiles(&profiles)?;
    for profile in &profiles.profiles {
        if let Some(n) = &profile.normalization {
            verify_normalization_artifact(n, EMBEDDED_NORMALIZATION)?;
        }
    }
    Ok(profiles)
}

struct PinnedInput {
    file: File,
    length: u64,
}
impl PinnedInput {
    fn open(path: &Path, limit: u64) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
            .open(path)
            .with_context(|| format!("open {}", path.display()))?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "input must be a regular file");
        ensure!(metadata.len() <= limit, "per-file byte limit exceeded");
        Ok(Self {
            file,
            length: metadata.len(),
        })
    }
    fn read(self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.file.take(self.length + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == self.length,
            "input length changed after preflight"
        );
        Ok(bytes)
    }
}

fn normalized_diagnostic(
    quote: &CompleteQuoteV1,
    observation: &CompleteObservationV1,
    profiles: &CompatibilityProfilesV1,
) -> Result<Option<NormalizedDiagnosticV1>> {
    let candidates: Vec<_> = profiles
        .profiles
        .iter()
        .filter_map(|profile| {
            let n = profile.normalization.as_ref()?;
            (score_matches(quote, profile)
                && observation_matches(observation, profile)
                && n.source_unit == "unzen_zkgas"
                && n.target_unit == observation.workload.unit
                && n.target_unit == profile.observation_unit
                && program_matches(&n.source_program, &profile.program)
                && program_matches(&n.target_program, &observation.program))
            .then_some(n)
        })
        .collect();
    // No arbitrary choice between competing normalization declarations.
    let [n] = candidates.as_slice() else {
        return Ok(None);
    };
    let converted = ceil_mul_ratio(
        &BigUint::from(quote.quote.node.unzen_zkgas_sum),
        &decimal(&n.numerator)?,
        &decimal(&n.denominator)?,
    )?;
    Ok(Some(NormalizedDiagnosticV1 {
        normalization_profile_id: n.profile_id.clone(),
        normalization_sha256: content_id("zkgas-normalization-wrapper-v1", n)?,
        source_unit: n.source_unit.clone(),
        target_unit: n.target_unit.clone(),
        converted_prediction: converted.to_string(),
        diagnostic_only: true,
        prediction_over_actual: (observation.workload.value != 0)
            .then(|| ratio(&converted, &BigUint::from(observation.workload.value)))
            .transpose()?,
    }))
}

/// Public so consumers can recompute the tagged identity during integrity verification.
pub fn comparison_row_id(result: &ComparisonResultV1) -> Result<String> {
    match result {
        ComparisonResultV1::Joined {
            quote,
            observation_key,
            profile_id,
            ..
        } => content_id(
            "zkgas-comparison/joined-v1",
            &(&quote.key, observation_key, profile_id),
        ),
        ComparisonResultV1::IncompatibleProfile {
            quote,
            observation_key,
            profiles_sha256,
            ..
        } => content_id(
            "zkgas-comparison/incompatible-profile-v1",
            &(
                &quote.key,
                observation_key,
                profiles_sha256,
                "no_matching_profile",
            ),
        ),
        ComparisonResultV1::AmbiguousProfile {
            quote,
            observation_key,
            matching_profile_ids,
            ..
        } => content_id(
            "zkgas-comparison/ambiguous-profile-v1",
            &(&quote.key, observation_key, matching_profile_ids),
        ),
        ComparisonResultV1::UnmatchedQuote {
            quote,
            quote_row_sha256,
        } => content_id(
            "zkgas-comparison/unmatched-quote-v1",
            &(&quote.key, quote_row_sha256),
        ),
        ComparisonResultV1::UnmatchedObservation {
            observation_key,
            observation_row_sha256,
            ..
        } => content_id(
            "zkgas-comparison/unmatched-observation-v1",
            &(observation_key, observation_row_sha256),
        ),
        ComparisonResultV1::InvalidQuote {
            quote,
            quote_row_sha256,
        } => content_id(
            "zkgas-comparison/invalid-quote-v1",
            &(
                &quote.corpus_id,
                &quote.sample_id,
                &quote.verified_key,
                quote_row_sha256,
            ),
        ),
        ComparisonResultV1::InvalidObservation {
            observation_set_id,
            observation,
            observation_row_sha256,
        } => content_id(
            "zkgas-comparison/invalid-observation-v1",
            &(
                observation_set_id,
                &observation.observation_id,
                &observation.attempt_id,
                observation_row_sha256,
            ),
        ),
    }
}

pub fn comparison_kind(result: &ComparisonResultV1) -> ComparisonKind {
    match result {
        ComparisonResultV1::Joined { .. } => ComparisonKind::Joined,
        ComparisonResultV1::IncompatibleProfile { .. } => ComparisonKind::IncompatibleProfile,
        ComparisonResultV1::AmbiguousProfile { .. } => ComparisonKind::AmbiguousProfile,
        ComparisonResultV1::UnmatchedQuote { .. } => ComparisonKind::UnmatchedQuote,
        ComparisonResultV1::UnmatchedObservation { .. } => ComparisonKind::UnmatchedObservation,
        ComparisonResultV1::InvalidQuote { .. } => ComparisonKind::InvalidQuote,
        ComparisonResultV1::InvalidObservation { .. } => ComparisonKind::InvalidObservation,
    }
}

pub fn row_sequence_sha256(rows: &[RowDigestV1]) -> Result<String> {
    Ok(sha256_hex(&canonical_json(rows)?))
}

/// The canonical descriptor is the entire manifest with its self-referential run_id blanked.
pub fn comparison_run_id(manifest: &ComparisonManifestV1) -> Result<String> {
    let mut descriptor = manifest.clone();
    descriptor.run_id.clear();
    content_id("zkgas-comparison-run-v1", &descriptor)
}

struct RowBuilder {
    files: Vec<(String, Vec<u8>)>,
    digests: Vec<RowDigestV1>,
    counts: BTreeMap<ComparisonKind, u64>,
    bytes: usize,
    ids: BTreeSet<String>,
}
impl RowBuilder {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            digests: Vec::new(),
            counts: BTreeMap::new(),
            bytes: 0,
            ids: BTreeSet::new(),
        }
    }
    fn push(&mut self, result: ComparisonResultV1) -> Result<()> {
        let kind = comparison_kind(&result);
        let id = comparison_row_id(&result)?;
        // Failed quotes may have no verified primary key; their row identities must still never
        // alias one output path. This is a structural collision, not a fabricated quote join key.
        ensure!(self.ids.insert(id.clone()), "duplicate comparison row ID");
        let bytes = canonical_json(&ComparisonRowV1 {
            schema_version: 1,
            comparison_row_id: id.clone(),
            result,
        })?;
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .context("output byte count overflow")?;
        ensure!(
            self.bytes <= MAX_COMPARE_OUTPUT_BYTES,
            "serialized output byte limit exceeded"
        );
        self.digests.push(RowDigestV1 {
            comparison_row_id: id.clone(),
            row_sha256: sha256_hex(&bytes),
        });
        self.files.push((id, bytes));
        *self.counts.entry(kind).or_default() += 1;
        Ok(())
    }
}

pub fn run_compare(args: &CompareArgs, executable: &ExecutableIdentity) -> Result<CompareRun> {
    ensure!(
        !args.quotes.is_empty() && !args.observations.is_empty(),
        "at least one quote file and observation file required"
    );
    let initial_count = args
        .quotes
        .len()
        .checked_add(args.observations.len())
        .and_then(|n| n.checked_add(usize::from(args.profiles.is_some())))
        .context("file count overflow")?;
    ensure!(
        initial_count <= MAX_COMPARE_FILES,
        "input file count limit exceeded"
    );
    // Open and stat every input before reading any input content. Keep descriptors pinned.
    let mut pinned = Vec::new();
    let mut total_bytes = 0u64;
    for path in args
        .quotes
        .iter()
        .chain(&args.observations)
        .chain(args.profiles.iter())
    {
        let limit = if args.profiles.as_ref() == Some(path) {
            MAX_PROFILE_BYTES
        } else {
            MAX_COMPARE_FILE_BYTES
        };
        let input = PinnedInput::open(path, limit)?;
        total_bytes = total_bytes
            .checked_add(input.length)
            .context("aggregate byte count overflow")?;
        ensure!(
            total_bytes <= MAX_COMPARE_TOTAL_BYTES,
            "aggregate input byte limit exceeded"
        );
        pinned.push(input);
    }
    let (profiles, profiles_sha256) = if let Some(path) = &args.profiles {
        // Read only the bounded config first to discover artifact dependencies. Corpus contents
        // stay unopened for reading until those files have also passed aggregate preflight.
        let raw = pinned.pop().context("profile contents missing")?.read()?;
        let profiles: CompatibilityProfilesV1 = toml::from_str(std::str::from_utf8(&raw)?)?;
        validate_profiles(&profiles)?;
        let normalizations: Vec<_> = profiles
            .profiles
            .iter()
            .filter_map(|p| p.normalization.as_ref())
            .collect();
        ensure!(
            initial_count + normalizations.len() <= MAX_COMPARE_FILES,
            "input file count limit exceeded"
        );
        let root = File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let mut artifacts = Vec::new();
        for n in &normalizations {
            // Reuse the race-resistant confined relative opener from quote; no input decoding.
            let file = crate::quote::open_manifest_input(&root, Path::new(&n.artifact_path))?;
            let length = file.metadata()?.len();
            ensure!(
                length <= MAX_PROFILE_BYTES,
                "normalization artifact byte limit exceeded"
            );
            total_bytes = total_bytes
                .checked_add(length)
                .context("aggregate byte count overflow")?;
            ensure!(
                total_bytes <= MAX_COMPARE_TOTAL_BYTES,
                "aggregate input byte limit exceeded"
            );
            artifacts.push(PinnedInput { file, length });
        }
        for (n, artifact) in normalizations.into_iter().zip(artifacts) {
            verify_normalization_artifact(n, &artifact.read()?)?;
        }
        (profiles, sha256_hex(&raw))
    } else {
        (
            embedded_profiles()?,
            sha256_hex(EMBEDDED_PROFILES.as_bytes()),
        )
    };
    let mut quote_file_sha256 = Vec::new();
    let mut observation_file_sha256 = Vec::new();
    let mut quotes = Vec::new();
    let mut observations = Vec::new();
    let mut quote_keys = BTreeSet::new();
    let mut observation_keys = BTreeSet::new();
    let mut row_count = 0usize;
    for (index, input) in pinned.into_iter().enumerate() {
        let raw = input.read()?;
        if index < args.quotes.len() {
            quote_file_sha256.push(sha256_hex(&raw));
            for line in std::str::from_utf8(&raw)?.lines() {
                ensure!(
                    row_count < MAX_COMPARE_ROWS,
                    "total input row count limit exceeded"
                );
                row_count += 1;
                let row: QuoteRowV1 = serde_json::from_str(line)?;
                if let QuoteRowV1::Complete(quote) = &row {
                    validate_complete_quote(quote).context("complete quote invariant violation")?;
                }
                let key = match &row {
                    QuoteRowV1::Complete(q) => Some(&q.key),
                    QuoteRowV1::Failed(q) => q.verified_key.as_ref(),
                };
                if let Some(key) = key {
                    ensure!(
                        quote_keys.insert(key.clone()),
                        "duplicate quote primary key"
                    );
                }
                quotes.push((row, sha256_hex(line.as_bytes())));
            }
        } else {
            observation_file_sha256.push(sha256_hex(&raw));
            let manifest: ObservationManifestV1 = serde_json::from_slice(&raw)?;
            row_count = row_count
                .checked_add(manifest.observations.len())
                .context("input row count overflow")?;
            ensure!(
                row_count <= MAX_COMPARE_ROWS,
                "total input row count limit exceeded"
            );
            for observation in manifest.observations {
                let (id, attempt) = match &observation {
                    ObservationRowV1::Complete(o) => (&o.observation_id, &o.attempt_id),
                    ObservationRowV1::Failed(o) => (&o.observation_id, &o.attempt_id),
                };
                let key = ObservationKey {
                    observation_set_id: manifest.observation_set_id.clone(),
                    observation_id: id.clone(),
                    attempt_id: attempt.clone(),
                };
                ensure!(
                    observation_keys.insert(key.clone()),
                    "duplicate observation primary key"
                );
                let hash = sha256_hex(&canonical_json(&observation)?);
                observations.push((key, observation, hash));
            }
        }
    }
    ensure!(!quotes.is_empty(), "quote population is empty");
    ensure!(!observations.is_empty(), "observation population is empty");
    let mut builder = RowBuilder::new();
    let mut complete_quotes = BTreeMap::new();
    let mut quote_order = Vec::new();
    let mut model_ids = BTreeSet::new();
    for (row, hash) in quotes {
        match row {
            QuoteRowV1::Complete(q) => {
                model_ids.insert(q.quote.model_id.clone());
                quote_order.push(q.key.clone());
                complete_quotes.insert(q.key.clone(), (q, hash));
            }
            QuoteRowV1::Failed(q) => builder.push(ComparisonResultV1::InvalidQuote {
                quote: *q,
                quote_row_sha256: hash,
            })?,
        }
    }
    let mut matched = BTreeSet::new();
    for (key, observation, hash) in observations {
        match observation {
            ObservationRowV1::Failed(o) => {
                builder.push(ComparisonResultV1::InvalidObservation {
                    observation_set_id: key.observation_set_id,
                    observation: *o,
                    observation_row_sha256: hash,
                })?
            }
            ObservationRowV1::Complete(o) => {
                if let Some((q, _)) = complete_quotes.get(&o.quote_key) {
                    matched.insert(o.quote_key.clone());
                    let result = match resolve_profile(q, &o, &profiles) {
                        ProfileResolution::Matched(id) => ComparisonResultV1::Joined {
                            quote: q.clone(),
                            observation: *o.clone(),
                            observation_key: key,
                            profile_id: id,
                            metrics: direct_metrics(
                                &BigUint::from(q.quote.prove.total_pgu),
                                &BigUint::from(o.workload.value),
                            )?,
                            normalization: normalized_diagnostic(q, &o, &profiles)?,
                        },
                        ProfileResolution::NoMatch => ComparisonResultV1::IncompatibleProfile {
                            quote: q.clone(),
                            observation: *o.clone(),
                            observation_key: key,
                            profiles_sha256: profiles_sha256.clone(),
                            reason: "no_matching_profile".into(),
                            cross_program_ratio: (q.prediction_surface == "prove"
                                && q.prediction_unit == "sp1_pgu"
                                && o.workload.unit == q.prediction_unit
                                && o.workload.value != 0)
                                .then(|| {
                                    ratio(
                                        &BigUint::from(q.quote.prove.total_pgu),
                                        &BigUint::from(o.workload.value),
                                    )
                                })
                                .transpose()?,
                            normalization: normalized_diagnostic(q, &o, &profiles)?,
                        },
                        ProfileResolution::Ambiguous(ids) => ComparisonResultV1::AmbiguousProfile {
                            quote: q.clone(),
                            observation: *o,
                            observation_key: key,
                            matching_profile_ids: ids,
                        },
                    };
                    builder.push(result)?;
                } else {
                    builder.push(ComparisonResultV1::UnmatchedObservation {
                        observation: *o,
                        observation_key: key,
                        observation_row_sha256: hash,
                    })?;
                }
            }
        }
    }
    for key in quote_order {
        if !matched.contains(&key) {
            let (quote, hash) = complete_quotes.remove(&key).context("quote disappeared")?;
            builder.push(ComparisonResultV1::UnmatchedQuote {
                quote,
                quote_row_sha256: hash,
            })?;
        }
    }
    ensure!(
        !builder.digests.is_empty(),
        "comparison population is empty"
    );
    let status = if builder.counts.keys().all(|kind| {
        matches!(
            kind,
            ComparisonKind::Joined | ComparisonKind::IncompatibleProfile
        )
    }) {
        RunStatus::Complete
    } else {
        RunStatus::Partial
    };
    let mut manifest = ComparisonManifestV1 {
        schema_version: 1,
        run_id: String::new(),
        executable: executable.clone(),
        quote_file_sha256,
        observation_file_sha256,
        profiles_sha256,
        arithmetic_policy: ARITHMETIC_POLICY.into(),
        model_ids: model_ids.into_iter().collect(),
        profile_ids: profiles
            .profiles
            .iter()
            .map(|p| p.profile_id.clone())
            .collect(),
        row_sequence_sha256: row_sequence_sha256(&builder.digests)?,
        rows: builder.digests,
        status,
        counts: builder.counts,
    };
    manifest.run_id = comparison_run_id(&manifest)?;
    let runs = args.output.join("runs");
    fs::create_dir_all(&runs)?;
    let staging = runs.join(format!(".zkgas-comparison-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staging)?;
    let run_path = runs.join(&manifest.run_id);
    let published = (|| {
        fs::create_dir(staging.join("rows"))?;
        fs::write(staging.join("manifest.json"), canonical_json(&manifest)?)?;
        for (id, bytes) in builder.files {
            fs::write(staging.join("rows").join(format!("{id}.json")), bytes)?;
        }
        commit_directory_bounded(
            &staging,
            &run_path,
            DirectoryCommitLimits {
                max_entries: MAX_COMPARE_ROWS + 2,
                max_depth: 2,
                max_bytes: MAX_COMPARE_OUTPUT_BYTES as u64,
            },
        )
    })();
    if published.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    Ok(CompareRun {
        manifest,
        run_path,
        commit: published?,
    })
}
