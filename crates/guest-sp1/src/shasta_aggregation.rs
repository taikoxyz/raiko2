//! Aggregates Shasta proposal proofs on SP1
#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::B256;
use raiko2_primitives::{
    guest::aggregate_shasta_zk_with_verifier, instance::words_to_bytes_be,
    ShastaZkAggregationGuestInput,
};
use sha2::{Digest, Sha256};

pub fn main() {
    // Read aggregation input prepared by the host
    let input = sp1_zkvm::io::read::<ShastaZkAggregationGuestInput>();

    let image_id_b256 = B256::from(words_to_bytes_be(&input.image_id));

    let agg_public_input_hash =
        aggregate_shasta_zk_with_verifier(&input, image_id_b256, |_i, block_input| {
            sp1_zkvm::lib::verify::verify_sp1_proof(
                &input.image_id,
                &Sha256::digest(block_input.as_slice()).into(),
            );
            Ok(())
        })
        .expect("aggregation failed");

    sp1_zkvm::io::commit_slice(agg_public_input_hash.as_slice());
}
