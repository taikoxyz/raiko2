//! Owned consensus transport retains original native bytes without embedding caller authority.
use alloy_primitives::{B256, hex};
use etna_core::{comet::codec::decode_canonical_block, fallback::FallbackKeys, key::ConsensusKey};
use raiko2_protocol_etna::input::{ConsensusInput, L1CommitteeInput, ParentConsensusInput};

fn input() -> ConsensusInput<Vec<u8>> {
    let key = ConsensusKey::from_slice(&hex!(
        "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    ))
    .unwrap();
    let (_, block) = decode_canonical_block(
        &hex::decode(include_str!("fixtures/first-block.hex").trim()).unwrap(),
    )
    .unwrap();
    let fallback = FallbackKeys::new(vec![key]).unwrap();
    ConsensusInput {
        original_blocks: vec![vec![0, 128, 255]],
        required_commits: vec![vec![255, 128, 0]],
        origin_fallback_keys: fallback.clone(),
        reference_state: vec![4, 5, 6],
        anchors: vec![L1CommitteeInput {
            anchor_number: 7,
            witness: vec![8, 9, 10],
            fallback_keys: fallback,
        }],
        parent: Some(ParentConsensusInput {
            header: block.header,
            active: vec![(key, 11)],
            next: vec![(key, 12)],
            anchor_header: vec![13, 14, 15],
        }),
    }
}

#[test]
fn generic_consensus_carrier_roundtrips_existing_witness_and_native_metadata() {
    let original = input();
    let encoded = serde_json::to_vec(&original).unwrap();
    let decoded = serde_json::from_slice::<ConsensusInput<Vec<u8>>>(&encoded).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), encoded);
    assert_eq!(decoded.original_blocks, original.original_blocks);
    assert_eq!(decoded.required_commits, original.required_commits);
    assert_eq!(decoded.reference_state, original.reference_state);
    assert_eq!(decoded.anchors[0].witness, original.anchors[0].witness);
    let parent = decoded.parent.unwrap();
    assert_eq!(parent.active[0].1, 11);
    assert_eq!(parent.next[0].1, 12);
    assert_eq!(parent.anchor_header, vec![13, 14, 15]);
}

#[test]
fn new_consensus_transport_rejects_extra_fields_and_malformed_consensus_keys() {
    for location in ["consensus", "anchor", "parent"] {
        let mut encoded = serde_json::to_value(input()).unwrap();
        let target = match location {
            "consensus" => &mut encoded,
            "anchor" => &mut encoded["anchors"][0],
            "parent" => &mut encoded["parent"],
            _ => unreachable!(),
        };
        target["host_verified"] = true.into();
        assert!(serde_json::from_value::<ConsensusInput<Vec<u8>>>(encoded).is_err());
    }
    let mut encoded = serde_json::to_value(input()).unwrap();
    encoded["parent"]["active"][0][0] = serde_json::to_value(B256::ZERO).unwrap();
    assert!(serde_json::from_value::<ConsensusInput<Vec<u8>>>(encoded).is_err());
}
