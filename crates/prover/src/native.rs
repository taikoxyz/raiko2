//! Native prover implementation (no zk proof).

use alloy_primitives::{Address, B256, Bytes, address, keccak256};
use raiko2_guest_common::{
    aggregate_shasta_zk_with_verifier, prove_shasta_proposal_for_proof_type,
};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{Proof, ProofType, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{
    GuestInput, encode_proof_carry_data,
    instance::{words_to_bytes_be, words_to_bytes_le},
};

use crate::{GuestInputCodec, build_shasta_aggregation_input};

/// Native prover for local execution (returns public input only).
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeProver;

const SHASTA_SGX_PROOF_LEN: usize = 89;
const SHASTA_NATIVE_MOCK_INSTANCE_ID: u32 = 0xDEAD_C0DE;
// Native proofs are explicit host-local mocks. Keep the instance stable so downstream
// fixture/tests can identify the mock path without embedding key material in the repo.
const SHASTA_NATIVE_MOCK_INSTANCE: Address = address!("0000777735367b36bc9b61c50022d9d0700db4ec");
const NATIVE_MOCK_SIGNATURE_DOMAIN: &[u8] = b"raiko2-native-mock-signature";

impl GuestInputCodec<GuestInput> for NativeProver {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {e}")))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for NativeProver
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;
        if input.witnesses.is_empty() {
            return Err(RaikoError::Guest(
                "GuestInput must contain at least one witness".to_string(),
            ));
        }

        let proof_carry_data = input.proof_carry_data.clone();

        let extra_data = encode_proof_carry_data(&proof_carry_data)?;
        let input_hash = prove_shasta_proposal_for_proof_type(&input, ProofType::Native)
            .map_err(|e| RaikoError::Guest(format!("Native proposal execute failed: {e}")))?;
        let signature = mock_signature(input_hash);
        let sgx_instance = signer_address()?;
        let proof =
            build_shasta_proof_bytes(SHASTA_NATIVE_MOCK_INSTANCE_ID, sgx_instance, signature);

        Ok(Proof {
            proof: Some(format!("0x{}", hex::encode(proof))),
            input: Some(input_hash),
            extra_data: Some(extra_data),
            ..Default::default()
        })
    }

    async fn aggregate(
        &self,
        input: raiko2_primitives::AggregationGuestInput,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let aggregation_input = build_shasta_aggregation_input(&input.proofs)?;

        let endianness = config
            .get("native_image_id_endianness")
            .and_then(|v| v.as_str())
            .unwrap_or("be");
        let image_id_bytes = match endianness {
            "be" => words_to_bytes_be(&aggregation_input.image_id),
            "le" => words_to_bytes_le(&aggregation_input.image_id),
            other => {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "Unsupported native_image_id_endianness: {other}"
                )));
            }
        };
        let sub_image_id = B256::from(image_id_bytes);
        let aggregation_hash = aggregate_shasta_zk_with_verifier(
            &aggregation_input,
            sub_image_id,
            |index, block_input| {
                let proof = input.proofs.get(index).ok_or_else(|| {
                    anyhow::anyhow!("missing native child proof at index {index}")
                })?;
                verify_native_proof_envelope(proof, *block_input)
                    .map_err(|err| anyhow::anyhow!(err.to_string()))
            },
        )
        .map_err(|e| RaikoError::Guest(format!("Native aggregation execute failed: {e}")))?;

        let sgx_instance = signer_address()?;
        let signature = mock_signature(aggregation_hash);
        let proof =
            build_shasta_proof_bytes(SHASTA_NATIVE_MOCK_INSTANCE_ID, sgx_instance, signature);

        Ok(Proof {
            proof: Some(format!("0x{}", hex::encode(proof))),
            input: Some(aggregation_hash),
            ..Default::default()
        })
    }
}

fn mock_signature(hash: B256) -> [u8; 65] {
    let mut left_seed = Vec::with_capacity(NATIVE_MOCK_SIGNATURE_DOMAIN.len() + 5 + 32);
    left_seed.extend_from_slice(NATIVE_MOCK_SIGNATURE_DOMAIN);
    left_seed.extend_from_slice(b":left");
    left_seed.extend_from_slice(hash.as_slice());

    let mut right_seed = Vec::with_capacity(NATIVE_MOCK_SIGNATURE_DOMAIN.len() + 6 + 32);
    right_seed.extend_from_slice(NATIVE_MOCK_SIGNATURE_DOMAIN);
    right_seed.extend_from_slice(b":right");
    right_seed.extend_from_slice(hash.as_slice());

    let left = keccak256(left_seed);
    let right = keccak256(right_seed);
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..32].copy_from_slice(left.as_slice());
    sig_bytes[32..64].copy_from_slice(right.as_slice());
    sig_bytes[64] = 27;
    sig_bytes
}

fn signer_address() -> RaikoResult<Address> {
    Ok(SHASTA_NATIVE_MOCK_INSTANCE)
}

fn verify_native_proof_envelope(proof: &Proof, expected_input: B256) -> RaikoResult<()> {
    let proof_hex = proof.proof.as_deref().ok_or_else(|| {
        RaikoError::InvalidRequestConfig("native proof is missing proof bytes".to_string())
    })?;
    let bytes = decode_native_proof_bytes(proof_hex)?;

    let instance_id = u32::from_be_bytes(
        bytes[..4]
            .try_into()
            .expect("native proof instance id prefix length"),
    );
    if instance_id != SHASTA_NATIVE_MOCK_INSTANCE_ID {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "native proof has unexpected instance id: expected {SHASTA_NATIVE_MOCK_INSTANCE_ID:#x}, got {instance_id:#x}"
        )));
    }

    let instance = Address::from_slice(&bytes[4..24]);
    if instance != SHASTA_NATIVE_MOCK_INSTANCE {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "native proof has unexpected instance address: expected {SHASTA_NATIVE_MOCK_INSTANCE}, got {instance}"
        )));
    }

    if let Some(input_hash) = proof.input
        && input_hash != expected_input
    {
        return Err(RaikoError::InvalidRequestConfig(
            "native proof input hash does not match expected child input".to_string(),
        ));
    }

    let sig: [u8; 65] = bytes[24..]
        .try_into()
        .expect("native proof signature length");
    if sig != mock_signature(expected_input) {
        return Err(RaikoError::InvalidRequestConfig(
            "native proof signature does not match expected child input".to_string(),
        ));
    }

    Ok(())
}

fn decode_native_proof_bytes(proof_hex: &str) -> RaikoResult<[u8; SHASTA_SGX_PROOF_LEN]> {
    let bytes = hex::decode(proof_hex.trim_start_matches("0x"))
        .map_err(|e| RaikoError::InvalidRequestConfig(format!("invalid native proof hex: {e}")))?;
    let len = bytes.len();
    bytes.try_into().map_err(|_| {
        RaikoError::InvalidRequestConfig(format!(
            "native proof has invalid length: expected {SHASTA_SGX_PROOF_LEN} bytes, got {}",
            len
        ))
    })
}

fn build_shasta_proof_bytes(instance_id: u32, instance: Address, sig: [u8; 65]) -> Vec<u8> {
    let mut proof = Vec::with_capacity(SHASTA_SGX_PROOF_LEN);
    proof.extend(instance_id.to_be_bytes());
    proof.extend(instance);
    proof.extend(sig);
    proof
}

#[cfg(test)]
mod tests {
    use super::NativeProver;
    use crate::Prover;
    use alloy_primitives::{Address, B256};
    use raiko2_guest_common::{
        aggregate_shasta_zk_with_verifier, prove_shasta_proposal_for_proof_type,
    };
    use raiko2_pipeline::NativeBackend;
    use raiko2_primitives::{
        AggregationGuestInput, Proof, ProofType, ProverConfig, SupportedChainSpecs,
    };
    use raiko2_primitives_shasta::{
        GuestInput, build_proof_carry_data, encode_proof_carry_data, instance::words_to_bytes_be,
    };
    use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
    use raiko2_protocol_shasta::shasta::ProofCarryData;
    use std::str::FromStr;

    const EXPECTED_ADDR: &str = "0x0000777735367b36bC9B61C50022d9D0700dB4Ec";
    const EXPECTED_INSTANCE_ID: u32 = super::SHASTA_NATIVE_MOCK_INSTANCE_ID;

    fn decode_proof_bytes(proof_hex: &str) -> [u8; 89] {
        super::decode_native_proof_bytes(proof_hex).expect("proof bytes")
    }

    fn fixture_guest_input() -> GuestInput {
        let raw = include_str!(
            "../../../test/guest_inputs/shasta/taiko_hoodi/proposals/proposal_17460.json"
        );
        let mut guest_input: GuestInput =
            serde_json::from_str(raw).expect("parse fixture GuestInput");
        if guest_input.taiko.l1_ancestor_headers.is_empty()
            && guest_input.taiko.l1_header.number != 0
        {
            guest_input.taiko.l1_ancestor_headers = vec![guest_input.taiko.l1_header.clone()];
        }
        if let Some(chain_spec) = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(guest_input.taiko.chain_spec.chain_id)
        {
            guest_input.taiko.chain_spec.name = chain_spec.name.clone();
            guest_input.taiko.chain_spec.chain_id = chain_spec.chain_id;
            guest_input.taiko.chain_spec.is_taiko = chain_spec.is_taiko;
            for witness in &mut guest_input.witnesses {
                witness.chain_spec = chain_spec.clone();
            }
            guest_input.proof_carry_data =
                build_proof_carry_data(&guest_input, ProofType::Native).expect("rebuild carry");
        }
        guest_input
    }

    fn native_proof_with_input(input_hash: B256) -> Proof {
        Proof {
            proof: Some(format!(
                "0x{}",
                hex::encode(super::build_shasta_proof_bytes(
                    EXPECTED_INSTANCE_ID,
                    Address::from_str(EXPECTED_ADDR).expect("expected addr"),
                    super::mock_signature(input_hash),
                ))
            )),
            input: Some(input_hash),
            extra_data: Some(
                serde_json::to_value(ProofCarryData {
                    chain_id: 1,
                    ..ProofCarryData::default()
                })
                .unwrap_or_default(),
            ),
            ..Proof::default()
        }
    }

    #[test]
    fn native_signer_address_matches_mock_instance_address() {
        let address = super::signer_address().expect("signer address");
        let expected = Address::from_str(EXPECTED_ADDR).expect("expected address");
        assert_eq!(address, expected);
    }

    #[tokio::test]
    async fn native_proposal_proof_matches_shasta_format() {
        let prover = NativeProver;
        let config = ProverConfig::default();
        let input = fixture_guest_input();
        let expected_hash =
            prove_shasta_proposal_for_proof_type(&input, ProofType::Native).expect("proposal");
        let proof = prover
            .prove(input, &config, &NativeBackend)
            .await
            .expect("prove");

        let proof_hex = proof.proof.clone().expect("missing proof");
        let bytes = decode_proof_bytes(&proof_hex);
        assert_eq!(&bytes[..4], EXPECTED_INSTANCE_ID.to_be_bytes());

        let instance = Address::from_slice(&bytes[4..24]);
        let expected_addr = Address::from_str(EXPECTED_ADDR).unwrap();
        assert_eq!(instance, expected_addr);
        assert_eq!(proof.input.expect("missing input"), expected_hash);

        let sig: [u8; 65] = bytes[24..].try_into().expect("sig bytes");
        assert_eq!(sig, super::mock_signature(expected_hash));
    }

    #[tokio::test]
    async fn native_aggregation_proof_signs_pcd_hash() {
        let prover = NativeProver;

        let proof_carry = ProofCarryData {
            chain_id: 1,
            ..ProofCarryData::default()
        };
        let child_input = hash_shasta_subproof_input(&proof_carry);
        let proofs = vec![Proof {
            proof: Some(format!(
                "0x{}",
                hex::encode(super::build_shasta_proof_bytes(
                    EXPECTED_INSTANCE_ID,
                    Address::from_str(EXPECTED_ADDR).unwrap(),
                    super::mock_signature(child_input),
                ))
            )),
            input: Some(child_input),
            extra_data: Some(encode_proof_carry_data(&proof_carry).expect("encode carry data")),
            ..Proof::default()
        }];

        let proof = prover
            .aggregate(
                AggregationGuestInput {
                    proofs: proofs.clone(),
                },
                &serde_json::json!({}),
                &NativeBackend,
            )
            .await
            .expect("aggregate");
        let aggregation_input =
            crate::build_shasta_aggregation_input(&proofs).expect("aggregation input");
        let image_id_b256 = B256::from(words_to_bytes_be(&aggregation_input.image_id));
        let expected_hash = aggregate_shasta_zk_with_verifier(
            &aggregation_input,
            image_id_b256,
            |index, block_input| {
                let child = proofs.get(index).expect("child proof");
                super::verify_native_proof_envelope(child, *block_input)
                    .map_err(|err| anyhow::anyhow!(err.to_string()))
            },
        )
        .expect("expected aggregation");

        let proof_hex = proof.proof.clone().expect("missing proof");
        let bytes = decode_proof_bytes(&proof_hex);
        assert_eq!(&bytes[..4], EXPECTED_INSTANCE_ID.to_be_bytes());
        assert_eq!(proof.input.expect("missing input"), expected_hash);

        let sig: [u8; 65] = bytes[24..].try_into().expect("sig bytes");
        assert_eq!(sig, super::mock_signature(expected_hash));
    }

    #[test]
    fn native_proof_envelope_verifier_accepts_matching_child_proof() {
        let input_hash = B256::repeat_byte(0x55);
        let proof = native_proof_with_input(input_hash);
        super::verify_native_proof_envelope(&proof, input_hash).expect("valid native proof");
    }

    #[test]
    fn native_proof_envelope_verifier_rejects_signature_mismatch() {
        let mut proof = native_proof_with_input(B256::repeat_byte(0x55));
        proof.input = None;
        let err = super::verify_native_proof_envelope(&proof, B256::repeat_byte(0x77))
            .expect_err("signature mismatch");
        assert!(
            err.to_string()
                .contains("native proof signature does not match expected child input")
        );
    }
}
