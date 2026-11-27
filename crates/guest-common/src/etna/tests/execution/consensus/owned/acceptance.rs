//! Complete retained-context PoC controls use real EVM execution, MPT roots and native QCs.
use super::*;
use crate::etna::{etna_devnet_config, verify_range_input_acceptance, EtnaAcceptancePolicy};

fn fixture_with_context(count: usize, overrides: &[(U256, U256)]) -> Fixture {
    Fixture::with_fixture_parameters(
        count,
        Candidate::Valid,
        U256::from(1),
        false,
        overrides,
        U256::from(0x0701),
        true,
    )
}
fn policy(input: &ConsensusRangeInput<ExecutionWitness>) -> EtnaAcceptancePolicy {
    EtnaAcceptancePolicy::devnet(
        input.range.statement.l1_chain_id,
        input.range.statement.inbox,
        STAKING,
        parameters().max_uncheckpointed,
    )
}
fn verify_acceptance(
    fixture: &Fixture,
    start: usize,
    input: &ConsensusRangeInput<ExecutionWitness>,
    source: &InputDataSource,
    policy: &EtnaAcceptancePolicy,
) -> anyhow::Result<B256> {
    let resources = fixture.execution.witnesses[start..]
        .iter()
        .map(|witness| EtnaBlockExecutionWitness {
            witness,
            ancestor_headers: &witness.headers,
        })
        .collect::<Vec<_>>();
    let config = etna_devnet_config(167_001, input.range.blocks[0].execution_header.timestamp)?;
    verify_range_input_acceptance(input, source, &resources, &[], &config, policy)
}

#[test]
fn native_acceptance_verifies_genuine_ranges_and_exports_sgx_request() {
    for (count, start) in [(1, 0), (2, 0), (3, 0), (4, 2)] {
        let fixture = fixture_with_context(count, &[]);
        let (input, source, context) = owned(&fixture, start);
        assert!(verify_owned(&fixture, start, &input, &source, &context, &parameters()).is_ok());
        let digest = verify_acceptance(&fixture, start, &input, &source, &policy(&input)).unwrap();
        assert_eq!(
            digest,
            input.range.statement.as_statement().try_digest().unwrap()
        );
        if count == 3 && start == 0 {
            if let Some(path) = std::env::var_os("RAIKO2_ETNA_TEST_REQUEST_OUTPUT") {
                let executions=fixture.execution.witnesses.iter().map(|witness|serde_json::json!({"witness":witness,"ancestor_headers":witness.headers})).collect::<Vec<_>>();
                let request = serde_json::json!({
                    "schema":"raiko2-etna-request-v1",
                    "payload":{
                        "input":input,"data_source":source,"executions":executions,
                        "shared_state_nodes":[],"staking":STAKING,
                        "unsettled_limit":parameters().max_uncheckpointed,
                        "verifier_address":Address::with_last_byte(0x99),
                        "expected_statement_hash":digest,
                    }
                });
                std::fs::write(path, serde_json::to_vec(&request).unwrap()).unwrap();
            }
        }
    }
}

#[test]
fn native_acceptance_rejects_proven_context_words_after_valid_native_composition() {
    let base = fixture_with_context(1, &[]);
    let anchor = etna_core::l1_header::L1HeaderView::from_raw(
        &base.execution.authenticated.range.l1_headers[0],
    )
    .unwrap()
    .timestamp();
    let origin = origin_slots(2);
    for (slot, value, expected) in [
        (U256::from(270), U256::from(2), "checkpoint"),
        (
            U256::from(282),
            U256::from(1) | (U256::from(1151) << 48),
            "protocol floor",
        ),
        (U256::from(293), U256::from(1) << 48, "ruleset"),
        (
            U256::from(293),
            U256::from(anchor - 1) << 144,
            "planned stop",
        ),
        (
            U256::from_be_bytes(origin[1].0),
            U256::from(19),
            "L1 origin",
        ),
        (
            U256::from_be_bytes(origin[2].0),
            U256::from(anchor + 1),
            "initial Comet time",
        ),
    ] {
        let fixture = fixture_with_context(1, &[(slot, value)]);
        let (input, source, context) = owned(&fixture, 0);
        assert!(
            verify_owned(&fixture, 0, &input, &source, &context, &parameters()).is_ok(),
            "native control for {expected}"
        );
        let error =
            verify_acceptance(&fixture, 0, &input, &source, &policy(&input)).expect_err(expected);
        assert!(
            format!("{error:#}").contains(expected),
            "{expected}: {error:#}"
        );
    }
}

#[test]
fn native_acceptance_rejects_gas_and_mix_with_otherwise_valid_execution_and_resigned_qc() {
    for mutation in ["gas", "mix"] {
        let mut fixture = fixture_with_context(1, &[]);
        let header = &mut fixture.execution.authenticated.range.blocks[0].execution;
        if mutation == "gas" {
            header.gas_limit -= 1;
        } else {
            header.mix_hash = B256::ZERO;
        }
        fixture.execution.rebind();
        fixture.seal_consensus(false);
        let (input, source, context) = owned(&fixture, 0);
        assert!(
            verify_owned(&fixture, 0, &input, &source, &context, &parameters()).is_ok(),
            "otherwise valid {mutation} execution must pass component"
        );
        let error =
            verify_acceptance(&fixture, 0, &input, &source, &policy(&input)).expect_err(mutation);
        assert!(format!("{error:#}").contains(mutation), "{error:#}");
    }
}

#[test]
fn native_acceptance_binds_statement_reference_policy_and_actual_state_nodes() {
    let fixture = fixture_with_context(2, &[]);
    for mutation in [
        "Q",
        "parent",
        "deployment",
        "origin_nodes",
        "anchor_nodes",
        "gas_policy",
    ] {
        let (mut input, source, _) = owned(&fixture, 0);
        let mut accepted = policy(&input);
        match mutation {
            "Q" => input.range.statement.l1_reference_block_hash = B256::ZERO,
            "parent" => input.range.statement.parent_hash = B256::ZERO,
            "deployment" => accepted.chain.inbox = Address::with_last_byte(0x77),
            "origin_nodes" => input.consensus.reference_state.state.clear(),
            "anchor_nodes" => input.consensus.anchors[0].witness.state.clear(),
            "gas_policy" => accepted.chain.block_gas_limit -= 1,
            _ => unreachable!(),
        }
        assert!(
            verify_acceptance(&fixture, 0, &input, &source, &accepted).is_err(),
            "{mutation}"
        );
    }
    assert!(etna_devnet_config(167_000, 100).is_err());
    assert!(etna_devnet_config(167_013, 100).is_err());
}
