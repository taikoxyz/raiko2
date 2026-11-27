//! Etna requests reuse the supplied-block verifier and existing SGX signer.

use raiko2_guest_common::etna::{
    EtnaAcceptancePolicy, EtnaBlockExecutionWitness, etna_devnet_config,
    verify_range_input_acceptance,
};
use raiko2_prover::remote_prover::protocol::{
    RAIKO2_ETNA_REQUEST_SCHEMA, Raiko2EtnaRequest, Raiko2ProofResponse,
};

use crate::{
    protocol::{RequestFailure, proof_result_from_etna_statement},
    tee::TeeProvider,
};

pub(crate) fn prove_etna_request<P: TeeProvider>(
    provider: &P,
    instance_id: u32,
    request: &Raiko2EtnaRequest,
) -> Result<Raiko2ProofResponse, RequestFailure> {
    if request.schema != RAIKO2_ETNA_REQUEST_SCHEMA {
        return Err(RequestFailure::invalid_request(
            "unsupported Etna request schema",
        ));
    }
    let payload = &request.payload;
    let statement = &payload.input.range.statement;
    let chain_id = u64::try_from(statement.l2_chain_id)
        .map_err(|_| RequestFailure::invalid_request("Etna L2 chain ID exceeds uint64"))?;
    if payload.verifier_address.is_zero() || payload.expected_statement_hash.is_zero() {
        return Err(RequestFailure::invalid_request(
            "Etna verifier and statement must be nonzero",
        ));
    }
    // The image selects the execution implementation; the caller cannot replace the chain
    // specification. This initial internal-devnet profile enables Etna throughout this range.
    let first = payload
        .input
        .range
        .blocks
        .first()
        .ok_or_else(|| RequestFailure::invalid_request("Etna range must not be empty"))?;
    let config = etna_devnet_config(chain_id, first.execution_header.timestamp)
        .map_err(|err| RequestFailure::invalid_request(err.to_string()))?;
    let policy = EtnaAcceptancePolicy::devnet(
        statement.l1_chain_id,
        statement.inbox,
        payload.staking,
        payload.unsettled_limit,
    );
    let witnesses: Vec<_> = payload
        .executions
        .iter()
        .map(|resource| EtnaBlockExecutionWitness {
            witness: &resource.witness,
            ancestor_headers: &resource.ancestor_headers,
        })
        .collect();
    let digest = verify_range_input_acceptance(
        &payload.input,
        &payload.data_source,
        &witnesses,
        &payload.shared_state_nodes,
        &config,
        &policy,
    )
    .map_err(|err| RequestFailure::invalid_request(format!("invalid Etna range: {err:#}")))?;
    if digest != payload.expected_statement_hash {
        return Err(RequestFailure::invalid_request(
            "Etna verified statement differs from client job identity",
        ));
    }
    // Key/quote access and signing occur only after the full verification relation succeeds.
    let result = proof_result_from_etna_statement(
        provider,
        instance_id,
        digest,
        payload.verifier_address,
        chain_id,
    )
    .map_err(|err| RequestFailure::prover_error(err.to_string()))?;
    Ok(Raiko2ProofResponse::success(result))
}

// Append to sgx-runtime/src/etna.rs. This is an intentionally invalid transport fixture;
// successful complete-context requests are verified by separate genuine execution fixtures.
#[cfg(test)]
pub(crate) mod independent_tests {
    use super::*;
    use alloy_primitives::{Address, B256, U256};
    use raiko2_primitives::{ExecutionWitness, WitnessHeader};
    use secp256k1::SecretKey;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Default)]
    pub(crate) struct NoSignProvider(pub(crate) Arc<AtomicUsize>);

    impl TeeProvider for NoSignProvider {
        fn save_private_key(&self, _: &SecretKey) -> anyhow::Result<()> {
            panic!("invalid request must not write a key")
        }
        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("invalid request reached signer")
        }
        fn load_quote(&self, _: Address) -> anyhow::Result<Vec<u8>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("invalid request reached quote provider")
        }
    }

    pub(crate) fn request() -> Raiko2EtnaRequest {
        let header = reth_ethereum_primitives::Block::default().header;
        let value = serde_json::json!({
            "schema": RAIKO2_ETNA_REQUEST_SCHEMA,
            "payload": {
                "input": {
                    "range": {
                        "blocks": [], "post_block_cursors": [], "parent_execution_header": header,
                        "l1_history": {"reference_header": [], "anchors": [], "beacon": null},
                        "statement": {
                            "l1_chain_id": "0x1", "l2_chain_id": "0x28c59",
                            "inbox": Address::repeat_byte(0x11), "generation": "0x0",
                            "parent_hash": B256::repeat_byte(1), "parent_comet_hash": B256::ZERO,
                            "config_hash": B256::repeat_byte(2), "l1_reference_block_hash": B256::repeat_byte(3),
                            "blob_hashes": [], "forced_inclusion_head_before": "0x0", "cut_leaves": [],
                            "input": {
                                "parent_height": "0x0", "last_height": "0x1", "last_block_hash": B256::repeat_byte(4),
                                "last_comet_hash": B256::repeat_byte(5), "last_state_root": B256::repeat_byte(6),
                                "l1_reference_block_number": "0x12", "forced_inclusion_head": "0x0"
                            }
                        }
                    },
                    "consensus": {
                        "original_blocks": [], "required_commits": [],
                        "origin_fallback_keys": ["0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"],
                        "reference_state": ExecutionWitness::default(), "anchors": [], "parent": null
                    }
                },
                "data_source": {"tx_data_from_calldata": [], "tx_data_from_blob": [], "blob_commitments": [], "blob_proofs": [], "is_forced_inclusion": false},
                "executions": [], "shared_state_nodes": [], "staking": Address::repeat_byte(0x22),
                "unsettled_limit": 128, "verifier_address": Address::repeat_byte(0x33),
                "expected_statement_hash": B256::repeat_byte(0x44)
            }
        });
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn independent_etna_request_roundtrip_preserves_raw_bytes_and_resource_counts() {
        let mut request = request();
        request.payload.input.consensus.original_blocks = vec![vec![0, 128, 255], vec![]];
        request.payload.input.consensus.required_commits = vec![vec![255, 0]];
        request.payload.executions = vec![
                raiko2_prover::remote_prover::protocol::Raiko2EtnaExecutionWitness {
                    witness: ExecutionWitness::default(),
                    ancestor_headers: vec![WitnessHeader::from_header(
                        reth_ethereum_primitives::Block::default().header
                    )],
                };
                2
            ];
        let json = serde_json::to_value(&request).unwrap();
        let decoded: Raiko2EtnaRequest = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), json);
        assert_eq!(
            decoded.payload.input.consensus.original_blocks,
            vec![vec![0, 128, 255], vec![]]
        );
        assert_eq!(decoded.payload.executions.len(), 2);
        assert_eq!(decoded.payload.input.range.blocks.len(), 0);
        assert_eq!(
            decoded.payload.input.consensus.required_commits,
            vec![vec![255, 0]]
        );
        for scope in ["request", "payload", "execution"] {
            let mut changed = json.clone();
            match scope {
                "request" => changed["host_verified"] = true.into(),
                "payload" => changed["payload"]["context_approved"] = true.into(),
                "execution" => changed["payload"]["executions"][0]["preverified"] = true.into(),
                _ => unreachable!(),
            }
            assert!(
                serde_json::from_value::<Raiko2EtnaRequest>(changed).is_err(),
                "{scope}"
            );
        }
    }

    #[test]
    fn independent_invalid_etna_requests_never_touch_signer_or_quote() {
        for mutation in [
            "schema",
            "wide_chain",
            "zero_statement",
            "zero_verifier",
            "empty_range",
        ] {
            let mut request = request();
            match mutation {
                "schema" => request.schema = "raiko2-shasta-request-v1".to_owned(),
                "wide_chain" => {
                    request.payload.input.range.statement.l2_chain_id = U256::ONE << 64usize
                }
                "zero_statement" => request.payload.expected_statement_hash = B256::ZERO,
                "zero_verifier" => request.payload.verifier_address = Address::ZERO,
                "empty_range" => {}
                _ => unreachable!(),
            }
            let provider = NoSignProvider::default();
            let failure = prove_etna_request(&provider, 1, &request).unwrap_err();
            assert_eq!(failure.code, "INVALID_REQUEST", "{mutation}");
            assert_eq!(provider.0.load(Ordering::SeqCst), 0);
        }
    }
}

// Append to sgx-runtime/src/etna.rs. CI first exports a genuine acceptance fixture using the
// same environment variable, then explicitly executes this ignored diagnostic test.
#[cfg(test)]
mod generated_fixture_independent_tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use raiko2_prover::remote_prover::protocol::{Raiko2ProofResponse, Raiko2ProofResult};
    use secp256k1::{
        Message, Secp256k1, SecretKey,
        ecdsa::{RecoverableSignature, RecoveryId},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tower::util::ServiceExt;

    const INSTANCE_ID: u32 = 0x1234_5678;

    #[derive(Clone, Default)]
    struct SyntheticTee {
        keys: Arc<AtomicUsize>,
        quotes: Arc<AtomicUsize>,
    }

    impl TeeProvider for SyntheticTee {
        fn save_private_key(&self, _: &SecretKey) -> anyhow::Result<()> {
            panic!("proving must not write a key")
        }
        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            self.keys.fetch_add(1, Ordering::SeqCst);
            Ok(SecretKey::from_slice(&[9; 32])?)
        }
        fn load_quote(&self, _: Address) -> anyhow::Result<Vec<u8>> {
            self.quotes.fetch_add(1, Ordering::SeqCst);
            Ok(vec![0x12, 0x34])
        }
    }

    fn assert_proof(result: &Raiko2ProofResult, statement: B256, verifier: Address, chain: u64) {
        let bytes =
            hex::decode(result.proof.as_ref().unwrap().strip_prefix("0x").unwrap()).unwrap();
        assert_eq!(bytes.len(), 89);
        assert_eq!(&bytes[..4], &INSTANCE_ID.to_be_bytes());
        let signer = Address::from_slice(&bytes[4..24]);
        // Construct the actual onchain hash independently of the runtime and generic hash helper.
        let mut preimage = [0u8; 160];
        preimage[..12].copy_from_slice(b"VERIFY_PROOF");
        preimage[56..64].copy_from_slice(&chain.to_be_bytes());
        preimage[76..96].copy_from_slice(verifier.as_slice());
        preimage[96..128].copy_from_slice(statement.as_slice());
        preimage[140..160].copy_from_slice(signer.as_slice());
        let digest = alloy_primitives::keccak256(preimage);
        assert_eq!(result.input, format!("{digest:#x}"));
        let recovery = RecoveryId::try_from(i32::from(bytes[88]) - 27).unwrap();
        let signature = RecoverableSignature::from_compact(&bytes[24..88], recovery).unwrap();
        let recovered = Secp256k1::new()
            .recover_ecdsa(
                &Message::from_digest_slice(digest.as_slice()).unwrap(),
                &signature,
            )
            .unwrap();
        assert_eq!(crate::bootstrap::public_key_to_address(&recovered), signer);
        assert_eq!(
            result.instance_address.as_deref(),
            Some(format!("{signer:#x}").as_str())
        );
    }

    #[tokio::test]
    #[ignore = "requires a generated genuine acceptance request; CI runs this explicitly"]
    async fn independent_generated_etna_request_verifies_and_signs_through_native_http() {
        let path = std::env::var_os("RAIKO2_ETNA_TEST_REQUEST_OUTPUT")
            .expect("CI must export the genuine acceptance request before this test");
        let bytes = std::fs::read(path).unwrap();
        let request: Raiko2EtnaRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request.schema, RAIKO2_ETNA_REQUEST_SCHEMA);
        assert!(!request.payload.input.consensus.required_commits.is_empty());
        let statement = request.payload.expected_statement_hash;
        let verifier = request.payload.verifier_address;
        let chain = u64::try_from(request.payload.input.range.statement.l2_chain_id).unwrap();
        let provider = SyntheticTee::default();
        let response = prove_etna_request(&provider, INSTANCE_ID, &request).unwrap();
        assert!(response.error.is_none());
        assert_proof(
            response.result.as_ref().unwrap(),
            statement,
            verifier,
            chain,
        );
        assert_eq!(provider.keys.load(Ordering::SeqCst), 1);
        assert_eq!(provider.quotes.load(Ordering::SeqCst), 1);

        for mutation in [
            "job_identity",
            "missing_qc",
            "reference_proof",
            "execution_count",
            "forced_inclusion",
            "missing_anchor",
        ] {
            let mut changed = request.clone();
            match mutation {
                "job_identity" => changed.payload.expected_statement_hash.0[0] ^= 1,
                "missing_qc" => {
                    assert!(
                        changed
                            .payload
                            .input
                            .consensus
                            .required_commits
                            .pop()
                            .is_some()
                    );
                }
                "reference_proof" => {
                    changed
                        .payload
                        .input
                        .consensus
                        .reference_state
                        .state
                        .clear();
                    changed
                        .payload
                        .input
                        .consensus
                        .reference_state
                        .state_indices
                        .clear();
                }
                "execution_count" => {
                    assert!(changed.payload.executions.pop().is_some());
                }
                "forced_inclusion" => changed.payload.data_source.is_forced_inclusion = true,
                "missing_anchor" => changed.payload.input.consensus.anchors.clear(),
                _ => unreachable!(),
            }
            let rejecting = SyntheticTee::default();
            let failure = prove_etna_request(&rejecting, INSTANCE_ID, &changed).unwrap_err();
            assert_eq!(failure.code, "INVALID_REQUEST", "{mutation}");
            assert_eq!(rejecting.keys.load(Ordering::SeqCst), 0, "{mutation}");
            assert_eq!(rejecting.quotes.load(Ordering::SeqCst), 0, "{mutation}");
        }

        let http_provider = SyntheticTee::default();
        let app = crate::server::router(crate::server::SgxProver {
            provider: http_provider.clone(),
            service_config: crate::config::ServiceConfig {
                listen_addr: "127.0.0.1:0".to_owned(),
                fork: "etna".to_owned(),
                instance_id: INSTANCE_ID,
            },
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/prove/etna")
                    .header("content-type", "application/json")
                    .body(Body::from(bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let response: Raiko2ProofResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(response.error.is_none());
        assert_proof(
            response.result.as_ref().unwrap(),
            statement,
            verifier,
            chain,
        );
        assert_eq!(http_provider.keys.load(Ordering::SeqCst), 1);
        assert_eq!(http_provider.quotes.load(Ordering::SeqCst), 1);
    }
}
