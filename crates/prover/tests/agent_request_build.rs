#![allow(missing_docs)]

use raiko2_prover::agent::types::AsyncProofRequestData;

#[test]
fn builds_agent_request() {
    let req = AsyncProofRequestData::new("boundless", "batch", vec![1, 2], vec![3, 4]);
    assert_eq!(req.prover_type, "boundless");
    assert_eq!(req.proof_type, "batch");
}
