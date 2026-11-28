use super::*;
use alloy_primitives::{hex, keccak256};
use serde_json::Value;

fn root(v: &Value) -> B256 {
    v.as_str().unwrap().parse().unwrap()
}

fn header(v: &Value) -> BeaconHeader {
    BeaconHeader {
        slot: v["slot"].as_str().unwrap().parse().unwrap(),
        proposer_index: v["proposer_index"].as_str().unwrap().parse().unwrap(),
        parent_root: root(&v["parent_root"]),
        state_root: root(&v["state_root"]),
        body_root: root(&v["body_root"]),
    }
}

fn nodes(values: &Value) -> Vec<SszProofNode> {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|v| SszProofNode {
            index: v[0].as_u64().unwrap(),
            root: root(&v[1]),
        })
        .collect()
}

fn with_mainnet(f: impl FnOnce(u64, B256, L1HistoryProof<'_>)) {
    let json: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/l1-history/mainnet.json")).unwrap();
    let q = hex::decode(json["q_raw"].as_str().unwrap()).unwrap();
    let targets = json["targets"].as_array().unwrap();
    let raw: Vec<_> = targets
        .iter()
        .map(|t| hex::decode(t["raw"].as_str().unwrap()).unwrap())
        .collect();
    let siblings: Vec<Vec<_>> = targets
        .iter()
        .map(|t| {
            t["body_siblings"]
                .as_array()
                .unwrap()
                .iter()
                .map(root)
                .collect()
        })
        .collect();
    let mut anchors: Vec<_> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            assert_eq!(keccak256(&raw[i]), root(&t["hash"]));
            AnchorHeaderWitness {
                raw_header: &raw[i],
                beacon_header: Some(header(&t["header"])),
                payload_siblings: &siblings[i],
            }
        })
        .collect();
    anchors.push(AnchorHeaderWitness {
        raw_header: &q,
        beacon_header: None,
        payload_siblings: &[],
    });
    let period = &json["period"];
    let group_helpers = nodes(&period["helpers"]);
    let periods = [HistoricalPeriodWitness {
        period: period["period"].as_u64().unwrap(),
        block_roots: root(&period["block_roots"]),
        legacy_state_roots: None,
        helpers: &group_helpers,
    }];
    let state_helpers = nodes(&json["state_helpers"]);
    f(
        json["q_number"].as_u64().unwrap(),
        root(&json["q_hash"]),
        L1HistoryProof {
            reference_header: &q,
            anchors: &anchors,
            limits: L1HistoryLimits {
                max_headers: 16,
                max_proof_nodes: 256,
            },
            beacon: Some(BeaconHistoryWitness {
                reference: header(&json["reference"]),
                state_helpers: &state_helpers,
                summary_length: json["summary_length"].as_u64(),
                legacy_length: None,
                periods: &periods,
            }),
        },
    );
}

#[test]
fn two_public_sparse_openings_and_direct_q_share_one_reference() {
    with_mainnet(|q, hash, proof| {
        let got = verify_l1_history(U256::from(1), q, hash, &proof).unwrap();
        assert_eq!(
            got.iter().map(L1HeaderView::number).collect::<Vec<_>>(),
            [25_198_983, 26_294_192, q]
        );
        assert_eq!(got[2].raw(), proof.reference_header);
    });
}

#[test]
fn exact_q_raw_number_and_chain_are_required() {
    with_mainnet(|q, hash, proof| {
        for (id, number, expected) in [
            (1, q - 1, hash),
            (1, q, B256::ZERO),
            (7, q, hash),
            (11_155_111, q, hash),
        ] {
            assert!(verify_l1_history(U256::from(id), number, expected, &proof).is_err());
        }
        let direct = L1HistoryProof {
            anchors: &proof.anchors[2..],
            beacon: None,
            ..proof
        };
        assert!(verify_l1_history(U256::from(1), q, hash, &direct).is_ok());
        assert!(
            verify_l1_history(
                U256::from(1),
                q,
                hash,
                &L1HistoryProof {
                    reference_header: proof.anchors[0].raw_header,
                    ..direct
                }
            )
            .is_err()
        );
    });
}

#[test]
fn detached_beacon_history_and_wrong_slot_are_rejected() {
    with_mainnet(|q, hash, proof| {
        for mutate_slot in [false, true] {
            let mut beacon = proof.beacon.unwrap();
            if mutate_slot {
                beacon.reference.slot -= 1;
            } else {
                beacon.reference.state_root = B256::ZERO;
            }
            assert!(
                verify_l1_history(
                    U256::from(1),
                    q,
                    hash,
                    &L1HistoryProof {
                        beacon: Some(beacon),
                        ..proof
                    }
                )
                .is_err()
            );
        }
        let mut anchors = proof.anchors.to_vec();
        anchors[1].beacon_header.as_mut().unwrap().slot -= 1;
        assert!(
            verify_l1_history(
                U256::from(1),
                q,
                hash,
                &L1HistoryProof {
                    anchors: &anchors,
                    ..proof
                }
            )
            .is_err()
        );
    });
}

#[test]
fn malformed_helpers_cannot_shadow_or_extend_the_proof() {
    with_mainnet(|q, hash, proof| {
        let mut beacon = proof.beacon.unwrap();
        for extra in [0, 1, 66, 269, 538, beacon.state_helpers[0].index] {
            let mut helpers = beacon.state_helpers.to_vec();
            helpers.push(SszProofNode {
                index: extra,
                root: B256::ZERO,
            });
            beacon.state_helpers = &helpers;
            assert!(
                verify_l1_history(
                    U256::from(1),
                    q,
                    hash,
                    &L1HistoryProof {
                        beacon: Some(beacon),
                        ..proof
                    }
                )
                .is_err()
            );
            beacon = proof.beacon.unwrap();
        }
        beacon.state_helpers = &beacon.state_helpers[1..];
        assert!(
            verify_l1_history(
                U256::from(1),
                q,
                hash,
                &L1HistoryProof {
                    beacon: Some(beacon),
                    ..proof
                }
            )
            .is_err()
        );
    });
}

#[test]
fn aggregate_budgets_precede_rlp_decoding() {
    with_mainnet(|q, hash, proof| {
        for limits in [
            L1HistoryLimits {
                max_headers: 1,
                max_proof_nodes: 256,
            },
            L1HistoryLimits {
                max_headers: 16,
                max_proof_nodes: 2,
            },
        ] {
            assert!(matches!(
                verify_l1_history(
                    U256::from(1),
                    q,
                    hash,
                    &L1HistoryProof {
                        reference_header: &[0xff],
                        limits,
                        ..proof
                    }
                ),
                Err(L1HistoryError::Budget)
            ));
        }
    });
}
#[test]
fn a_supplied_period_length_or_root_cannot_substitute_for_state_membership() {
    with_mainnet(|q, hash, proof| {
        let original = proof.beacon.unwrap();
        for length in [None, Some(0), Some(1137), Some(1139), Some(u64::MAX)] {
            let beacon = BeaconHistoryWitness {
                summary_length: length,
                ..original
            };
            assert!(
                verify_l1_history(
                    U256::from(1),
                    q,
                    hash,
                    &L1HistoryProof {
                        beacon: Some(beacon),
                        ..proof
                    }
                )
                .is_err()
            );
        }
        let mut group = original.periods[0];
        group.block_roots = B256::ZERO;
        let groups = [group];
        let beacon = BeaconHistoryWitness {
            periods: &groups,
            ..original
        };
        assert!(
            verify_l1_history(
                U256::from(1),
                q,
                hash,
                &L1HistoryProof {
                    beacon: Some(beacon),
                    ..proof
                }
            )
            .is_err()
        );
    });
}

// Independent fixed-path sparse tree for synthetic boundary cases. A subtree
// outside the input paths has a chosen zero commitment. Helpers are emitted
// at their first divergence from the supplied paths.
fn synthetic_tree(leaves: &[(u64, B256)]) -> (B256, Vec<SszProofNode>) {
    fn visit(index: u64, leaves: &[(u64, B256)], helpers: &mut Vec<SszProofNode>) -> B256 {
        if let Some((_, root)) = leaves.iter().find(|(leaf, _)| *leaf == index) {
            return *root;
        }
        if !leaves.iter().any(|(leaf, _)| {
            leaf.ilog2() > index.ilog2() && leaf >> (leaf.ilog2() - index.ilog2()) == index
        }) {
            helpers.push(SszProofNode {
                index,
                root: B256::ZERO,
            });
            return B256::ZERO;
        }
        pair(
            visit(index * 2, leaves, helpers),
            visit(index * 2 + 1, leaves, helpers),
        )
    }
    let mut helpers = Vec::new();
    let root = visit(1, leaves, &mut helpers);
    helpers.sort_by_key(|h| h.index);
    (root, helpers)
}

#[derive(Clone, Copy)]
enum Position {
    Recent,
    Summary,
    Legacy,
}

fn synthetic_history(
    slot: u64,
    reference_slot: u64,
    position: Position,
    state_slot: u64,
    version: u8,
    max_proof_nodes: usize,
) -> Result<(), L1HistoryError> {
    use alloy_consensus::Header;
    let target_raw = alloy_rlp::encode(Header {
        number: 17,
        timestamp: 1_606_824_023 + slot * 12,
        ..Header::default()
    });
    let payload_gindex: u64 = if slot < 269_568 * 32 { 412 } else { 812 };
    let body_root = synthetic_tree(&[(payload_gindex, keccak256(&target_raw))]).0;
    let siblings = [B256::ZERO; 9];
    let target = BeaconHeader {
        slot,
        proposer_index: 0,
        parent_root: B256::ZERO,
        state_root: B256::ZERO,
        body_root,
    };
    let anchor = AnchorHeaderWitness {
        raw_header: &target_raw,
        beacon_header: Some(target),
        payload_siblings: &siblings[..payload_gindex.ilog2() as usize],
    };
    let period = slot / 8192;
    let offset = slot % 8192;
    let mut fork = B256::ZERO;
    fork[0] = version;
    let mut state = alloc::vec![(66, scalar(state_slot)), (269, fork)];
    let mut groups = Vec::new();
    let mut summary_length = None;
    let mut legacy_length = None;
    let (vector_root, vector_helpers) = synthetic_tree(&[(8192 + offset, header_root(&target))]);
    match position {
        Position::Recent => state.push((69 * 8192 + offset, header_root(&target))),
        Position::Legacy | Position::Summary => {
            let summaries = reference_slot / 8192 - 758;
            summary_length = Some(summaries);
            state.push((183, scalar(summaries)));
            let legacy = matches!(position, Position::Legacy);
            let state_roots = B256::repeat_byte(0x33);
            if legacy {
                legacy_length = Some(758);
                state.push((143, scalar(758)));
                state.push((142 * (1 << 24) + period, pair(vector_root, state_roots)));
            } else {
                state.push(((182 * (1 << 24) + (period - 758)) * 2, vector_root));
            }
            groups.push(HistoricalPeriodWitness {
                period,
                block_roots: vector_root,
                legacy_state_roots: legacy.then_some(state_roots),
                helpers: &vector_helpers,
            });
        }
    }
    let (state_root, state_helpers) = synthetic_tree(&state);
    let reference = BeaconHeader {
        slot: reference_slot,
        state_root,
        ..target
    };
    let q = alloy_rlp::encode(Header {
        number: 101,
        timestamp: 1_606_824_023 + (reference_slot + 3) * 12,
        base_fee_per_gas: Some(1),
        withdrawals_root: Some(B256::ZERO),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(header_root(&reference)),
        ..Header::default()
    });
    let proof = L1HistoryProof {
        reference_header: &q,
        anchors: core::slice::from_ref(&anchor),
        limits: L1HistoryLimits {
            max_headers: 2,
            max_proof_nodes,
        },
        beacon: Some(BeaconHistoryWitness {
            reference,
            state_helpers: &state_helpers,
            summary_length,
            legacy_length,
            periods: &groups,
        }),
    };
    verify_l1_history(U256::from(1), 101, keccak256(&q), &proof).map(|_| ())
}

#[test]
fn recent_window_is_inclusive_and_completed_history_crosses_capella() {
    let reference = 1650 * 8192;
    for (slot, position) in [
        (reference - 8192, Position::Recent),
        (reference - 8193, Position::Summary),
        (758 * 8192 - 1, Position::Legacy),
        (758 * 8192, Position::Summary),
    ] {
        synthetic_history(slot, reference, position, reference, 6, 128).unwrap();
    }
    assert!(
        synthetic_history(
            reference - 8193,
            reference,
            Position::Recent,
            reference,
            6,
            128
        )
        .is_err()
    );
    assert!(
        synthetic_history(
            reference - 8192,
            reference,
            Position::Summary,
            reference,
            6,
            128
        )
        .is_err()
    );
}

#[test]
fn legacy_batch_state_roots_sibling_consumes_one_proof_node() {
    let reference = 1650 * 8192;
    let target = 758 * 8192 - 1;
    // Eight body siblings, thirteen period vector helpers, 36 shared state
    // helpers and the HistoricalBatch.state_roots sibling.
    let exact = 8 + 13 + 36 + 1;
    synthetic_history(target, reference, Position::Legacy, reference, 6, exact).unwrap();
    assert_eq!(
        synthetic_history(target, reference, Position::Legacy, reference, 6, exact - 1),
        Err(L1HistoryError::Budget)
    );
}

#[test]
fn state_slot_and_actual_fork_version_must_open_under_the_reference() {
    let reference = 1650 * 8192;
    assert!(
        synthetic_history(
            reference - 1,
            reference,
            Position::Recent,
            reference - 1,
            6,
            128
        )
        .is_err()
    );
    assert!(
        synthetic_history(
            reference - 1,
            reference,
            Position::Recent,
            reference,
            7,
            128
        )
        .is_err()
    );
}

#[test]
fn schedules_have_explicit_unsupported_and_simultaneous_fork_boundaries() {
    let hoodi = Network::from_id(U256::from(560_048)).unwrap();
    assert_eq!(hoodi.schema(0).unwrap().version, [0x50, 0, 9, 0x10]);
    let sepolia = Network::from_id(U256::from(11_155_111)).unwrap();
    assert_eq!(
        sepolia.schema(353_024 * 32 - 1).unwrap().version,
        [0x90, 0, 0, 0x75]
    );
    assert!(sepolia.schema(353_024 * 32).is_err());
    let mainnet = Network::from_id(U256::from(1)).unwrap();
    assert!(mainnet.schema(144_896 * 32 - 1).is_err());
    assert!(Network::from_id((U256::from(1) << 64) + U256::from(1)).is_err());
}

#[test]
fn owned_input_independent_replays_public_mainnet_sparse_history() {
    use crate::input::L1HistoryInput;
    with_mainnet(|q_number, q_hash, proof| {
        let original = verify_l1_history(U256::from(1), q_number, q_hash, &proof).unwrap();
        let expected: Vec<_> = original
            .iter()
            .map(|header| {
                (
                    header.number(),
                    header.hash(),
                    header.state_root(),
                    header.raw().to_vec(),
                )
            })
            .collect();
        let encoded = serde_json::to_vec(&L1HistoryInput::from(proof)).unwrap();
        let decoded: L1HistoryInput = serde_json::from_slice(&encoded).unwrap();
        let actual = decoded
            .with_proof(proof.limits, |borrowed| {
                verify_l1_history(U256::from(1), q_number, q_hash, &borrowed).map(|headers| {
                    headers
                        .iter()
                        .map(|header| {
                            (
                                header.number(),
                                header.hash(),
                                header.state_root(),
                                header.raw().to_vec(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap()
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), encoded);

        for mutation in [
            "state_root",
            "state_index",
            "period_root",
            "period_index",
            "summary_length",
            "raw_q",
        ] {
            let mut changed: L1HistoryInput = serde_json::from_slice(&encoded).unwrap();
            match mutation {
                "state_root" => changed.beacon.as_mut().unwrap().state_helpers[0].root[0] ^= 1,
                "state_index" => changed.beacon.as_mut().unwrap().state_helpers[0].index = 0,
                "period_root" => {
                    changed.beacon.as_mut().unwrap().periods[0].helpers[0].root[0] ^= 1;
                }
                "period_index" => changed.beacon.as_mut().unwrap().periods[0].helpers[0].index = 0,
                "summary_length" => changed.beacon.as_mut().unwrap().summary_length = None,
                "raw_q" => changed.reference_header.push(0),
                _ => unreachable!(),
            }
            let result = changed
                .with_proof(proof.limits, |borrowed| {
                    verify_l1_history(U256::from(1), q_number, q_hash, &borrowed).map(|_| ())
                })
                .unwrap();
            assert!(result.is_err(), "{mutation}");
        }

        for limits in [
            L1HistoryLimits {
                max_headers: 0,
                max_proof_nodes: usize::MAX,
            },
            L1HistoryLimits {
                max_headers: usize::MAX,
                max_proof_nodes: 0,
            },
        ] {
            assert!(matches!(
                decoded.with_proof(limits, |_| panic!("budget callback reached")),
                Err(L1HistoryError::Budget)
            ));
        }
    });
}

const INTERNAL_L1_CHAIN_ID: u64 = 32_382;

fn devnet_fixture() -> (u64, B256, crate::input::L1HistoryInput, Value) {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/l1-history/devnet-recent.json"
    ))
    .unwrap();
    assert_eq!(fixture["chain_id"], INTERNAL_L1_CHAIN_ID);
    (
        fixture["reference_number"].as_u64().unwrap(),
        root(&fixture["reference_hash"]),
        serde_json::from_value(fixture["input"].clone()).unwrap(),
        fixture,
    )
}

fn devnet_limits() -> L1HistoryLimits {
    L1HistoryLimits {
        max_headers: 3,
        max_proof_nodes: 50,
    }
}

fn verify_devnet(
    input: &crate::input::L1HistoryInput,
    number: u64,
    hash: B256,
) -> Result<Vec<(u64, B256, B256)>, L1HistoryError> {
    input.with_proof(devnet_limits(), |proof| {
        verify_l1_history(U256::from(INTERNAL_L1_CHAIN_ID), number, hash, &proof).map(|headers| {
            headers
                .iter()
                .map(|header| (header.number(), header.hash(), header.state_root()))
                .collect()
        })
    })?
}

fn replace_header_field(raw: &[u8], position: usize, replacement: &[u8]) -> Vec<u8> {
    let mut cursor = raw;
    let alloy_rlp::PayloadView::List(mut fields) =
        alloy_rlp::Header::decode_raw(&mut cursor).unwrap()
    else {
        panic!("expected raw header list")
    };
    assert!(cursor.is_empty());
    fields[position] = replacement;
    let mut encoded = Vec::new();
    alloy_rlp::Header {
        list: true,
        payload_length: fields.iter().map(|field| field.len()).sum(),
    }
    .encode(&mut encoded);
    for field in fields {
        encoded.extend_from_slice(field);
    }
    encoded
}

#[test]
fn actual_devnet_recent_anchors_share_one_owned_native_reference() {
    let (number, hash, input, fixture) = devnet_fixture();
    let bytes = serde_json::to_vec(&input).unwrap();
    let decoded: crate::input::L1HistoryInput = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
    assert_eq!(decoded.anchors.len(), 2);
    assert_eq!(decoded.beacon.as_ref().unwrap().state_helpers.len(), 32);
    assert!(decoded.beacon.as_ref().unwrap().periods.is_empty());
    assert_eq!(decoded.beacon.as_ref().unwrap().reference.slot, 89_935);
    let got = verify_devnet(&decoded, number, hash).unwrap();
    let expected = fixture["expected_anchors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|target| {
            (
                target["number"].as_u64().unwrap(),
                root(&target["hash"]),
                root(&target["state_root"]),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(got, expected);
    assert_eq!(
        got.iter().map(|header| header.0).collect::<Vec<_>>(),
        [53_231, 55_363]
    );
    assert_eq!(fixture["expected_anchors"][0]["age_seconds"], 86_400);
    assert_eq!(fixture["expected_anchors"][1]["age_seconds"], 14_400);
    assert!(
        decoded
            .beacon
            .as_ref()
            .unwrap()
            .state_helpers
            .windows(2)
            .all(|pair| pair[0].index < pair[1].index)
    );
}

#[test]
fn actual_devnet_q_identity_roots_slots_and_unknown_profiles_fail_closed() {
    let (number, hash, input, _) = devnet_fixture();
    assert_eq!(
        verify_devnet(&input, number - 1, hash).unwrap_err(),
        L1HistoryError::Identity
    );
    assert_eq!(
        verify_devnet(&input, number, B256::ZERO).unwrap_err(),
        L1HistoryError::Identity
    );
    for id in [
        U256::ZERO,
        U256::from(1),
        U256::from(11_155_111),
        U256::from(560_048),
        U256::from(INTERNAL_L1_CHAIN_ID + 1),
    ] {
        assert!(
            input
                .with_proof(devnet_limits(), |proof| verify_l1_history(
                    id, number, hash, &proof
                )
                .map(|_| ()))
                .unwrap()
                .is_err(),
            "{id}"
        );
    }
    let alias = U256::from(INTERNAL_L1_CHAIN_ID) | (U256::from(1) << 200usize);
    assert!(matches!(
        input
            .with_proof(devnet_limits(), |proof| verify_l1_history(
                alias, number, hash, &proof
            )
            .map(|_| ()))
            .unwrap(),
        Err(L1HistoryError::Schema)
    ));
    for mutation in [
        "raw_q",
        "reference_root",
        "q_parent_root",
        "q_slot",
        "target_slot",
        "target_state_root",
        "target_height",
        "substituted_anchor",
    ] {
        let mut changed = input.clone();
        let mut expected_hash = hash;
        match mutation {
            "raw_q" => changed.reference_header.push(0),
            "reference_root" => changed.beacon.as_mut().unwrap().reference.state_root = B256::ZERO,
            "q_parent_root" => {
                changed.reference_header = replace_header_field(
                    &changed.reference_header,
                    19,
                    &alloy_rlp::encode(B256::repeat_byte(0xaa)),
                );
                expected_hash = keccak256(&changed.reference_header);
            }
            "q_slot" => {
                let timestamp = L1HeaderView::from_raw(&changed.reference_header)
                    .unwrap()
                    .timestamp()
                    - 12;
                changed.reference_header = replace_header_field(
                    &changed.reference_header,
                    11,
                    &alloy_rlp::encode(timestamp),
                );
                expected_hash = keccak256(&changed.reference_header);
            }
            "target_slot" => changed.anchors[0].beacon_header.as_mut().unwrap().slot += 1,
            "target_state_root" => {
                changed.anchors[0].raw_header = replace_header_field(
                    &changed.anchors[0].raw_header,
                    3,
                    &alloy_rlp::encode(B256::repeat_byte(0xaa)),
                )
            }
            "target_height" => {
                changed.anchors[0].raw_header = replace_header_field(
                    &changed.anchors[0].raw_header,
                    8,
                    &alloy_rlp::encode(53_232u64),
                )
            }
            "substituted_anchor" => changed.anchors[0] = changed.anchors[1].clone(),
            _ => unreachable!(),
        }
        assert!(
            verify_devnet(&changed, number, expected_hash).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn actual_devnet_all_shared_and_payload_helpers_are_required_and_authenticated() {
    let (number, hash, input, _) = devnet_fixture();
    for index in 0..input.beacon.as_ref().unwrap().state_helpers.len() {
        let mut changed = input.clone();
        changed.beacon.as_mut().unwrap().state_helpers[index].root[0] ^= 1;
        assert!(
            verify_devnet(&changed, number, hash).is_err(),
            "shared node {index}"
        );
        let mut changed = input.clone();
        changed.beacon.as_mut().unwrap().state_helpers.remove(index);
        assert!(
            verify_devnet(&changed, number, hash).is_err(),
            "missing shared node {index}"
        );
    }
    for target in 0..input.anchors.len() {
        for index in 0..input.anchors[target].payload_siblings.len() {
            let mut changed = input.clone();
            changed.anchors[target].payload_siblings[index][0] ^= 1;
            assert!(
                verify_devnet(&changed, number, hash).is_err(),
                "payload node {target}/{index}"
            );
        }
    }
    let mut changed = input.clone();
    changed.beacon.as_mut().unwrap().state_helpers.swap(0, 1);
    assert!(verify_devnet(&changed, number, hash).is_err());
    let mut changed = input.clone();
    changed.beacon.as_mut().unwrap().state_helpers[0].index = 0;
    assert!(verify_devnet(&changed, number, hash).is_err());
    let mut changed = input.clone();
    changed.beacon.as_mut().unwrap().state_helpers.insert(
        0,
        SszProofNode {
            index: 1,
            root: B256::ZERO,
        },
    );
    let roomy = L1HistoryLimits {
        max_headers: 3,
        max_proof_nodes: 51,
    };
    assert!(
        changed
            .with_proof(roomy, |proof| verify_l1_history(
                U256::from(INTERNAL_L1_CHAIN_ID),
                number,
                hash,
                &proof
            )
            .map(|_| ()))
            .unwrap()
            .is_err()
    );
    assert!(matches!(
        input.with_proof(
            L1HistoryLimits {
                max_headers: 3,
                max_proof_nodes: 49
            },
            |_| panic!("budget callback reached")
        ),
        Err(L1HistoryError::Budget)
    ));
}

// Derive a minimal authenticated subset from the already valid shared multiproof. This is test
// code only: requested anchor coverage belongs to the guest composer, not this membership API.
fn devnet_subset(
    mut input: crate::input::L1HistoryInput,
    keep: usize,
) -> crate::input::L1HistoryInput {
    let beacon = input.beacon.as_ref().unwrap();
    let mut leaves = BTreeMap::new();
    leaves.insert(66, scalar(beacon.reference.slot));
    let mut version = B256::ZERO;
    version[..4].copy_from_slice(&[0x20, 0, 0, 0x95]);
    leaves.insert(269, version);
    for anchor in &input.anchors {
        let target = anchor.beacon_header.as_ref().unwrap();
        leaves.insert(69 * 8192 + target.slot % 8192, header_root(target));
    }
    let mut all = leaves.clone();
    all.extend(
        beacon
            .state_helpers
            .iter()
            .map(|node| (node.index, node.root)),
    );
    let mut pending = all.clone();
    while let Some((index, node)) = pending.pop_last() {
        if index == 1 {
            assert_eq!(node, beacon.reference.state_root);
            break;
        }
        let sibling = pending
            .remove(&(index ^ 1))
            .expect("valid shared proof sibling");
        let parent = if index & 1 == 0 {
            pair(node, sibling)
        } else {
            pair(sibling, node)
        };
        assert!(pending.insert(index / 2, parent).is_none());
        all.insert(index / 2, parent);
    }
    let selected = input.anchors[keep].clone();
    let removed = input.anchors[1 - keep].beacon_header.as_ref().unwrap();
    leaves.remove(&(69 * 8192 + removed.slot % 8192));
    let mut helpers = Vec::new();
    while let Some((index, node)) = leaves.pop_last() {
        if index == 1 {
            assert_eq!(node, beacon.reference.state_root);
            break;
        }
        let sibling = leaves.remove(&(index ^ 1)).unwrap_or_else(|| {
            let root = all[&(index ^ 1)];
            helpers.push(SszProofNode {
                index: index ^ 1,
                root,
            });
            root
        });
        let parent = if index & 1 == 0 {
            pair(node, sibling)
        } else {
            pair(sibling, node)
        };
        assert!(leaves.insert(index / 2, parent).is_none());
    }
    helpers.sort_by_key(|node| node.index);
    input.anchors = alloc::vec![selected];
    input.beacon.as_mut().unwrap().state_helpers = helpers;
    input
}

#[test]
fn actual_devnet_anchor_coverage_is_a_composer_obligation() {
    let (number, hash, input, _) = devnet_fixture();
    // Merely removing an anchor leaves a noncanonical/incomplete shared proof and must fail.
    let mut missing = input.clone();
    missing.anchors.remove(0);
    assert!(verify_devnet(&missing, number, hash).is_err());
    // A correctly rebuilt one-anchor witness authenticates that subset. This API has no
    // expected-anchor list. guest-common::etna::verify_l1_anchor_roots checks every projected
    // block consumes an opening and rejects unused/missing openings against the real EL range.
    for index in 0..2 {
        let subset = devnet_subset(input.clone(), index);
        let got = verify_devnet(&subset, number, hash).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, [53_231, 55_363][index]);
    }
    let mut empty = input.clone();
    empty.anchors.clear();
    assert!(verify_devnet(&empty, number, hash).is_err());
}

#[test]
fn internal_l1_schedule_is_compiled_for_the_captured_deployment() {
    let network = Network::from_id(U256::from(INTERNAL_L1_CHAIN_ID)).unwrap();
    assert_eq!(network.genesis, 1_794_749_112);
    assert_eq!(network.epochs, [0; 5]);
    assert_eq!(
        network.versions,
        [
            0x2000_0091,
            0x2000_0092,
            0x2000_0093,
            0x2000_0094,
            0x2000_0095
        ]
    );
    assert_eq!(network.slot(1_795_828_344).unwrap(), 89_936);
    assert!(network.slot(network.genesis - 1).is_err());
    assert!(network.slot(network.genesis + 1).is_err());
    let fulu = network.schema(0).unwrap();
    assert_eq!(fulu.state_base, 64);
    assert_eq!(fulu.payload_base, 32);
    assert_eq!(fulu.version, [0x20, 0, 0, 0x95]);
}
