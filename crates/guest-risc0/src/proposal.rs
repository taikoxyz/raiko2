//! RISC0 guest program for single proposal proof
//!
//! This guest receives GuestInput (containing witnesses for each block),
//! performs stateless validation through the witness data, and produces
//! a protocol instance hash as output.
#![no_main]
risc0_zkvm::guest::entry!(main);

use alloy_primitives::B256;
use raiko2_primitives::{
    instance::{ProtocolInstance, ShastaBatchMetadata, ShastaTransition},
    GuestInput,
};
use raiko2_protocol::{hash_shasta_subproof_input, ProofCarryData};
use raiko2_stateless::validate_block;
use reth_chainspec::{ChainSpec, HOLESKY, HOODI, MAINNET, SEPOLIA};
use reth_evm_ethereum::EthEvmConfig;
use risc0_zkvm::guest::env;
use std::sync::Arc;

pub fn main() {
    // Read the guest input prepared by the host
    let guest_input: GuestInput = env::read();

    // Read the proof carry data that contains the transition input
    let proof_carry_data: ProofCarryData = env::read();

    // Validate that guests input is not empty
    assert!(
        !guest_input.witnesses.is_empty(),
        "GuestInput must contain at least one witness"
    );

    // Select the chain spec based on the per-block ChainSpec provided in StatelessInput.
    //
    // Today, raiko2-stateless expects a `reth_chainspec::ChainSpec` + `EthEvmConfig`.
    // We map from the provided chain_id to the closest built-in reth chain specs.
    let mut selected_chain_id: Option<u64> = None;
    let mut chain_spec: Option<Arc<ChainSpec>> = None;
    let mut evm_config: Option<EthEvmConfig> = None;

    // =========================================================================
    // Witness Validation
    // =========================================================================
    // In the zkVM context, we verify the witness data for each block.
    // Each StatelessInput contains:
    // - block: The block being executed
    // - witness: ExecutionWitness for stateless validation
    // - accounts: The accounts being accessed in validation
    //
    // The witness provides the cryptographic proof that the state transitions
    // are valid without requiring the full state trie.

    // Collect blocks from witnesses for protocol instance construction
    let mut blocks = Vec::with_capacity(guest_input.witnesses.len());

    let mut previous_block_hash: Option<B256> = None;

    for (index, stateless_input) in guest_input.witnesses.iter().enumerate() {
        let this_chain_id = stateless_input.chain_spec.chain_id;

        // raiko2-stateless currently uses Ethereum beacon consensus + Eth EVM config.
        // Taiko L2 chains have different fork IDs and header validity rules, so we
        // refuse to validate them with the Ethereum-L1 configuration.
        assert!(
            !stateless_input.chain_spec.is_taiko,
            "taiko chain specs are not supported by raiko2-stateless in risc0-proposal (chain_id={this_chain_id})"
        );

        if selected_chain_id != Some(this_chain_id) {
            let spec = chain_spec_for_chain_id(this_chain_id);
            evm_config = Some(EthEvmConfig::ethereum(spec.clone()));
            chain_spec = Some(spec);
            selected_chain_id = Some(this_chain_id);
        }

        let chain_spec = chain_spec.as_ref().expect("chain spec not initialized");
        let evm_config = evm_config.as_ref().expect("evm config not initialized");

        if let Some(prev_hash) = previous_block_hash {
            assert_eq!(
                stateless_input.block.header.parent_hash, prev_hash,
                "block {index} must link to previous block hash"
            );
        }

        // Perform full stateless validation (exec + post-state root check) using
        // the witness data. This ensures the proof binds to a correct state
        // transition for each block.
        let validated_hash = validate_block(
            stateless_input.block.clone(),
            stateless_input.witness.clone(),
            stateless_input.accounts.clone(),
            chain_spec.clone(),
            evm_config.clone(),
        )
        .expect("stateless block validation failed");

        previous_block_hash = Some(validated_hash);

        blocks.push(stateless_input.block.clone());
    }

    // =========================================================================
    // Protocol Instance Construction
    // =========================================================================
    // Build the protocol instance from validated blocks.
    // This binds the transition data for cryptographic commitment.

    let first_block = blocks.first().expect("At least one block required");
    let last_block = blocks.last().expect("At least one block required");

    // Build transition from blocks
    let transition = ShastaTransition {
        parent_hash: first_block.header.parent_hash,
        block_hash: last_block.header.hash_slow(),
        state_root: last_block.header.state_root,
    };

    // Build batch metadata from taiko manifest
    let batch_metadata = ShastaBatchMetadata {
        info_hash: B256::ZERO, // Will be computed from tx data
        proposer: proof_carry_data.transition_input.transition.proposer,
        batch_id: guest_input.taiko.batch_id,
        proposed_at: proof_carry_data.transition_input.transition.timestamp,
    };

    // Construct the protocol instance
    let protocol_instance = ProtocolInstance {
        transition,
        batch_metadata,
        prover: proof_carry_data.transition_input.actual_prover,
        chain_id: proof_carry_data.chain_id,
        verifier_address: proof_carry_data.verifier,
    };

    // =========================================================================
    // Hash Computation and Commitment
    // =========================================================================
    // Compute the protocol instance hash that binds all validated data.
    // This hash serves as the public output that the aggregator will verify.

    let instance_hash = protocol_instance.instance_hash();

    // Also compute the subproof input hash for aggregation compatibility
    // This binds chain_id, verifier, and all transition data
    let subproof_input_hash = hash_shasta_subproof_input(&proof_carry_data);

    // Commit both hashes as the public output
    // The aggregator will verify these match the expected values
    env::commit_slice(instance_hash.as_slice());
    env::commit_slice(subproof_input_hash.as_slice());
}

fn chain_spec_for_chain_id(chain_id: u64) -> Arc<ChainSpec> {
    match chain_id {
        1 => MAINNET.clone(),
        17000 => HOLESKY.clone(),
        11155111 => SEPOLIA.clone(),
        560048 => HOODI.clone(),
        _ => MAINNET.clone(),
    }
}
