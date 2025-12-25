//! Helpers for zkVM guest programs.

use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::spec::TaikoChainSpec;
use alloy_primitives::B256;
use anyhow::{ensure, Context, Result};
use raiko2_primitives::{
    blob_utils::verify_proposal_mode_blob_usage,
    instance::{
        build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
        shasta_zk_aggregation_output, ProtocolInstance, ShastaProposalMetadata, ShastaTransition,
    },
    GuestInput, ShastaZkAggregationGuestInput, StatelessInput,
};
use raiko2_protocol::{hash_shasta_subproof_input, ProofCarryData};
use raiko2_stateless::validate_block;
use std::sync::Arc;

pub struct TaikoRuntime {
    pub chain_spec: Arc<TaikoChainSpec>,
    pub evm_config: TaikoEvmConfig,
}

impl TaikoRuntime {
    fn from_chain_spec(chain_spec: &raiko2_primitives::ChainSpec) -> Result<Self> {
        let chain_spec = chain_spec.to_taiko_chain_spec()?;
        let evm_config = TaikoEvmConfig::new(chain_spec.clone());
        Ok(Self {
            chain_spec,
            evm_config,
        })
    }
}

pub fn prove_shasta_proposal(
    guest_input: &GuestInput,
    proof_carry_data: &ProofCarryData,
) -> Result<(B256, B256)> {
    verify_proposal_mode_blob_usage(guest_input)
        .context("proposal mode blob usage verification failed")?;

    prove_shasta_proposal_with_validator(
        guest_input,
        proof_carry_data,
        |stateless_input, runtime| {
            validate_block(
                stateless_input.block.clone(),
                stateless_input.witness.clone(),
                stateless_input.accounts.clone(),
                runtime.chain_spec.clone(),
                runtime.evm_config.clone(),
            )
            .map_err(|e| anyhow::anyhow!(e))
        },
    )
}

pub fn prove_shasta_proposal_with_validator<V>(
    guest_input: &GuestInput,
    proof_carry_data: &ProofCarryData,
    mut validate_block: V,
) -> Result<(B256, B256)>
where
    V: FnMut(&StatelessInput, &TaikoRuntime) -> Result<B256>,
{
    ensure!(
        !guest_input.witnesses.is_empty(),
        "GuestInput must contain at least one witness"
    );

    let first_chain_spec = &guest_input.witnesses.first().expect("checked").chain_spec;

    for (i, witness) in guest_input.witnesses.iter().enumerate() {
        ensure!(
            witness.chain_spec.chain_id == first_chain_spec.chain_id,
            "witness {i} chain_id mismatch: expected {}, got {}",
            first_chain_spec.chain_id,
            witness.chain_spec.chain_id
        );
        ensure!(
            witness.chain_spec.is_taiko == first_chain_spec.is_taiko,
            "witness {i} is_taiko mismatch: expected {}, got {}",
            first_chain_spec.is_taiko,
            witness.chain_spec.is_taiko
        );
    }

    ensure!(
        proof_carry_data.chain_id == first_chain_spec.chain_id,
        "proof_carry_data.chain_id mismatch: expected {}, got {}",
        first_chain_spec.chain_id,
        proof_carry_data.chain_id
    );

    let runtime = TaikoRuntime::from_chain_spec(first_chain_spec)
        .context("Failed to build Taiko runtime from GuestInput chain_spec")?;

    let mut blocks = Vec::with_capacity(guest_input.witnesses.len());
    let mut previous_block_hash: Option<B256> = None;

    for (index, stateless_input) in guest_input.witnesses.iter().enumerate() {
        if let Some(prev_hash) = previous_block_hash {
            ensure!(
                stateless_input.block.header.parent_hash == prev_hash,
                "block {index} must link to previous block hash"
            );
        }

        let validated_hash = validate_block(stateless_input, &runtime)
            .with_context(|| format!("stateless block validation failed at index {index}"))?;
        previous_block_hash = Some(validated_hash);

        blocks.push(stateless_input.block.clone());
    }

    let first_block = blocks.first().expect("checked");
    let last_block = blocks.last().expect("checked");

    let transition = ShastaTransition {
        parent_hash: first_block.header.parent_hash,
        block_hash: last_block.header.hash_slow(),
        state_root: last_block.header.state_root,
    };

    let proposal_metadata = ShastaProposalMetadata {
        info_hash: B256::ZERO,
        proposer: proof_carry_data.transition_input.transition.proposer,
        proposal_id: guest_input.taiko.proposal_id,
        proposed_at: proof_carry_data.transition_input.transition.timestamp,
    };

    let protocol_instance = ProtocolInstance {
        transition,
        proposal_metadata,
        prover: proof_carry_data.transition_input.actual_prover,
        chain_id: proof_carry_data.chain_id,
        verifier_address: proof_carry_data.verifier,
    };

    let instance_hash = protocol_instance.instance_hash();
    let subproof_input_hash = hash_shasta_subproof_input(proof_carry_data);

    Ok((instance_hash, subproof_input_hash))
}

pub fn aggregate_shasta_zk_with_verifier<V>(
    input: &ShastaZkAggregationGuestInput,
    image_id_b256: B256,
    mut verify_proof: V,
) -> Result<B256>
where
    V: FnMut(usize, &B256) -> Result<()>,
{
    ensure!(
        input.block_inputs.len() == input.proof_carry_data_vec.len(),
        "block_inputs/proof_carry_data_vec length mismatch: {} vs {}",
        input.block_inputs.len(),
        input.proof_carry_data_vec.len()
    );
    ensure!(
        !input.proof_carry_data_vec.is_empty(),
        "proof_carry_data_vec must not be empty"
    );

    for (i, block_input) in input.block_inputs.iter().enumerate() {
        verify_proof(i, block_input)
            .with_context(|| format!("proof verification failed at index {i}"))?;

        let expected = hash_shasta_subproof_input(&input.proof_carry_data_vec[i]);
        ensure!(
            *block_input == expected,
            "block_input mismatch at index {i}"
        );
    }

    let commitment = build_shasta_commitment_from_proof_carry_data_vec(&input.proof_carry_data_vec)
        .context("invalid proof_carry_data_vec")?;
    let first = input.proof_carry_data_vec.first().expect("checked");
    let aggregation_hash = shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        input.prover_address,
    );

    Ok(shasta_zk_aggregation_output(
        image_id_b256,
        aggregation_hash,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use raiko2_primitives::chain_spec::Eip1559Constants;
    use raiko2_primitives::instance::{ProtocolInstance, ShastaProposalMetadata, ShastaTransition};
    use raiko2_primitives::{ChainSpec, StatelessInput, TaikoManifest};
    use raiko2_protocol::{hash_shasta_subproof_input, TransitionInputData};
    use reth_revm::primitives::hardfork::SpecId;

    fn taiko_mainnet_chain_spec() -> ChainSpec {
        ChainSpec::new_single(
            "taiko_mainnet".to_string(),
            167000,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        )
    }

    #[test]
    fn prove_shasta_proposal_builds_expected_hashes() {
        let chain_spec = taiko_mainnet_chain_spec();

        let mut input = StatelessInput {
            chain_spec,
            ..Default::default()
        };
        input.block.header.parent_hash = B256::from([9u8; 32]);
        input.block.header.state_root = B256::from([1u8; 32]);

        let guest_input = GuestInput {
            witnesses: vec![input.clone()],
            taiko: TaikoManifest {
                proposal_id: 42,
                ..Default::default()
            },
        };

        let proof_carry_data = ProofCarryData {
            chain_id: 167000,
            verifier: Address::from([0x11; 20]),
            transition_input: TransitionInputData {
                actual_prover: Address::from([0x22; 20]),
                transition: raiko2_protocol::ShastaTransitionInput {
                    proposer: Address::from([0x33; 20]),
                    designatedProver: Address::ZERO,
                    timestamp: 123,
                },
                ..Default::default()
            },
        };

        let (instance_hash, subproof_input_hash) = prove_shasta_proposal_with_validator(
            &guest_input,
            &proof_carry_data,
            |stateless_input, _runtime| Ok(stateless_input.block.header.hash_slow()),
        )
        .expect("proposal proving should succeed");

        let transition = ShastaTransition {
            parent_hash: input.block.header.parent_hash,
            block_hash: input.block.header.hash_slow(),
            state_root: input.block.header.state_root,
        };

        let proposal_metadata = ShastaProposalMetadata {
            info_hash: B256::ZERO,
            proposer: proof_carry_data.transition_input.transition.proposer,
            proposal_id: guest_input.taiko.proposal_id,
            proposed_at: proof_carry_data.transition_input.transition.timestamp,
        };

        let expected_instance = ProtocolInstance {
            transition,
            proposal_metadata,
            prover: proof_carry_data.transition_input.actual_prover,
            chain_id: proof_carry_data.chain_id,
            verifier_address: proof_carry_data.verifier,
        };

        assert_eq!(instance_hash, expected_instance.instance_hash());
        assert_eq!(
            subproof_input_hash,
            hash_shasta_subproof_input(&proof_carry_data)
        );
    }

    #[test]
    fn rejects_parent_hash_mismatch() {
        let chain_spec = taiko_mainnet_chain_spec();

        let mut first = StatelessInput {
            chain_spec: chain_spec.clone(),
            ..Default::default()
        };
        first.block.header.parent_hash = B256::from([1u8; 32]);

        let first_hash = first.block.header.hash_slow();

        let mut second = StatelessInput {
            chain_spec,
            ..Default::default()
        };
        second.block.header.parent_hash = B256::from([2u8; 32]);

        let guest_input = GuestInput {
            witnesses: vec![first, second],
            taiko: TaikoManifest {
                proposal_id: 42,
                ..Default::default()
            },
        };

        let proof_carry_data = ProofCarryData {
            chain_id: 167000,
            ..Default::default()
        };

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            &proof_carry_data,
            |stateless_input, _runtime| Ok(stateless_input.block.header.hash_slow()),
        )
        .expect_err("expected parent-hash mismatch to fail");

        assert!(err.to_string().contains("must link to previous block hash"));
        assert_ne!(
            guest_input.witnesses[1].block.header.parent_hash,
            first_hash
        );
    }

    #[test]
    fn rejects_chain_id_mismatch_between_input_and_proof_carry_data() {
        let chain_spec = taiko_mainnet_chain_spec();

        let input = StatelessInput {
            chain_spec,
            ..Default::default()
        };

        let guest_input = GuestInput {
            witnesses: vec![input],
            taiko: TaikoManifest {
                proposal_id: 1,
                ..Default::default()
            },
        };

        let proof_carry_data = ProofCarryData {
            chain_id: 167001,
            ..Default::default()
        };

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            &proof_carry_data,
            |_stateless_input, _runtime| Ok(B256::ZERO),
        )
        .expect_err("expected chain_id mismatch to fail");

        assert!(err
            .to_string()
            .contains("proof_carry_data.chain_id mismatch"));
    }

    #[test]
    fn rejects_non_taiko_chain_spec() {
        let chain_spec = ChainSpec::new_single(
            "ethereum".to_string(),
            1,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            false,
        );

        let input = StatelessInput {
            chain_spec,
            ..Default::default()
        };

        let guest_input = GuestInput {
            witnesses: vec![input],
            ..Default::default()
        };

        let proof_carry_data = ProofCarryData {
            chain_id: 1,
            ..Default::default()
        };

        assert!(prove_shasta_proposal_with_validator(
            &guest_input,
            &proof_carry_data,
            |_stateless_input, _runtime| Ok(B256::ZERO),
        )
        .is_err());
    }

    #[test]
    fn aggregate_shasta_zk_computes_expected_public_input() {
        let proof_carry_data = ProofCarryData {
            chain_id: 167000,
            verifier: Address::from([0x11; 20]),
            transition_input: TransitionInputData {
                actual_prover: Address::from([0x22; 20]),
                transition: raiko2_protocol::ShastaTransitionInput {
                    proposer: Address::from([0x33; 20]),
                    designatedProver: Address::ZERO,
                    timestamp: 123,
                },
                ..Default::default()
            },
        };

        let expected_block_input = hash_shasta_subproof_input(&proof_carry_data);

        let input = ShastaZkAggregationGuestInput {
            image_id: [1u32; 8],
            block_inputs: vec![expected_block_input],
            proof_carry_data_vec: vec![proof_carry_data.clone()],
            prover_address: Address::from([0x44; 20]),
        };

        let image_id_b256 = B256::from([0xAA; 32]);
        let mut calls = 0usize;

        let out = aggregate_shasta_zk_with_verifier(&input, image_id_b256, |_i, _block_input| {
            calls += 1;
            Ok(())
        })
        .expect("aggregation should succeed");

        assert_eq!(calls, 1);

        let commitment =
            build_shasta_commitment_from_proof_carry_data_vec(&input.proof_carry_data_vec)
                .expect("commitment should build");
        let first = input.proof_carry_data_vec.first().expect("checked");
        let aggregation_hash = shasta_aggregation_output(
            &commitment,
            first.chain_id,
            first.verifier,
            input.prover_address,
        );
        let expected = shasta_zk_aggregation_output(image_id_b256, aggregation_hash);

        assert_eq!(out, expected);
    }
}
