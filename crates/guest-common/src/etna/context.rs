//! Deterministic Etna acceptance context for the explicitly FI-omitting native PoC.

use super::{verify_range_input_consensus, ConsensusParameters, EtnaBlockExecutionWitness};
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::{spec::TaikoDevnetConfigExt, TAIKO_DEVNET};
use alloy_consensus::Header;
use alloy_primitives::{b256, keccak256, Address, B256, U256};
use anyhow::{ensure, Context, Result};
use etna_core::{
    domain::DeploymentDomain,
    extra_data::ExtraData,
    l1_header::L1HeaderView,
    params::{ChainParams, UINT48_MAX},
    types::OriginRecord,
};
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::{
    consensus::{authenticate_origin, origin_slots},
    input::ConsensusRangeInput,
    l1_history::L1HistoryLimits,
    range::RangeProjectionContext,
};
use raiko2_stateless::read_storage_batch_with_witness_resources;
use std::{collections::BTreeMap, sync::Arc};
use tendermint::Time;

const IMPLEMENTATION_SLOT: B256 =
    b256!("360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc");
const DEVNET_CHAIN_ID: u64 = 167_001;
const DEVNET_BLOCK_GAS_LIMIT: u64 = 45_000_000;

/// Caller-selected immutable image/profile policy; never accepted from the serialized request.
#[derive(Clone, Debug)]
pub struct EtnaAcceptancePolicy {
    /// Existing deployment parameters, including any trusted switch implementation constants.
    pub chain: ChainParams,
    /// Trusted L2 SignalService proxy address for switch-start implementation proofs.
    pub signal_service_proxy: Address,
    /// Maximum covered blocks.
    pub max_blocks: usize,
    /// Maximum private envelope bytes, independently of the fixed 124,000-byte execution item cap.
    pub max_envelope_bytes: usize,
    /// Maximum original native block protobuf bytes.
    pub max_block_bytes: usize,
    /// Maximum standalone native certificate protobuf bytes.
    pub max_commit_bytes: usize,
    /// Trusted history witness budgets.
    pub history_limits: L1HistoryLimits,
}

impl EtnaAcceptancePolicy {
    /// Builds the fixed internal-devnet PoC profile from statement-bound deployment words.
    ///
    /// The runtime still checks the statement digest and SGX verifier domain before signing.
    /// This profile has no production switching constants; a range starting at a nonzero S
    /// fails closed until a caller supplies a reviewed switching profile instead.
    #[must_use]
    pub fn devnet(
        l1_chain_id: U256,
        inbox: Address,
        staking: Address,
        unsettled_limit: u64,
    ) -> Self {
        Self {
            chain: ChainParams {
                l2_chain_id: DEVNET_CHAIN_ID,
                inbox,
                staking,
                l1_chain_id,
                unsettled_limit,
                fee_recipient: Address::ZERO,
                block_gas_limit: DEVNET_BLOCK_GAS_LIMIT,
                switch: None,
            },
            signal_service_proxy: raiko2_primitives::shasta_checkpoint_store_address(
                DEVNET_CHAIN_ID,
            )
            .expect("the fixed devnet chain has a canonical SignalService proxy address"),
            max_blocks: 64,
            max_envelope_bytes: 1_500_000,
            max_block_bytes: 2_097_152,
            max_commit_bytes: 524_288,
            history_limits: L1HistoryLimits {
                max_headers: 1024,
                max_proof_nodes: 65_536,
            },
        }
    }
}

/// Selects the reviewed built-in devnet EVM and enables Etna for this complete range.
///
/// Only Etna activation changes; all other fork parameters come from the existing built-in spec.
/// This does not interpret an arbitrary serialized chainspec or enable unsupported production
/// staking profiles. Context predicates still validate every supplied timestamp and header.
///
/// # Errors
/// Rejects unsupported chain IDs or an invalid built-in Etna activation override.
pub fn etna_devnet_config(chain_id: u64, first_timestamp: u64) -> Result<TaikoEvmConfig> {
    ensure!(
        chain_id == DEVNET_CHAIN_ID,
        "Etna PoC supports only the built-in internal devnet profile"
    );
    Ok(TaikoEvmConfig::new(Arc::new(
        TAIKO_DEVNET
            .clone_with_devnet_etna_timestamp(first_timestamp)
            .context("invalid built-in devnet Etna activation")?,
    )))
}

/// Verifies the native Etna relation and all retained deterministic acceptance predicates.
///
/// The domain, p/h and endpoint identities come directly from the exact requested statement.
/// L1 recomputes these public words when accepting a landing; this function does not turn
/// deserialized context or an enclave RPC answer into independent authority. Q's original raw
/// header must open the statement's Q identity. Origin/config/max-age and per-anchor context
/// come from existing MPT witnesses under Q or an ordinarily authenticated SSZ/direct-Q root.
/// The p-only anchor exception supplies committee inputs only, never these context predicates.
///
/// This first PoC deliberately omits forced-inclusion eligibility/skip semantics. Existing
/// native verification rejects carried FI witnesses and cursor consumption; callers must not
/// advertise this image as a production FI-capable proof relation. No queue flag is trusted.
/// Stock `app_hash`, proposer priority and wall-clock admission remain outside the guest statement.
/// SGX verifier-domain hashing and signing belong to the runtime after this function succeeds.
///
/// # Errors
/// Rejects any native component, trusted-policy, public-word, origin/storage, timestamp/age,
/// generation/backpressure, supported-rules/stop, gas/mix-hash or switch-implementation failure.
#[allow(clippy::too_many_arguments)]
pub fn verify_range_input_acceptance(
    input: &ConsensusRangeInput<ExecutionWitness>,
    source: &InputDataSource,
    execution_witnesses: &[EtnaBlockExecutionWitness<'_>],
    shared_state_nodes: &[WitnessStateNode],
    config: &TaikoEvmConfig,
    policy: &EtnaAcceptancePolicy,
) -> Result<B256> {
    policy.chain.validate()?;
    ensure!(
        !input.range.blocks.is_empty() && input.range.blocks.len() <= policy.max_blocks,
        "Etna acceptance range exceeds trusted block budget"
    );
    let statement = input.range.statement.as_statement();
    ensure!(
        statement.l1_chain_id == policy.chain.l1_chain_id
            && statement.l2_chain_id == U256::from(policy.chain.l2_chain_id)
            && statement.inbox == policy.chain.inbox,
        "Etna statement deployment differs from acceptance policy"
    );
    let generation = protocol_u48(statement.generation, "generation")?;
    let parent_height = protocol_u48(statement.input.parent_height, "parent height")?;
    let last_height = protocol_u48(statement.input.last_height, "last height")?;
    ensure!(
        parent_height < last_height,
        "invalid Etna acceptance endpoints"
    );
    let reference_number = protocol_u48(
        statement.input.l1_reference_block_number,
        "reference number",
    )?;
    let reference = L1HeaderView::from_raw(&input.range.l1_history.reference_header)?;
    ensure!(
        reference.number() == reference_number
            && reference.hash() == statement.l1_reference_block_hash,
        "Etna acceptance reference header identity mismatch"
    );
    // Only Q is read before the native composer: its identity is already the statement's exact
    // public reference. No p-anchor alternative is used to bootstrap ordinary context authority.
    let mut slots = vec![
        (policy.chain.inbox, U256::from(272)),
        (policy.chain.inbox, U256::from(282)),
    ];
    slots.extend(
        origin_slots(generation).map(|slot| (policy.chain.inbox, U256::from_be_bytes(slot.0))),
    );
    let words = read_storage_batch_with_witness_resources(
        reference.state_root(),
        &slots,
        &input.consensus.reference_state,
        shared_state_nodes,
    )?;
    ensure!(
        (words[0] >> 96usize).is_zero(),
        "noncanonical Etna genesis height/L1 origin word"
    );
    let mask = U256::from(UINT48_MAX);
    let genesis_height = protocol_u48(words[0] & mask, "genesis height")?;
    let l1_0 = protocol_u48((words[0] >> 48usize) & mask, "genesis L1 origin")?;
    let max_anchor_age = decode_max_anchor_age(words[1])?;
    let origin_words: [U256; 4] = words[2..].try_into().context("Etna origin word count")?;
    let origin_height = protocol_u48(origin_words[0], "generation origin height")?;
    let origin = authenticate_origin(
        &origin_words,
        origin_height,
        &input.consensus.origin_fallback_keys,
    )?;
    ensure!(
        origin_height >= genesis_height && origin.l1_number >= l1_0,
        "Etna generation origin precedes deployment start"
    );
    if generation == 0 {
        ensure!(
            origin_height == genesis_height && origin.l1_number == l1_0,
            "Etna generation-zero origin differs from deployment start"
        );
    }
    let context = RangeProjectionContext {
        domain: DeploymentDomain {
            l1_chain_id: statement.l1_chain_id,
            l2_chain_id: statement.l2_chain_id,
            inbox: statement.inbox,
            generation,
        },
        origin_height,
        parent_height,
        last_height,
        parent_el_hash: statement.parent_hash,
        parent_comet_hash: statement.parent_comet_hash,
        last_el_hash: statement.input.last_block_hash,
        last_comet_hash: statement.input.last_comet_hash,
        max_blocks: policy.max_blocks,
        max_envelope_bytes: policy.max_envelope_bytes,
    };
    let parameters = ConsensusParameters {
        staking: policy.chain.staking,
        max_uncheckpointed: policy.chain.unsettled_limit,
        max_block_bytes: policy.max_block_bytes,
        max_commit_bytes: policy.max_commit_bytes,
    };
    let digest = verify_range_input_consensus(
        input,
        source,
        &context,
        execution_witnesses,
        shared_state_nodes,
        policy.history_limits,
        config,
        &parameters,
    )?;
    // The same immutable raw anchor bytes below have now passed ordinary SSZ/direct-Q history
    // verification and exact EL root binding in the native composer. No second history mode or
    // advisory decoded AnchorState is introduced.
    let anchors = authenticated_anchor_contexts(input, shared_state_nodes, policy.chain.inbox)?;
    let mut parent = &input.range.parent_execution_header;
    for block in &input.range.blocks {
        let anchor_number = ExtraData::decode(block.execution_header.extra_data.as_ref())?.anchor;
        let anchor = anchors
            .get(&anchor_number)
            .context("missing ordinarily authenticated Etna anchor context")?;
        verify_header_context(
            &block.execution_header,
            parent,
            block.comet_header.time,
            anchor,
            &origin,
            generation,
            policy.chain.unsettled_limit,
            max_anchor_age,
            policy.chain.block_gas_limit,
        )?;
        parent = &block.execution_header;
    }
    if parent_height == genesis_height && genesis_height != 0 {
        let switch = policy
            .chain
            .switch
            .as_ref()
            .context("missing trusted Etna switching profile at S")?;
        let parent = &input.range.parent_execution_header;
        ensure!(
            parent.timestamp < switch.fork_timestamp,
            "Etna switching origin timestamp reaches fork time"
        );
        ensure!(
            !policy.signal_service_proxy.is_zero(),
            "missing trusted L2 SignalService proxy address"
        );
        let witness = execution_witnesses
            .first()
            .context("missing Etna switch-start witness")?;
        let values = read_storage_batch_with_witness_resources(
            parent.state_root,
            &[
                (
                    policy.signal_service_proxy,
                    U256::from_be_bytes(IMPLEMENTATION_SLOT.0),
                ),
                (
                    policy.chain.staking,
                    U256::from_be_bytes(IMPLEMENTATION_SLOT.0),
                ),
            ],
            witness.witness,
            shared_state_nodes,
        )?;
        verify_implementation_word(values[0], switch.signal_service_implementation)?;
        verify_implementation_word(values[1], switch.staking_implementation)?;
    }
    Ok(digest)
}

fn protocol_u48(word: U256, field: &str) -> Result<u64> {
    let value = u64::try_from(word).with_context(|| format!("Etna {field} exceeds uint48"))?;
    ensure!(value <= UINT48_MAX, "Etna {field} exceeds uint48");
    Ok(value)
}

fn decode_max_anchor_age(word: U256) -> Result<u64> {
    ensure!(
        (word >> 96usize).is_zero(),
        "noncanonical Etna forced-inclusion/max-anchor-age word"
    );
    let age = protocol_u48(word >> 48usize, "maximum anchor age")?;
    ensure!(
        age >= 1152,
        "Etna maximum anchor age is below protocol floor"
    );
    Ok(age)
}

#[derive(Clone, Copy, Debug)]
struct Rules {
    active: u64,
    next: u64,
    time: u64,
    stop: u64,
}
impl Rules {
    fn decode(word: U256) -> Result<Self> {
        ensure!(
            (word >> 192usize).is_zero(),
            "noncanonical Etna rules schedule word"
        );
        let field = |offset| {
            protocol_u48(
                (word >> offset) & U256::from(UINT48_MAX),
                "rules schedule field",
            )
        };
        Ok(Self {
            active: field(0usize)?,
            next: field(48usize)?,
            time: field(96usize)?,
            stop: field(144usize)?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct AnchorContext {
    number: u64,
    timestamp: u64,
    generation: U256,
    checkpoint: u64,
    rules: Rules,
}

fn authenticated_anchor_contexts(
    input: &ConsensusRangeInput<ExecutionWitness>,
    shared: &[WitnessStateNode],
    inbox: Address,
) -> Result<BTreeMap<u64, AnchorContext>> {
    let mut result = BTreeMap::new();
    for anchor in &input.range.l1_history.anchors {
        let header = L1HeaderView::from_raw(&anchor.raw_header)?;
        let resource = input
            .consensus
            .anchors
            .binary_search_by_key(&header.number(), |resource| resource.anchor_number)
            .ok()
            .and_then(|index| input.consensus.anchors.get(index))
            .context("missing ordinary Etna anchor storage witness")?;
        let words = read_storage_batch_with_witness_resources(
            header.state_root(),
            &[
                (inbox, U256::from(268)),
                (inbox, U256::from(270)),
                (inbox, U256::from(293)),
            ],
            &resource.witness,
            shared,
        )?;
        result.insert(
            header.number(),
            AnchorContext {
                number: header.number(),
                timestamp: header.timestamp(),
                generation: words[0],
                checkpoint: protocol_u48(words[1], "checkpoint height")?,
                rules: Rules::decode(words[2])?,
            },
        );
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn verify_header_context(
    header: &Header,
    parent: &Header,
    time: Time,
    anchor: &AnchorContext,
    origin: &OriginRecord,
    generation: u64,
    unsettled_limit: u64,
    max_anchor_age: u64,
    gas_limit: u64,
) -> Result<()> {
    ensure!(
        anchor.number >= origin.l1_number,
        "Etna anchor is below generation L1 origin"
    );
    ensure!(
        anchor.generation == U256::from(generation),
        "Etna anchor generation mismatch"
    );
    let unsettled = header
        .number
        .checked_sub(anchor.checkpoint)
        .context("Etna anchor checkpoint is above block height")?;
    ensure!(
        unsettled <= unsettled_limit,
        "Etna block exceeds unsettled limit U"
    );
    let bft_seconds =
        u64::try_from(time.unix_timestamp()).context("Etna Comet time is negative")?;
    if header.number == origin.start_height + 1 {
        ensure!(
            time.unix_timestamp_nanos() == i128::from(origin.l1_timestamp) * 1_000_000_000,
            "Etna initial Comet time differs from generation origin"
        );
    }
    let expected_time = parent
        .timestamp
        .checked_add(1)
        .context("Etna parent timestamp overflow")?
        .max(bft_seconds)
        .max(anchor.timestamp);
    ensure!(
        header.timestamp == expected_time,
        "Etna execution timestamp differs from exact parent/Comet/anchor maximum"
    );
    let age = header
        .timestamp
        .checked_sub(anchor.timestamp)
        .context("Etna timestamp precedes anchor")?;
    ensure!(age <= max_anchor_age, "Etna anchor exceeds maximum age");
    let selected = if header.timestamp >= anchor.rules.time {
        anchor.rules.next
    } else {
        anchor.rules.active
    };
    ensure!(
        selected == 0,
        "Etna selected ruleset is unsupported by this image"
    );
    ensure!(
        anchor.rules.stop == 0 || parent.timestamp < anchor.rules.stop,
        "Etna parent timestamp reaches planned stop"
    );
    ensure!(
        header.gas_limit == gas_limit,
        "Etna block gas limit differs from trusted profile"
    );
    ensure!(
        header.mix_hash == expected_mix_hash(parent.difficulty, header.number),
        "Etna block mix hash mismatch"
    );
    Ok(())
}

fn expected_mix_hash(parent_difficulty: U256, height: u64) -> B256 {
    let mut words = [0u8; 64];
    words[..32].copy_from_slice(&parent_difficulty.to_be_bytes::<32>());
    words[32..].copy_from_slice(&U256::from(height).to_be_bytes::<32>());
    keccak256(words)
}

fn verify_implementation_word(value: U256, expected: Address) -> Result<()> {
    ensure!(
        (value >> 160usize).is_zero() && Address::from_word(B256::from(value)) == expected,
        "Etna switch-start proxy implementation mismatch or noncanonical address word"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
