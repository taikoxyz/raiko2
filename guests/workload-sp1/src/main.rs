//! Offline workload experiment; this is not the production proposal program.
#![no_main]
sp1_zkvm::entrypoint!(main);

// Reuse the production crypto/syscall implementations without duplicating them.
#[path = "../../sp1/src/crypto.rs"]
mod crypto;
#[path = "../../sp1/src/sys.rs"]
mod sys;

use raiko2_guest_common::prove_shasta_proposal_with_reconstructor;
use raiko2_primitives_shasta::GuestInput;
use raiko2_stateless::{
    reconstruct_block_from_transactions_with_executor_and_witness_resources,
    workload_lab::{ExecutionProfile, PlainDerivedBlockExecutor},
};

pub fn main() {
    crypto::install_guest_crypto();
    let input = sp1_zkvm::io::read::<GuestInput>();
    let public_output = prove_shasta_proposal_with_reconstructor(&input, |request| {
        let mut executor = PlainDerivedBlockExecutor::new(ExecutionProfile::EvmGasOnly);
        reconstruct_block_from_transactions_with_executor_and_witness_resources(
            request.anchor_tx,
            request.transactions,
            request.block_env,
            request.witness,
            request.ancestor_headers,
            request.shared_state_nodes,
            request.chain_spec,
            request.evm_config,
            &mut executor,
        )
        .map_err(Into::into)
    })
    .expect("workload proposal reconstruction failed");
    sp1_zkvm::io::commit_slice(public_output.as_slice());
}
