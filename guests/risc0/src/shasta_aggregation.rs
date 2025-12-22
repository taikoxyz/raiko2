//! Aggregates Shasta proposal proofs on RISC0
#![no_main]
#![allow(missing_docs)]
risc0_zkvm::guest::entry!(main);

use alloy_primitives::B256;
use raiko2_guest_common::aggregate_shasta_zk_with_verifier;
use raiko2_primitives::{
    instance::words_to_bytes_le, ShastaZkAggregationGuestInput,
};
use risc0_zkvm::guest::env;

pub fn main() {
    // Read aggregation input prepared by the host
    let input: ShastaZkAggregationGuestInput = env::read();

    let image_id_bytes = words_to_bytes_le(&input.image_id);
    let image_id_b256 = B256::from(image_id_bytes);

    let agg_public_input_hash =
        aggregate_shasta_zk_with_verifier(&input, image_id_b256, |_i, block_input| {
            env::verify(image_id_bytes, block_input.as_slice()).expect("proof verification failed");
            Ok(())
        })
        .expect("aggregation failed");

    env::commit_slice(agg_public_input_hash.as_slice());
}
