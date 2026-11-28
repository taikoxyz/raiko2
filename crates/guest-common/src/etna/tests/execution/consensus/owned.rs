//! Exercises serialized native consensus inputs through the actual state/QC composer.
use super::*;
use crate::etna::verify_range_input_consensus;
use raiko2_protocol_etna::input::{
    ConsensusInput, ConsensusRangeInput, L1CommitteeInput, ParentConsensusInput,
};

fn owned(
    fixture: &Fixture,
    start: usize,
) -> (
    ConsensusRangeInput<ExecutionWitness>,
    InputDataSource,
    RangeProjectionContext,
) {
    let authenticated = &fixture.execution.authenticated;
    let range = &authenticated.range;
    let mut input = fixture.execution.input();
    let mut context = range.context;
    let parent = if start == 0 {
        range.parent.clone()
    } else {
        range.blocks[start - 1].execution.clone()
    };
    context.parent_height = parent.number;
    context.parent_el_hash = parent.hash_slow();
    context.parent_comet_hash = if start == 0 {
        B256::ZERO
    } else {
        B256::from_slice(header_hash(&range.blocks[start - 1].comet.header).as_bytes())
    };
    input.blocks.drain(..start);
    input.post_block_cursors.drain(..start);
    input.statement.cut_leaves.drain(..start);
    input.parent_execution_header = parent;
    input.statement.parent_hash = context.parent_el_hash;
    input.statement.parent_comet_hash = context.parent_comet_hash;
    input.statement.input.parent_height = U256::from(context.parent_height);
    input.statement.config_hash = config_hash(
        167_001,
        fixture.configured_staking,
        parameters().max_uncheckpointed,
    );
    let items = range.blocks[start..]
        .iter()
        .map(|block| block.item.as_slice())
        .collect::<Vec<_>>();
    let (source, hashes) = prove_blobs(pack(&v6_payload(&list(&items), Compression::default())));
    input.statement.blob_hashes = hashes;
    let members = |validators: &Validators| {
        validators
            .validators()
            .iter()
            .map(|validator| (validator.pub_key, validator.power()))
            .collect()
    };
    let consensus = ConsensusInput {
        original_blocks: fixture.originals[start..].to_vec(),
        required_commits: fixture
            .commits
            .iter()
            .filter(|raw| {
                etna_core::comet::codec::decode_canonical_commit(raw)
                    .unwrap()
                    .height
                    .value()
                    > context.parent_height
            })
            .cloned()
            .collect(),
        origin_fallback_keys: fixture.fallback.clone(),
        reference_state: fixture.l1.clone(),
        anchors: vec![L1CommitteeInput {
            anchor_number: 18,
            witness: fixture.l1.clone(),
            fallback_keys: fixture.fallback.clone(),
        }],
        parent: (start != 0).then(|| ParentConsensusInput {
            header: range.blocks[start - 1].comet.header.clone(),
            active: members(&fixture.sets[start - 1]),
            next: members(&fixture.sets[start]),
            anchor_header: range.l1_headers[0].clone(),
        }),
    };
    (
        ConsensusRangeInput {
            range: input,
            consensus,
        },
        source,
        context,
    )
}

fn verify_owned(
    fixture: &Fixture,
    start: usize,
    input: &ConsensusRangeInput<ExecutionWitness>,
    source: &InputDataSource,
    context: &RangeProjectionContext,
    params: &ConsensusParameters,
) -> anyhow::Result<B256> {
    let resources = fixture.execution.witnesses[start..]
        .iter()
        .map(|witness| EtnaBlockExecutionWitness {
            witness,
            ancestor_headers: &witness.headers,
        })
        .collect::<Vec<_>>();
    verify_range_input_consensus(
        input,
        source,
        context,
        &resources,
        &[],
        L1HistoryLimits {
            max_headers: 131_072,
            max_proof_nodes: 128,
        },
        &config(),
        params,
    )
}

#[test]
fn owned_consensus_roundtrip_verifies_real_execution_qcs_and_parent_bootstrap() {
    for (count, start) in [(1, 0), (2, 0), (3, 0), (4, 1), (4, 2), (4, 3)] {
        let fixture = Fixture::new(count, Candidate::Valid, U256::from(1), false);
        let (input, source, context) = owned(&fixture, start);
        let encoded = serde_json::to_vec(&input).unwrap();
        let decoded =
            serde_json::from_slice::<ConsensusRangeInput<ExecutionWitness>>(&encoded).unwrap();
        // JSON deserialization canonicalizes existing witness nodes by hash. Compare only
        // those node bags without order; all native bytes, range lists and other arrays stay exact.
        let normalize_state_order = |value: &mut serde_json::Value| {
            let consensus = &mut value["consensus"];
            consensus["reference_state"]["state"]
                .as_array_mut()
                .unwrap()
                .sort_by_cached_key(|node| node.to_string());
            for anchor in consensus["anchors"].as_array_mut().unwrap() {
                anchor["witness"]["state"]
                    .as_array_mut()
                    .unwrap()
                    .sort_by_cached_key(|node| node.to_string());
            }
        };
        let mut expected: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        let mut actual = serde_json::to_value(&decoded).unwrap();
        normalize_state_order(&mut expected);
        normalize_state_order(&mut actual);
        assert_eq!(actual, expected);
        assert_eq!(
            verify_owned(&fixture, start, &decoded, &source, &context, &parameters()).unwrap(),
            decoded.range.statement.as_statement().try_digest().unwrap()
        );
    }
}

#[test]
fn owned_consensus_rejects_missing_extra_or_noncanonical_native_inputs() {
    let fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
    for mutation in [
        "missing_original",
        "extra_original",
        "missing_qc",
        "extra_qc",
        "noncanonical_original",
        "noncanonical_qc",
        "missing_anchor",
        "extra_anchor",
    ] {
        let (mut input, source, context) = owned(&fixture, 0);
        match mutation {
            "missing_original" => {
                input.consensus.original_blocks.pop();
            }
            "extra_original" => input
                .consensus
                .original_blocks
                .push(input.consensus.original_blocks[0].clone()),
            "missing_qc" => {
                input.consensus.required_commits.remove(0);
            }
            "extra_qc" => input.consensus.required_commits.insert(
                0,
                RawCommit::from(fixture.all_commits[0].clone()).encode_to_vec(),
            ),
            "noncanonical_original" => {
                input.consensus.original_blocks[0].extend_from_slice(&[0xf8, 0x07, 1])
            }
            "noncanonical_qc" => {
                input.consensus.required_commits[0].extend_from_slice(&[0xf8, 0x07, 1])
            }
            "missing_anchor" => input.consensus.anchors.clear(),
            "extra_anchor" => input
                .consensus
                .anchors
                .push(input.consensus.anchors[0].clone()),
            _ => unreachable!(),
        }
        assert!(
            verify_owned(&fixture, 0, &input, &source, &context, &parameters()).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn owned_consensus_rejects_qc_crypto_parent_proofs_and_trusted_budget_mutations() {
    let fixture = Fixture::new(4, Candidate::Valid, U256::from(1), false);
    for mutation in [
        "forged_qc",
        "two_thirds",
        "reference_state",
        "anchor_state",
        "parent_active",
        "parent_next",
        "parent_anchor",
        "block_budget",
        "commit_budget",
    ] {
        let (mut input, source, context) = owned(&fixture, 1);
        let mut params = parameters();
        match mutation {
            "forged_qc" | "two_thirds" => {
                let mut commit = etna_core::comet::codec::decode_canonical_commit(
                    &input.consensus.required_commits[0],
                )
                .unwrap();
                if mutation == "two_thirds" {
                    commit.signatures[0] = CommitSig::BlockIdFlagAbsent;
                } else {
                    let CommitSig::BlockIdFlagCommit { signature, .. } = &mut commit.signatures[0]
                    else {
                        panic!()
                    };
                    *signature = Some(vec![0; 65].try_into().unwrap());
                }
                input.consensus.required_commits[0] = RawCommit::from(commit).encode_to_vec();
            }
            "reference_state" => input.consensus.reference_state.state.clear(),
            "anchor_state" => input.consensus.anchors[0].witness.state.clear(),
            "parent_active" => input.consensus.parent.as_mut().unwrap().active[0].1 += 1,
            "parent_next" => input.consensus.parent.as_mut().unwrap().next[0].1 += 1,
            "parent_anchor" => input.consensus.parent.as_mut().unwrap().anchor_header[0] ^= 1,
            "block_budget" => params.max_block_bytes = 1,
            "commit_budget" => params.max_commit_bytes = 1,
            _ => unreachable!(),
        }
        assert!(
            verify_owned(&fixture, 1, &input, &source, &context, &params).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn owned_consensus_cannot_serialize_authority_or_accept_unknown_consensus_fields() {
    let fixture = Fixture::new(2, Candidate::Valid, U256::from(1), false);
    let (input, _, _) = owned(&fixture, 1);
    for path in ["request", "consensus", "anchor", "parent"] {
        for field in ["host_verified", "context", "limits", "parameters"] {
            let mut encoded = serde_json::to_value(&input).unwrap();
            let target = match path {
                "request" => &mut encoded,
                "consensus" => &mut encoded["consensus"],
                "anchor" => &mut encoded["consensus"]["anchors"][0],
                "parent" => &mut encoded["consensus"]["parent"],
                _ => unreachable!(),
            };
            target[field] = serde_json::json!(true);
            assert!(
                serde_json::from_value::<ConsensusRangeInput<ExecutionWitness>>(encoded).is_err(),
                "{path}/{field}"
            );
        }
    }
}

// Append to guest-common/src/etna/tests/execution/consensus/owned.rs.
#[test]
fn independent_owned_canonical_inputs_cannot_be_reordered_or_substituted() {
    let fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
    for mutation in [
        "blocks",
        "commits",
        "header_projection",
        "transaction_projection",
    ] {
        let (mut input, source, context) = owned(&fixture, 0);
        match mutation {
            "blocks" => input.consensus.original_blocks.swap(0, 1),
            "commits" => input.consensus.required_commits.swap(0, 1),
            "header_projection" => {
                input.range.blocks[1].comet_header.validators_hash =
                    tendermint::Hash::Sha256([0xaa; 32])
            }
            "transaction_projection" => input.range.blocks[1].comet_transactions[0].push(0),
            _ => unreachable!(),
        }
        assert!(
            verify_owned(&fixture, 0, &input, &source, &context, &parameters()).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn independent_owned_bootstrap_presence_depends_on_authenticated_origin() {
    let fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
    let (mut origin, origin_source, origin_context) = owned(&fixture, 0);
    let (mut suffix, suffix_source, suffix_context) = owned(&fixture, 1);
    origin.consensus.parent = suffix.consensus.parent.take();
    assert!(verify_owned(
        &fixture,
        0,
        &origin,
        &origin_source,
        &origin_context,
        &parameters()
    )
    .is_err());
    assert!(verify_owned(
        &fixture,
        1,
        &suffix,
        &suffix_source,
        &suffix_context,
        &parameters()
    )
    .is_err());
}

#[test]
fn independent_owned_external_execution_resources_are_complete_and_positional() {
    let fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
    let (input, source, context) = owned(&fixture, 0);
    let resources: Vec<_> = fixture
        .execution
        .witnesses
        .iter()
        .map(|witness| EtnaBlockExecutionWitness {
            witness,
            ancestor_headers: &witness.headers,
        })
        .collect();
    let limits = L1HistoryLimits {
        max_headers: 131_072,
        max_proof_nodes: 128,
    };
    for supplied in [
        resources[..2].to_vec(),
        vec![resources[0], resources[1], resources[2], resources[2]],
        vec![resources[1], resources[0], resources[2]],
    ] {
        assert!(verify_range_input_consensus(
            &input,
            &source,
            &context,
            &supplied,
            &[],
            limits,
            &config(),
            &parameters()
        )
        .is_err());
    }
}

#[test]
fn independent_owned_wrong_trusted_domain_and_reference_remain_rejected() {
    let fixture = Fixture::new(2, Candidate::Valid, U256::from(1), false);
    let (input, source, context) = owned(&fixture, 0);
    for mutation in ["generation", "inbox", "l1_chain", "l2_chain", "reference"] {
        let mut changed = context;
        match mutation {
            "generation" => changed.domain.generation += 1,
            "inbox" => changed.domain.inbox = Address::ZERO,
            "l1_chain" => changed.domain.l1_chain_id += U256::ONE,
            "l2_chain" => changed.domain.l2_chain_id += U256::ONE,
            "reference" => { /* Reference identity is committed by the supplied statement below. */
            }
            _ => unreachable!(),
        }
        let mut changed_input = input.clone();
        if mutation == "reference" {
            changed_input.range.statement.l1_reference_block_hash = B256::ZERO;
        }
        assert!(
            verify_owned(
                &fixture,
                0,
                &changed_input,
                &source,
                &changed,
                &parameters()
            )
            .is_err(),
            "{mutation}"
        );
    }
}

mod acceptance;
