//! Aggregates Shasta proposal proofs on SP1
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

use raiko2_guest_common::aggregate_shasta_zk_with_verifier;
use raiko2_primitives_shasta::{
    instance::sp1_contract_block_program_id, ShastaZkAggregationGuestInput,
};
use sha2::{Digest, Sha256};

pub fn main() {
    // Read aggregation input prepared by the host
    let input = sp1_zkvm::io::read::<ShastaZkAggregationGuestInput>();

    let image_id_b256 = sp1_contract_block_program_id(&input.image_id);

    #[cfg(feature = "bench")]
    println!("cycle-tracker-start: shasta-aggregation");
    let agg_public_input_hash =
        aggregate_shasta_zk_with_verifier(&input, image_id_b256, |_i, block_input| {
            sp1_zkvm::lib::verify::verify_sp1_proof(
                &input.image_id,
                &Sha256::digest(block_input.as_slice()).into(),
            );
            Ok(())
        })
        .expect("aggregation failed");
    #[cfg(feature = "bench")]
    println!("cycle-tracker-end: shasta-aggregation");

    sp1_zkvm::io::commit_slice(agg_public_input_hash.as_slice());
}
