//! Owned input preservation, caller-budget and authority-field regressions.

use alloy_primitives::{Address, B256, U256};
use raiko2_protocol_etna::{
    input::{
        AnchorHeaderInput, BeaconHistoryInput, HistoricalPeriodInput, L1HistoryInput,
        LandStatementInput,
    },
    l1_history::{BeaconHeader, L1HistoryError, L1HistoryLimits, SszProofNode},
    statement::LandInput,
};

fn statement() -> LandStatementInput {
    LandStatementInput {
        l1_chain_id: U256::from(1),
        l2_chain_id: U256::from(2),
        inbox: Address::repeat_byte(3),
        registry: Address::repeat_byte(4),
        generation: U256::from(5),
        parent_hash: B256::repeat_byte(6),
        parent_comet_hash: B256::repeat_byte(7),
        committee_config_hash: B256::repeat_byte(8),
        input: LandInput {
            parent_height: U256::from(9),
            last_height: U256::from(11),
            last_block_hash: B256::repeat_byte(10),
            last_comet_hash: B256::repeat_byte(11),
            last_state_root: B256::repeat_byte(12),
            l1_reference_block_number: U256::from(13),
            forced_inclusion_head: U256::from(14),
            active_committee_snapshot: U256::from(15),
        },
        l1_reference_block_hash: B256::repeat_byte(16),
        blob_hashes: vec![B256::repeat_byte(17)],
        forced_inclusion_head_before: U256::from(18),
        cut_leaves: vec![B256::repeat_byte(19)],
    }
}

fn history() -> L1HistoryInput {
    let header = BeaconHeader {
        slot: 1,
        proposer_index: 2,
        parent_root: B256::repeat_byte(3),
        state_root: B256::repeat_byte(4),
        body_root: B256::repeat_byte(5),
    };
    L1HistoryInput {
        reference_header: vec![0, 128, 255],
        anchors: vec![AnchorHeaderInput {
            raw_header: vec![255, 0, 1],
            beacon_header: Some(header),
            payload_siblings: vec![B256::repeat_byte(6)],
        }],
        beacon: Some(BeaconHistoryInput {
            reference: header,
            state_helpers: vec![SszProofNode {
                index: 7,
                root: B256::repeat_byte(8),
            }],
            summary_length: Some(9),
            legacy_length: Some(10),
            periods: vec![HistoricalPeriodInput {
                period: 11,
                block_roots: B256::repeat_byte(12),
                legacy_state_roots: Some(B256::repeat_byte(13)),
                helpers: vec![SszProofNode {
                    index: 14,
                    root: B256::repeat_byte(15),
                }],
            }],
        }),
    }
}

#[test]
fn owned_statement_roundtrip_preserves_complete_preimage() {
    let original = statement();
    let decoded: LandStatementInput =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(
        decoded.as_statement().preimage(),
        original.as_statement().preimage()
    );
    assert_eq!(
        decoded.as_statement().try_digest(),
        original.as_statement().try_digest()
    );
    let mut encoded = serde_json::to_value(original).unwrap();
    encoded["input"]["host_verified"] = true.into();
    assert!(serde_json::from_value::<LandStatementInput>(encoded).is_err());
}

#[test]
fn owned_ssz_roundtrip_preserves_all_material_and_metadata() {
    let original = history();
    let decoded: L1HistoryInput =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    let result = decoded.with_proof(
        L1HistoryLimits {
            max_headers: 2,
            max_proof_nodes: 4,
        },
        |proof| {
            assert_eq!(proof.reference_header, original.reference_header);
            assert_eq!(proof.anchors.len(), 1);
            assert_eq!(proof.anchors[0].raw_header, original.anchors[0].raw_header);
            assert_eq!(
                proof.anchors[0].payload_siblings,
                original.anchors[0].payload_siblings
            );
            let beacon = proof.beacon.unwrap();
            assert_eq!(
                beacon.reference,
                original.beacon.as_ref().unwrap().reference
            );
            assert_eq!(beacon.summary_length, Some(9));
            assert_eq!(beacon.legacy_length, Some(10));
            assert_eq!(beacon.state_helpers[0].index, 7);
            assert_eq!(beacon.state_helpers[0].root, B256::repeat_byte(8));
            assert_eq!(beacon.periods.len(), 1);
            assert_eq!(beacon.periods[0].period, 11);
            assert_eq!(beacon.periods[0].block_roots, B256::repeat_byte(12));
            assert_eq!(
                beacon.periods[0].legacy_state_roots,
                Some(B256::repeat_byte(13))
            );
            assert_eq!(beacon.periods[0].helpers[0].index, 14);
            assert_eq!(beacon.periods[0].helpers[0].root, B256::repeat_byte(15));
            42
        },
    );
    assert_eq!(result.unwrap(), 42);
    let mut no_metadata = history();
    let beacon = no_metadata.beacon.as_mut().unwrap();
    beacon.summary_length = None;
    beacon.legacy_length = None;
    beacon.periods[0].legacy_state_roots = None;
    let decoded: L1HistoryInput =
        serde_json::from_slice(&serde_json::to_vec(&no_metadata).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::to_value(no_metadata).unwrap()
    );
}

#[test]
fn json_cannot_supply_limits_or_extra_carrier_fields() {
    for field in ["limits", "max_headers", "max_proof_nodes", "host_verified"] {
        let mut encoded = serde_json::to_value(history()).unwrap();
        encoded[field] =
            serde_json::json!({ "max_headers": 1_000_000, "max_proof_nodes": 1_000_000 });
        assert!(
            serde_json::from_value::<L1HistoryInput>(encoded).is_err(),
            "{field}"
        );
    }
    for path in ["anchor", "beacon", "period", "node", "header"] {
        let mut encoded = serde_json::to_value(history()).unwrap();
        let target = match path {
            "anchor" => &mut encoded["anchors"][0],
            "beacon" => &mut encoded["beacon"],
            "period" => &mut encoded["beacon"]["periods"][0],
            "node" => &mut encoded["beacon"]["state_helpers"][0],
            "header" => &mut encoded["beacon"]["reference"],
            _ => unreachable!(),
        };
        target["unexpected"] = true.into();
        assert!(
            serde_json::from_value::<L1HistoryInput>(encoded).is_err(),
            "{path}"
        );
    }
    let mut encoded = serde_json::to_value(history()).unwrap();
    encoded["reference_header"][0] = 256.into();
    assert!(serde_json::from_value::<L1HistoryInput>(encoded).is_err());
}

#[test]
fn trusted_limits_reject_before_borrowed_view_callback() {
    for limits in [
        L1HistoryLimits {
            max_headers: 1,
            max_proof_nodes: 4,
        },
        L1HistoryLimits {
            max_headers: 2,
            max_proof_nodes: 3,
        },
    ] {
        let decoded: L1HistoryInput =
            serde_json::from_slice(&serde_json::to_vec(&history()).unwrap()).unwrap();
        assert!(matches!(
            decoded.with_proof(limits, |_| panic!("over-budget callback")),
            Err(L1HistoryError::Budget)
        ));
    }
}
