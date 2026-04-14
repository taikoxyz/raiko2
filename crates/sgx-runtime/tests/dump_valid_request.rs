//! SGX request fixture helpers.

use alloy_primitives::B256;
use raiko2_primitives::{ChainSpec, ExecutionWitness, StatelessInput};
use raiko2_protocol_shasta::shasta::{Checkpoint, ProofCarryData};
use raiko2_prover::gaiko2::protocol::{
    GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ShastaPayload, Gaiko2ShastaRequest,
};
use reth_ethereum_primitives::Block;

const FIXTURE_PATH: &str =
    "../../tests/fixtures/shasta_remote_request_fixture_chain_167013_block_42.json";

fn u48(value: u64) -> alloy_primitives::Uint<48, 1> {
    alloy_primitives::Uint::from_limbs([value])
}

fn request_fixture() -> Gaiko2ShastaRequest {
    let mut carry = ProofCarryData {
        chain_id: 167_013,
        ..ProofCarryData::default()
    };
    carry.transition_input.parent_block_hash = B256::from([0x11; 32]);
    carry.transition_input.checkpoint = Checkpoint {
        blockNumber: u48(42),
        blockHash: B256::from([0x22; 32]),
        stateRoot: B256::from([0x33; 32]),
    };

    let mut stateless = StatelessInput {
        block: Block::default(),
        chain_spec: ChainSpec::default(),
        witness: ExecutionWitness::default(),
        accounts: Default::default(),
    };
    stateless.block.header.number = 42;
    stateless.block.header.parent_hash = B256::from([0x11; 32]);
    stateless.block.header.state_root = B256::from([0x33; 32]);
    stateless.chain_spec.chain_id = 167_013;

    Gaiko2ShastaRequest {
        schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Gaiko2ShastaPayload {
            chain_id: 167_013,
            blocks: vec![stateless.into()],
            proof_carry_data: carry,
        },
    }
}

#[test]
fn dump_valid_request_json() {
    let request = request_fixture();

    println!(
        "{}",
        serde_json::to_string_pretty(&request).expect("serialize request")
    );
}

#[test]
fn checked_in_request_fixture_matches_generated_shape() {
    let fixture = std::fs::read_to_string(FIXTURE_PATH).expect("read fixture");
    let fixture_value: serde_json::Value =
        serde_json::from_str(&fixture).expect("decode checked-in fixture");
    let generated_value =
        serde_json::to_value(request_fixture()).expect("encode generated fixture");

    assert_eq!(fixture_value, generated_value);
}
