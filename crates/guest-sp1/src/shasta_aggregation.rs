//! Aggregates Shasta proposal proofs on SP1
#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::B256;
use raiko2_primitives::{
    instance::{
        build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
        shasta_zk_aggregation_output, words_to_bytes_be,
    },
    ShastaZkAggregationGuestInput,
};
use raiko2_protocol::hash_shasta_subproof_input;
use sha2::{Digest, Sha256};

pub fn main() {
    // Read aggregation input prepared by the host
    let input = sp1_zkvm::io::read::<ShastaZkAggregationGuestInput>();

    assert_eq!(input.block_inputs.len(), input.proof_carry_data_vec.len());

    // Re-verify every underlying SP1 proof against the provided image id.
    for (i, block_input) in input.block_inputs.iter().enumerate() {
        sp1_zkvm::lib::verify::verify_sp1_proof(
            &input.image_id,
            &Sha256::digest(block_input.as_slice()).into(),
        );
        assert_eq!(
            *block_input,
            hash_shasta_subproof_input(&input.proof_carry_data_vec[i])
        );
    }

    // Compute the aggregation hash expected by the Shasta verifier.
    let commitment =
        build_shasta_commitment_from_proof_carry_data_vec(&input.proof_carry_data_vec).unwrap();
    let first = input.proof_carry_data_vec.first().unwrap();
    let aggregation_hash = shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        input.prover_address,
    );

    let agg_public_input_hash = shasta_zk_aggregation_output(
        B256::from(words_to_bytes_be(&input.image_id)),
        aggregation_hash,
    );

    sp1_zkvm::io::commit_slice(agg_public_input_hash.as_slice());
}
