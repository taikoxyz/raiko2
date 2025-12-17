//! Aggregates Shasta proposal proofs on RISC0
#![no_main]
risc0_zkvm::guest::entry!(main);

use alloy_primitives::B256;
use raiko2_primitives::{
    instance::{
        build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
        shasta_zk_aggregation_output,
    },
    ShastaZkAggregationGuestInput,
};
use raiko2_protocol::hash_shasta_subproof_input;
use risc0_zkvm::guest::env;

pub fn main() {
    // Read aggregation input prepared by the host
    let input: ShastaZkAggregationGuestInput = env::read();

    assert_eq!(input.block_inputs.len(), input.proof_carry_data_vec.len());

    // Re-verify every underlying RISC0 proof against the provided image id.
    // Convert image_id from [u32; 8] to [u8; 32] for RISC0's verify
    let image_id_bytes: [u8; 32] = {
        let mut bytes = [0u8; 32];
        for (i, word) in input.image_id.iter().enumerate() {
            bytes[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    };

    for (i, block_input) in input.block_inputs.iter().enumerate() {
        // Verify the underlying proof using RISC0's assumption verification
        env::verify(image_id_bytes, block_input.as_slice()).expect("proof verification failed");

        // Verify the block_input matches the expected subproof input hash
        assert_eq!(
            *block_input,
            hash_shasta_subproof_input(&input.proof_carry_data_vec[i]),
            "block_input mismatch at index {i}"
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

    // Convert image_id to B256 for the final hash
    let image_id_b256 = B256::from(image_id_bytes);
    let agg_public_input_hash = shasta_zk_aggregation_output(image_id_b256, aggregation_hash);

    env::commit_slice(agg_public_input_hash.as_slice());
}
