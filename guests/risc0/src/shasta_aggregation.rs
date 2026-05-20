//! Aggregates Shasta proposal receipts on RISC0.
#![no_main]
#![allow(missing_docs)]
risc0_zkvm::guest::entry!(main);

extern crate alloc;

use alloc::vec;

use alloy_primitives::B256;
use raiko2_guest_common::aggregate_shasta_zk_with_verifier;
use raiko2_primitives_shasta::{
    ShastaRisc0AggregationGuestInput, ShastaZkAggregationGuestInput,
    instance::words_to_bytes_le,
};
use risc0_zkvm::{Digest, InnerReceipt, Receipt, VerifierContext, guest::env};

fn verify_receipt(receipt: &Receipt, image_id: Digest) {
    let result = if matches!(&receipt.inner, InnerReceipt::Fake(_)) {
        receipt.verify_with_context(&VerifierContext::default().with_dev_mode(true), image_id)
    } else {
        receipt.verify(image_id)
    };
    result.expect("receipt verification failed");
}

pub fn main() {
    let mut len = 0u32;
    env::read_slice(core::slice::from_mut(&mut len));

    let mut input_buf = vec![0u8; len as usize];
    env::read_slice(&mut input_buf);
    let input: ShastaRisc0AggregationGuestInput =
        bincode::deserialize(&input_buf).expect("failed to deserialize RISC0 aggregation input");

    assert_eq!(
        input.receipts.len(),
        input.proof_carry_data_vec.len(),
        "receipts/proof_carry_data_vec length mismatch"
    );

    let image_id_bytes = words_to_bytes_le(&input.image_id);
    let image_id_b256 = B256::from(image_id_bytes);
    let image_id =
        Digest::try_from(image_id_bytes.as_slice()).expect("invalid aggregation image id");

    let block_inputs = input
        .receipts
        .iter()
        .map(|encoded_receipt| {
            let receipt: Receipt =
                bincode::deserialize(encoded_receipt).expect("failed to deserialize receipt");
            verify_receipt(&receipt, image_id);
            assert_eq!(
                receipt.journal.bytes.len(),
                32,
                "proposal receipt journal must be exactly 32 bytes"
            );
            B256::from_slice(&receipt.journal.bytes)
        })
        .collect::<Vec<_>>();

    let zk_input = ShastaZkAggregationGuestInput {
        image_id: input.image_id,
        block_inputs,
        proof_carry_data_vec: input.proof_carry_data_vec,
        prover_address: input.prover_address,
    };
    let agg_public_input_hash =
        aggregate_shasta_zk_with_verifier(&zk_input, image_id_b256, |_i, _block_input| Ok(()))
            .expect("aggregation failed");

    env::commit_slice(agg_public_input_hash.as_slice());
}
