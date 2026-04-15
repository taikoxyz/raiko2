//! Native prover implementation (no zk proof).

use alloy_primitives::{Address, B256, Bytes, keccak256};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{
    GuestInput, encode_proof_carry_data,
    instance::{
        build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
        shasta_zk_aggregation_public_input_from_proof_carry_data_vec, words_to_bytes_be,
        words_to_bytes_le,
    },
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

use crate::{GuestInputCodec, build_shasta_aggregation_input};

/// Native prover for local execution (returns public input only).
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeProver;

const NATIVE_PROOF_PRIVATE_KEY: &str =
    "92954368afd3caa1f3ce3ead0069c1af414054aefe1ef9aeacc1bf426222ce38";
const SHASTA_SGX_PROOF_LEN: usize = 89;
const SHASTA_NATIVE_MOCK_INSTANCE_ID: u32 = 0xDEAD_C0DE;

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

        let proof_carry_data = input.proof_carry_data;

        let extra_data = encode_proof_carry_data(&proof_carry_data)?;
        let input_hash = hash_shasta_subproof_input(&proof_carry_data);
        let signature = sign_hash(input_hash)?;
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

        if shasta_zk_aggregation_public_input_from_proof_carry_data_vec(
            sub_image_id,
            &aggregation_input.proof_carry_data_vec,
            aggregation_input.prover_address,
        )
        .is_none()
        {
            return Err(RaikoError::InvalidRequestConfig(
                "Invalid proof_carry_data_vec".to_string(),
            ));
        }

        let commitment = build_shasta_commitment_from_proof_carry_data_vec(
            &aggregation_input.proof_carry_data_vec,
        )
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig("Invalid proof_carry_data_vec".to_string())
        })?;
        let first = aggregation_input
            .proof_carry_data_vec
            .first()
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig("Missing proof_carry_data_vec".to_string())
            })?;
        let sgx_instance = signer_address()?;
        let aggregation_hash =
            shasta_aggregation_output(&commitment, first.chain_id, first.verifier, sgx_instance);
        let signature = sign_hash(aggregation_hash)?;
        let proof =
            build_shasta_proof_bytes(SHASTA_NATIVE_MOCK_INSTANCE_ID, sgx_instance, signature);

        Ok(Proof {
            proof: Some(format!("0x{}", hex::encode(proof))),
            input: Some(aggregation_hash),
            ..Default::default()
        })
    }
}

fn sign_hash(hash: B256) -> RaikoResult<[u8; 65]> {
    let key_bytes = hex::decode(NATIVE_PROOF_PRIVATE_KEY).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Invalid native proof private key: {e}"))
    })?;
    let secret_key = SecretKey::from_slice(&key_bytes).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Invalid native proof private key: {e}"))
    })?;
    let message = Message::from_digest_slice(hash.as_slice())
        .map_err(|e| RaikoError::InvalidRequestConfig(format!("Invalid hash: {e}")))?;
    let secp = Secp256k1::new();
    let sig = secp.sign_ecdsa_recoverable(&message, &secret_key);
    let (rec_id, data) = sig.serialize_compact();
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(&data);
    let recovery_byte = i32::from(rec_id) + 27;
    sig_bytes[64] = u8::try_from(recovery_byte)
        .map_err(|_| RaikoError::InvalidRequestConfig("Invalid recovery id value".to_string()))?;
    Ok(sig_bytes)
}

fn signer_address() -> RaikoResult<Address> {
    let key_bytes = hex::decode(NATIVE_PROOF_PRIVATE_KEY).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Invalid native proof private key: {e}"))
    })?;
    let secret_key = SecretKey::from_slice(&key_bytes).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Invalid native proof private key: {e}"))
    })?;
    let secp = Secp256k1::new();
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);
    Ok(public_key_to_address(&public_key))
}

fn public_key_to_address(public_key: &PublicKey) -> Address {
    let hash = keccak256(&public_key.serialize_uncompressed()[1..]);
    Address::from_slice(&hash[12..])
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
    use crate::{Prover, hash_shasta_subproof_input};
    use alloy_primitives::{Address, B256, keccak256};
    use raiko2_pipeline::NativeBackend;
    use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig};
    use raiko2_primitives_shasta::{
        GuestInput, encode_proof_carry_data,
        instance::{build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output},
    };
    use raiko2_protocol_shasta::shasta::ProofCarryData;
    use secp256k1::{Message, PublicKey, Secp256k1, ecdsa::RecoverableSignature};
    use std::str::FromStr;

    const EXPECTED_ADDR: &str = "0x0000777735367b36bC9B61C50022d9D0700dB4Ec";
    const EXPECTED_INSTANCE_ID: u32 = super::SHASTA_NATIVE_MOCK_INSTANCE_ID;

    fn recover_address(sig_bytes: &[u8; 65], message: B256) -> Address {
        let rec_id = secp256k1::ecdsa::RecoveryId::try_from(i32::from(sig_bytes[64] - 27))
            .expect("recovery id");
        let sig = RecoverableSignature::from_compact(&sig_bytes[..64], rec_id)
            .expect("recoverable signature");
        let secp = Secp256k1::new();
        let public_key = secp
            .recover_ecdsa(
                &Message::from_digest_slice(message.as_slice()).unwrap(),
                &sig,
            )
            .expect("recover public key");
        public_key_to_address(&public_key)
    }

    fn public_key_to_address(public_key: &PublicKey) -> Address {
        let hash = keccak256(&public_key.serialize_uncompressed()[1..]);
        Address::from_slice(&hash[12..])
    }

    fn decode_proof_bytes(proof_hex: &str) -> [u8; 89] {
        let bytes = hex::decode(proof_hex.trim_start_matches("0x")).expect("hex");
        bytes.try_into().expect("proof length")
    }

    fn minimal_guest_input() -> GuestInput {
        let mut input = GuestInput::default();
        input
            .witnesses
            .push(raiko2_primitives::StatelessInput::default());
        input
    }

    #[test]
    fn signer_address_matches_golden_touch_account() {
        let address = super::signer_address().expect("signer address");
        let expected = Address::from_str(EXPECTED_ADDR).expect("expected address");
        assert_eq!(address, expected);
    }

    #[tokio::test]
    async fn native_proposal_proof_matches_shasta_format() {
        let prover = NativeProver;
        let config = ProverConfig::default();
        let input = minimal_guest_input();
        let proof = prover
            .prove(input, &config, &NativeBackend)
            .await
            .expect("prove");

        let proof_hex = proof.proof.expect("missing proof");
        let bytes = decode_proof_bytes(&proof_hex);
        assert_eq!(&bytes[..4], EXPECTED_INSTANCE_ID.to_be_bytes());

        let instance = Address::from_slice(&bytes[4..24]);
        let expected_addr = Address::from_str(EXPECTED_ADDR).unwrap();
        assert_eq!(instance, expected_addr);

        let sig: [u8; 65] = bytes[24..].try_into().expect("sig bytes");
        let recovered = recover_address(&sig, proof.input.expect("missing input"));
        assert_eq!(recovered, expected_addr);
    }

    #[tokio::test]
    async fn native_aggregation_proof_signs_pcd_hash() {
        let prover = NativeProver;

        let proof_carry = ProofCarryData {
            chain_id: 1,
            ..ProofCarryData::default()
        };
        let proofs = vec![Proof {
            input: Some(hash_shasta_subproof_input(&proof_carry)),
            extra_data: Some(encode_proof_carry_data(&proof_carry).expect("encode carry data")),
            ..Proof::default()
        }];

        let proof = prover
            .aggregate(
                AggregationGuestInput { proofs },
                &serde_json::json!({}),
                &NativeBackend,
            )
            .await
            .expect("aggregate");

        let commitment =
            build_shasta_commitment_from_proof_carry_data_vec(&[proof_carry]).expect("commitment");
        let expected_hash = shasta_aggregation_output(
            &commitment,
            1,
            Address::ZERO,
            Address::from_str(EXPECTED_ADDR).unwrap(),
        );

        let proof_hex = proof.proof.expect("missing proof");
        let bytes = decode_proof_bytes(&proof_hex);
        assert_eq!(&bytes[..4], EXPECTED_INSTANCE_ID.to_be_bytes());
        assert_eq!(proof.input.expect("missing input"), expected_hash);

        let sig: [u8; 65] = bytes[24..].try_into().expect("sig bytes");
        let recovered = recover_address(&sig, expected_hash);
        let expected_addr = Address::from_str(EXPECTED_ADDR).unwrap();
        assert_eq!(recovered, expected_addr);
    }
}
