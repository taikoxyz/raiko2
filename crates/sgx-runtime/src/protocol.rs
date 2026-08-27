//! Shared SGX proof protocol helpers.

use alloy_primitives::{Address, B256};
use anyhow::{Context, Result};
use axum::{Json, http::StatusCode};
use raiko2_prover::remote_prover::protocol::{
    Raiko2ProofError, Raiko2ProofResponse, Raiko2ProofResult,
};
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

use crate::{bootstrap::public_key_to_address, tee::TeeProvider};

const SGX_PROOF_LEN: usize = 89;

/// Structured request failure mapped to the remote prover response envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestFailure {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl RequestFailure {
    pub(crate) fn invalid_json(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_JSON",
            message: message.into(),
        }
    }

    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_REQUEST",
            message: message.into(),
        }
    }

    pub(crate) fn prover_error(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "PROVER_ERROR",
            message: message.into(),
        }
    }

    pub(crate) fn into_response(self) -> (StatusCode, Json<Raiko2ProofResponse>) {
        (
            self.status,
            Json(Raiko2ProofResponse::error(Raiko2ProofError {
                code: self.code.to_string(),
                message: self.message,
            })),
        )
    }
}

impl std::fmt::Display for RequestFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RequestFailure {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SignerIdentity {
    pub(crate) instance_address: Address,
}

pub(crate) fn load_signer_identity<P: TeeProvider>(provider: &P) -> Result<SignerIdentity> {
    let secret_key = provider
        .load_private_key()
        .context("load SGX private key")?;
    let public_key = PublicKey::from_secret_key(&Secp256k1::new(), &secret_key);
    Ok(SignerIdentity {
        instance_address: public_key_to_address(&public_key),
    })
}

pub(crate) fn proof_result_from_input_hash<P: TeeProvider>(
    provider: &P,
    instance_id: u32,
    input_hash: B256,
) -> Result<Raiko2ProofResult> {
    let secret_key = provider
        .load_private_key()
        .context("load SGX private key")?;
    let public_key = PublicKey::from_secret_key(&Secp256k1::new(), &secret_key);
    let instance_address = public_key_to_address(&public_key);
    let quote = provider
        .load_quote(instance_address)
        .context("load SGX quote")?;
    let signature = sign_hash(&secret_key, input_hash)?;
    let proof = build_proposal_proof_bytes(instance_id, instance_address, signature);

    Ok(Raiko2ProofResult {
        proof: Some(prefixed_hex(&proof)),
        quote: (!quote.is_empty()).then(|| prefixed_hex(&quote)),
        public_key: Some(prefixed_hex(public_key.serialize_uncompressed())),
        instance_address: Some(format!("{instance_address:#x}")),
        input: format!("{input_hash:#x}"),
    })
}

fn sign_hash(secret_key: &SecretKey, hash: B256) -> Result<[u8; 65]> {
    let message = Message::from_digest_slice(hash.as_slice()).context("decode input hash")?;
    let secp = Secp256k1::new();
    let signature = secp.sign_ecdsa_recoverable(&message, secret_key);
    let (recovery_id, data) = signature.serialize_compact();
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(&data);
    sig_bytes[64] = u8::try_from(i32::from(recovery_id) + 27).context("encode recovery id")?;
    Ok(sig_bytes)
}

fn build_proposal_proof_bytes(
    instance_id: u32,
    instance_address: Address,
    signature: [u8; 65],
) -> Vec<u8> {
    let mut proof = Vec::with_capacity(SGX_PROOF_LEN);
    proof.extend(instance_id.to_be_bytes());
    proof.extend(instance_address);
    proof.extend(signature);
    proof
}

fn prefixed_hex<T: AsRef<[u8]>>(bytes: T) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256};
    use secp256k1::SecretKey;

    use super::proof_result_from_input_hash;
    use crate::tee::TeeProvider;

    #[derive(Clone)]
    struct FakeProvider {
        secret_key: SecretKey,
        quote: Vec<u8>,
    }

    impl TeeProvider for FakeProvider {
        fn save_private_key(&self, _key: &SecretKey) -> anyhow::Result<()> {
            unreachable!("unused in tests")
        }

        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            Ok(self.secret_key)
        }

        fn load_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            Ok(self.quote.clone())
        }
    }

    #[test]
    fn proof_result_contains_sgx_signature_quote_and_identity() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[7u8; 32]).expect("secret key"),
            quote: vec![0x12, 0x34, 0x56],
        };

        let result = proof_result_from_input_hash(&provider, 31337, B256::from([0x44; 32]))
            .expect("build proof result");

        let proof_hex = result.proof.expect("proof");
        let proof_bytes = hex::decode(proof_hex.trim_start_matches("0x")).expect("proof hex");
        assert_eq!(&proof_bytes[..4], &31337u32.to_be_bytes());
        assert_eq!(proof_bytes.len(), 89);
        assert_eq!(result.input, format!("{:#x}", B256::from([0x44; 32])));
        assert_eq!(result.quote.as_deref(), Some("0x123456"));
        assert!(
            result
                .public_key
                .as_deref()
                .is_some_and(|value| value.starts_with("0x04"))
        );
        assert!(
            result
                .instance_address
                .as_deref()
                .is_some_and(|value| value.starts_with("0x"))
        );
    }

    #[test]
    fn proof_result_omits_quote_when_provider_returns_empty_quote() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[7u8; 32]).expect("secret key"),
            quote: Vec::new(),
        };

        let result = proof_result_from_input_hash(&provider, 31337, B256::from([0x44; 32]))
            .expect("build proof result");

        assert_eq!(result.quote, None);
    }
}
