//! Key-bound full-leaf proof aggregation. Requires genuine build-time identities.
#![no_main]

mod sys;

use alloy_primitives::B256;
use anyhow::ensure;
use bincode::Options;
use raiko2_primitives_shasta::{
    zisk::{digest_from_proof, program_id, ZiskAggregationInput},
    ShastaZkAggregationGuestInput,
};

include!(concat!(env!("OUT_DIR"), "/zisk_keys.rs"));
ziskos::entrypoint!(main);

fn main() {
    let bytes = ziskos::io::read_slice();
    let input: ZiskAggregationInput = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(bytes.as_ref())
        .expect("invalid bincode 1.3 aggregation payload");
    assert_eq!(
        input.proofs.len(),
        input.proof_carry_data_vec.len(),
        "proof/carry length mismatch"
    );
    let block_inputs = input
        .proofs
        .iter()
        .map(|proof| digest_from_proof(proof, &PROPOSAL_VK).map(B256::from))
        .collect::<anyhow::Result<Vec<_>>>()
        .expect("invalid child proof public prefix");
    let shared = ShastaZkAggregationGuestInput {
        // This shared helper does not read image_id; its explicit B256 argument
        // below is the authoritative ZisK identity, without SP1 conversion.
        image_id: [0; 8],
        block_inputs,
        proof_carry_data_vec: input.proof_carry_data_vec,
        prover_address: input.prover_address,
    };
    let hash = raiko2_guest_common::aggregate_shasta_zk_with_verifier(
        &shared,
        program_id(&PROPOSAL_VK),
        |index, expected| {
            let proof = &input.proofs[index];
            ensure!(
                ziskos::zisklib::verify_zisk_proof(proof, &SETUP_VK, &PROPOSAL_VK),
                "invalid ZisK child proof"
            );
            ensure!(
                digest_from_proof(proof, &PROPOSAL_VK)? == expected.0,
                "verified public digest mismatch"
            );
            Ok(())
        },
    )
    .expect("key-bound Shasta aggregation failed");
    ziskos::io::commit_slice(hash.as_slice());
}
