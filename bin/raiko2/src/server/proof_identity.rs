use alloy_primitives::{Address, hex};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::Proof;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[cfg(test)]
use std::sync::LazyLock;

use crate::server::proof_artifact::ProofArtifactPayload;
#[cfg(feature = "host")]
use alloy_primitives::B256;

const SGX_PROOF_LEN: usize = 89;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SgxInstanceIdentity {
    pub(crate) id: u32,
    pub(crate) address: Address,
}

#[cfg(feature = "host")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZkProofIdentity {
    Risc0 {
        proposal_image_id: B256,
        aggregation_image_id: B256,
        allow_mock_aggregate: bool,
    },
    Sp1 {
        proposal_vkey_digest: B256,
        aggregation_vkey_digest: B256,
        aggregation_contract_vkey: B256,
    },
}

#[cfg(feature = "host")]
impl ZkProofIdentity {
    pub(crate) const fn risc0(proposal_image_id: B256, aggregation_image_id: B256) -> Self {
        Self::Risc0 {
            proposal_image_id,
            aggregation_image_id,
            allow_mock_aggregate: false,
        }
    }

    pub(crate) const fn risc0_mock(proposal_image_id: B256, aggregation_image_id: B256) -> Self {
        Self::Risc0 {
            proposal_image_id,
            aggregation_image_id,
            allow_mock_aggregate: true,
        }
    }

    pub(crate) const fn sp1(
        proposal_vkey_digest: B256,
        aggregation_vkey_digest: B256,
        aggregation_contract_vkey: B256,
    ) -> Self {
        Self::Sp1 {
            proposal_vkey_digest,
            aggregation_vkey_digest,
            aggregation_contract_vkey,
        }
    }

    /// Checks a proposal proof before it is reused as an aggregation sub-proof.
    ///
    /// This intentionally uses the proposal guest identity. The aggregation guest identity is
    /// relevant only to a final aggregate artifact.
    pub(crate) fn matches_proposal_subproof(self, proof: &Proof) -> Result<bool, String> {
        match self {
            Self::Risc0 {
                proposal_image_id, ..
            } => Ok(risc0_image_id_from_proof(proof)? == proposal_image_id),
            Self::Sp1 {
                proposal_vkey_digest,
                ..
            } => Ok(sp1_vkey_digest_from_proof(proof)? == proposal_vkey_digest),
        }
    }

    /// Checks a cached final aggregate artifact.
    ///
    /// The final artifact must identify both the aggregation guest that produced it and the
    /// proposal guest expected by that aggregation guest.
    pub(crate) fn matches_cached_final_aggregate(self, proof: &Proof) -> Result<bool, String> {
        match self {
            Self::Risc0 {
                proposal_image_id,
                aggregation_image_id,
                allow_mock_aggregate,
            } => {
                if risc0_image_id_from_proof(proof)? != aggregation_image_id {
                    return Ok(false);
                }
                if allow_mock_aggregate && is_risc0_mock_aggregate(proof) {
                    return Ok(risc0_mock_aggregate_program_ids(proof)
                        == Some((proposal_image_id, aggregation_image_id)));
                }
                let (actual_proposal, actual_aggregation) =
                    risc0_aggregate_program_ids_from_proof(proof)?;
                Ok(actual_proposal == proposal_image_id
                    && actual_aggregation == aggregation_image_id)
            }
            Self::Sp1 {
                proposal_vkey_digest,
                aggregation_vkey_digest,
                aggregation_contract_vkey,
            } => {
                let (actual_aggregation, actual_proposal) =
                    sp1_aggregate_program_ids_from_proof(proof)?;
                Ok(actual_proposal == proposal_vkey_digest
                    && actual_aggregation == aggregation_contract_vkey
                    && sp1_vkey_digest_from_proof(proof)? == aggregation_vkey_digest)
            }
        }
    }
}

#[derive(Clone)]
enum ProofIdentity {
    RemoteSgx(RemoteSgxIdentity),
    #[cfg(feature = "host")]
    Zk(ZkProofIdentity),
}

/// Active proof identities reconstructed from the host's configured lanes and local guests.
#[derive(Default)]
pub(crate) struct ProofIdentityRegistry {
    identities: HashMap<PipelineKey, ProofIdentity>,
}

impl ProofIdentityRegistry {
    #[cfg(test)]
    pub(crate) fn empty() -> &'static Self {
        static EMPTY: LazyLock<ProofIdentityRegistry> =
            LazyLock::new(ProofIdentityRegistry::default);
        &EMPTY
    }

    pub(crate) fn insert_remote_sgx(
        &mut self,
        pipeline_key: PipelineKey,
        identity: RemoteSgxIdentity,
    ) {
        self.identities
            .insert(pipeline_key, ProofIdentity::RemoteSgx(identity));
    }

    #[cfg(feature = "host")]
    pub(crate) fn insert_zk(&mut self, pipeline_key: PipelineKey, identity: ZkProofIdentity) {
        self.identities
            .insert(pipeline_key, ProofIdentity::Zk(identity));
    }

    pub(crate) fn remote_sgx(&self, pipeline_key: PipelineKey) -> Option<RemoteSgxIdentity> {
        match self.identities.get(&pipeline_key) {
            Some(ProofIdentity::RemoteSgx(identity)) => Some(identity.clone()),
            #[cfg(feature = "host")]
            Some(ProofIdentity::Zk(_)) | None => None,
            #[cfg(not(feature = "host"))]
            None => None,
        }
    }

    /// Validates direct external aggregation inputs without teaching an unknown remote lane.
    ///
    /// ZK inputs retain their backend-native validation. SGX inputs must all identify the same
    /// remote instance and, when configured, match that lane's expected instance.
    pub(crate) fn validate_external_aggregate_inputs(
        &self,
        pipeline_key: PipelineKey,
        proofs: &[Proof],
    ) -> Result<(), String> {
        match self.identities.get(&pipeline_key) {
            Some(ProofIdentity::RemoteSgx(identity)) => {
                identity.validate_external_aggregate_inputs(proofs)
            }
            #[cfg(feature = "host")]
            Some(ProofIdentity::Zk(_)) | None => Ok(()),
            #[cfg(not(feature = "host"))]
            None => Ok(()),
        }
    }

    /// Rejects a newly returned remote proof that does not match an immutable configured or
    /// already learned identity. This does not mutate process state.
    pub(crate) fn validate_new_remote_sgx_proof(
        &self,
        pipeline_key: PipelineKey,
        proof: &Proof,
    ) -> Result<(), String> {
        match self.identities.get(&pipeline_key) {
            Some(ProofIdentity::RemoteSgx(identity)) => identity.validate_new(proof),
            #[cfg(feature = "host")]
            Some(ProofIdentity::Zk(_)) | None => Ok(()),
            #[cfg(not(feature = "host"))]
            None => Ok(()),
        }
    }

    /// Learns a remote identity only after a canonical proof artifact has fully finalized.
    pub(crate) fn learn_remote_sgx_after_finalization(
        &self,
        pipeline_key: PipelineKey,
        proof: &Proof,
    ) -> Result<(), String> {
        match self.identities.get(&pipeline_key) {
            Some(ProofIdentity::RemoteSgx(identity)) => {
                identity.learn_after_finalization_from_proof(proof)
            }
            #[cfg(feature = "host")]
            Some(ProofIdentity::Zk(_)) | None => Ok(()),
            #[cfg(not(feature = "host"))]
            None => Ok(()),
        }
    }

    /// Returns whether a completed cache artifact is compatible with this host.
    ///
    /// Proposal artifacts are aggregation sub-proofs and use proposal guest identity. Final
    /// artifacts use aggregation guest identity plus their encoded proposal identity. External
    /// aggregation inputs remain backend-validated request data, so this gate intentionally skips
    /// them.
    pub(crate) fn matches_cached_artifact(
        &self,
        pipeline_key: PipelineKey,
        payload: ProofArtifactPayload,
        proof: &Proof,
    ) -> Result<bool, String> {
        let Some(identity) = self.identities.get(&pipeline_key) else {
            return Ok(true);
        };
        match identity {
            ProofIdentity::RemoteSgx(identity) => match payload {
                ProofArtifactPayload::Proposal | ProofArtifactPayload::Final => {
                    identity.matches_cached(proof)
                }
                ProofArtifactPayload::AggregateInput => Ok(true),
            },
            #[cfg(feature = "host")]
            ProofIdentity::Zk(identity) => match payload {
                ProofArtifactPayload::Proposal => identity.matches_proposal_subproof(proof),
                ProofArtifactPayload::Final => identity.matches_cached_final_aggregate(proof),
                ProofArtifactPayload::AggregateInput => Ok(true),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteSgxIdentity {
    expected: Arc<RwLock<Option<SgxInstanceIdentity>>>,
    finalization: Arc<tokio::sync::Mutex<()>>,
}

impl RemoteSgxIdentity {
    pub(crate) fn unknown() -> Self {
        Self {
            expected: Arc::new(RwLock::new(None)),
            finalization: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) fn configured(expected: SgxInstanceIdentity) -> Self {
        Self {
            expected: Arc::new(RwLock::new(Some(expected))),
            finalization: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) fn expected(&self) -> Result<Option<SgxInstanceIdentity>, String> {
        self.expected
            .read()
            .map(|expected| *expected)
            .map_err(|_| "remote SGX identity lock is poisoned".to_string())
    }

    pub(crate) async fn lock_finalization(&self) -> tokio::sync::OwnedMutexGuard<()> {
        Arc::clone(&self.finalization).lock_owned().await
    }

    #[cfg(test)]
    pub(crate) fn accepts_new(&self, proof: &Proof) -> Result<bool, String> {
        let actual = Self::required_identity(proof)?;
        Ok(self.expected()?.is_none_or(|expected| expected == actual))
    }

    pub(crate) fn validate_new(&self, proof: &Proof) -> Result<(), String> {
        let actual = Self::required_identity(proof)?;
        if let Some(expected) = self.expected()?
            && expected != actual
        {
            return Err(Self::mismatch_message(expected, actual));
        }
        Ok(())
    }

    pub(crate) fn validate_external_aggregate_inputs(
        &self,
        proofs: &[Proof],
    ) -> Result<(), String> {
        let expected = self.expected()?;
        let mut first: Option<SgxInstanceIdentity> = None;
        for (index, proof) in proofs.iter().enumerate() {
            let actual = Self::required_identity(proof)
                .map_err(|error| format!("aggregate proof {index}: {error}"))?;
            if let Some(expected) = expected
                && expected != actual
            {
                return Err(format!(
                    "aggregate proof {index}: {}",
                    Self::mismatch_message(expected, actual)
                ));
            }
            if let Some(first) = first {
                if first != actual {
                    return Err(format!(
                        "aggregate proof {index}: remote SGX instance mismatch: first proof has id {} address {:#x}; got id {} address {:#x}",
                        first.id, first.address, actual.id, actual.address,
                    ));
                }
            } else {
                first = Some(actual);
            }
        }
        Ok(())
    }

    pub(crate) fn matches_cached(&self, proof: &Proof) -> Result<bool, String> {
        Ok(self
            .expected()?
            .zip(remote_sgx_identity_from_proof(proof)?)
            .is_some_and(|(expected, actual)| expected == actual))
    }

    pub(crate) fn learn_after_finalization(
        &self,
        actual: SgxInstanceIdentity,
    ) -> Result<(), String> {
        let mut expected = self
            .expected
            .write()
            .map_err(|_| "remote SGX identity lock is poisoned".to_string())?;
        match *expected {
            Some(current) if current != actual => Err(format!(
                "remote SGX instance mismatch: expected id {} address {:#x}; got id {} address {:#x}",
                current.id, current.address, actual.id, actual.address,
            )),
            Some(_) => Ok(()),
            None => {
                *expected = Some(actual);
                Ok(())
            }
        }
    }

    pub(crate) fn learn_after_finalization_from_proof(&self, proof: &Proof) -> Result<(), String> {
        self.learn_after_finalization(Self::required_identity(proof)?)
    }

    fn required_identity(proof: &Proof) -> Result<SgxInstanceIdentity, String> {
        remote_sgx_identity_from_proof(proof)?
            .ok_or_else(|| "remote SGX proof is missing an instance header".to_string())
    }

    fn mismatch_message(expected: SgxInstanceIdentity, actual: SgxInstanceIdentity) -> String {
        format!(
            "remote SGX instance mismatch: expected id {} address {:#x}; got id {} address {:#x}",
            expected.id, expected.address, actual.id, actual.address,
        )
    }
}

pub(crate) fn remote_sgx_identity_from_proof(
    proof: &Proof,
) -> Result<Option<SgxInstanceIdentity>, String> {
    let Some(raw) = proof.proof.as_deref() else {
        return Ok(None);
    };
    let bytes = hex::decode(raw.trim_start_matches("0x"))
        .map_err(|error| format!("invalid remote SGX proof encoding: {error}"))?;
    if bytes.len() != SGX_PROOF_LEN {
        return Err(format!(
            "invalid remote SGX proof length: got {} expected {SGX_PROOF_LEN}",
            bytes.len()
        ));
    }
    Ok(Some(SgxInstanceIdentity {
        id: u32::from_be_bytes(bytes[..4].try_into().expect("four-byte SGX instance id")),
        address: Address::from_slice(&bytes[4..24]),
    }))
}

#[cfg(feature = "host")]
fn risc0_image_id_from_proof(proof: &Proof) -> Result<B256, String> {
    parse_b256_uuid(proof.uuid.as_deref(), "RISC0 image id")
}

#[cfg(feature = "host")]
fn sp1_vkey_digest_from_proof(proof: &Proof) -> Result<B256, String> {
    let uuid = proof
        .uuid
        .as_deref()
        .ok_or_else(|| "SP1 proof is missing verifying-key metadata".to_string())?;
    raiko2_prover::sp1::sp1_vk_digest_from_uuid(uuid)
}

#[cfg(feature = "host")]
fn risc0_aggregate_program_ids_from_proof(proof: &Proof) -> Result<(B256, B256), String> {
    let bytes = aggregate_proof_prefix(proof, "RISC0")?;
    let dynamic_offset = abi_word_as_usize(
        bytes
            .get(..32)
            .ok_or_else(|| "invalid RISC0 aggregate proof head".to_string())?,
        "seal offset",
    )?;
    if dynamic_offset != 96 {
        return Err(format!(
            "invalid RISC0 aggregate proof seal offset: got {dynamic_offset} expected 96"
        ));
    }
    let seal_length_end = dynamic_offset
        .checked_add(32)
        .ok_or_else(|| "invalid RISC0 aggregate proof seal offset overflow".to_string())?;
    let seal_length = abi_word_as_usize(
        bytes
            .get(dynamic_offset..seal_length_end)
            .ok_or_else(|| "invalid RISC0 aggregate proof seal length".to_string())?,
        "seal length",
    )?;
    let padded_seal_length = seal_length
        .checked_add(31)
        .map(|length| length / 32 * 32)
        .ok_or_else(|| "invalid RISC0 aggregate proof seal length overflow".to_string())?;
    let expected_length = seal_length_end
        .checked_add(padded_seal_length)
        .ok_or_else(|| "invalid RISC0 aggregate proof length overflow".to_string())?;
    if bytes.len() != expected_length {
        return Err(format!(
            "invalid RISC0 aggregate proof length: got {} expected {expected_length}",
            bytes.len()
        ));
    }
    Ok((
        B256::from_slice(&bytes[32..64]),
        B256::from_slice(&bytes[64..96]),
    ))
}

#[cfg(feature = "host")]
fn abi_word_as_usize(word: &[u8], field: &str) -> Result<usize, String> {
    if word.len() != 32 {
        return Err(format!(
            "invalid RISC0 aggregate proof {field}: expected a 32-byte ABI word"
        ));
    }
    if word[..24].iter().any(|byte| *byte != 0) {
        return Err(format!(
            "invalid RISC0 aggregate proof {field}: value does not fit usize"
        ));
    }
    let value = u64::from_be_bytes(
        word[24..]
            .try_into()
            .expect("the final ABI word bytes always have length eight"),
    );
    usize::try_from(value)
        .map_err(|_| format!("invalid RISC0 aggregate proof {field}: value does not fit usize"))
}

#[cfg(feature = "host")]
fn is_risc0_mock_aggregate(proof: &Proof) -> bool {
    let Some(metadata) = proof
        .extra_data
        .as_ref()
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    metadata.get("zkvm").and_then(serde_json::Value::as_str) == Some("risc0")
        && metadata.get("mode").and_then(serde_json::Value::as_str) == Some("mock")
        && metadata
            .get("fake_receipt")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

#[cfg(feature = "host")]
fn risc0_mock_aggregate_program_ids(proof: &Proof) -> Option<(B256, B256)> {
    let metadata = proof
        .extra_data
        .as_ref()
        .and_then(serde_json::Value::as_object)?;
    let proposal_image_id = metadata
        .get("proposal_image_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|image_id| image_id.parse::<B256>().ok())?;
    let aggregation_image_id = metadata
        .get("image_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|image_id| image_id.parse::<B256>().ok())?;
    Some((proposal_image_id, aggregation_image_id))
}

#[cfg(feature = "host")]
fn sp1_aggregate_program_ids_from_proof(proof: &Proof) -> Result<(B256, B256), String> {
    let bytes = aggregate_proof_prefix(proof, "SP1")?;
    Ok((
        B256::from_slice(&bytes[..32]),
        B256::from_slice(&bytes[32..64]),
    ))
}

#[cfg(feature = "host")]
fn aggregate_proof_prefix(proof: &Proof, backend: &str) -> Result<Vec<u8>, String> {
    let raw = proof
        .proof
        .as_deref()
        .ok_or_else(|| format!("{backend} aggregate proof is missing payload"))?;
    let bytes = hex::decode(raw.trim_start_matches("0x"))
        .map_err(|error| format!("invalid {backend} aggregate proof encoding: {error}"))?;
    if bytes.len() < 64 {
        return Err(format!(
            "invalid {backend} aggregate proof length: got {} expected at least 64",
            bytes.len()
        ));
    }
    Ok(bytes)
}

#[cfg(feature = "host")]
fn parse_b256_uuid(raw: Option<&str>, name: &str) -> Result<B256, String> {
    let raw = raw.ok_or_else(|| format!("proof is missing {name}"))?;
    raw.parse::<B256>()
        .map_err(|error| format!("invalid {name}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        ProofIdentityRegistry, RemoteSgxIdentity, SgxInstanceIdentity,
        remote_sgx_identity_from_proof,
    };
    use alloy_primitives::{Address, hex};
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::Proof;
    use raiko2_primitives_shasta::encode_proof_carry_data;
    use raiko2_protocol_shasta::shasta::ProofCarryData;

    #[cfg(feature = "host")]
    use super::ZkProofIdentity;
    #[cfg(feature = "host")]
    use crate::server::proof_artifact::ProofArtifactPayload;
    #[cfg(feature = "host")]
    use alloy::sol_types::SolValue;
    #[cfg(feature = "host")]
    use alloy_primitives::B256;

    fn proof(id: u32, address: Address) -> Proof {
        let mut bytes = vec![0_u8; 89];
        bytes[..4].copy_from_slice(&id.to_be_bytes());
        bytes[4..24].copy_from_slice(address.as_slice());
        Proof {
            proof: Some(format!("0x{}", hex::encode(bytes))),
            ..Proof::default()
        }
    }

    #[cfg(feature = "host")]
    fn risc0_aggregation_seal_payload(
        seal: &[u8],
        proposal_image_id: B256,
        aggregation_image_id: B256,
    ) -> String {
        // This mirrors the RISC0 ABI payload encoder: remove the outer tuple offset, while
        // retaining the inner dynamic-bytes head before the two program identifiers.
        let encoded = (seal.to_vec(), proposal_image_id, aggregation_image_id).abi_encode();
        hex::encode_prefixed(&encoded[32..])
    }

    #[test]
    fn unknown_remote_sgx_identity_does_not_reuse_a_cached_proof() {
        let identity = RemoteSgxIdentity::unknown();
        let proof = proof(1, Address::repeat_byte(0x11));

        assert!(
            !identity
                .matches_cached(&proof)
                .expect("parse cached proof identity")
        );
    }

    #[test]
    fn remote_sgx_identity_learns_only_after_finalization_and_never_rotates() {
        let identity = RemoteSgxIdentity::unknown();
        let first = proof(1, Address::repeat_byte(0x11));
        let rotated = proof(2, Address::repeat_byte(0x22));

        let first_instance = remote_sgx_identity_from_proof(&first)
            .expect("parse first proof")
            .expect("first proof contains an SGX identity");
        assert!(identity.accepts_new(&first).expect("validate first proof"));
        assert!(
            identity
                .expected()
                .expect("read expected identity")
                .is_none()
        );

        identity
            .learn_after_finalization(first_instance)
            .expect("learn finalized proof identity");

        assert!(
            identity
                .matches_cached(&first)
                .expect("match learned cached proof")
        );
        assert!(
            !identity
                .accepts_new(&rotated)
                .expect("reject rotated proof without mutation")
        );
        assert_eq!(
            identity.expected().expect("read expected identity"),
            Some(first_instance)
        );
    }

    #[tokio::test]
    async fn remote_sgx_finalization_serializes_first_identity_learning() {
        let identity = RemoteSgxIdentity::unknown();
        let first = proof(1, Address::repeat_byte(0x11));
        let rotated = proof(2, Address::repeat_byte(0x22));
        let first_instance = remote_sgx_identity_from_proof(&first)
            .expect("parse first proof")
            .expect("first proof contains an SGX identity");

        let first_guard = identity.lock_finalization().await;
        let waiting_identity = identity.clone();
        let mut waiting = tokio::spawn(async move {
            let _guard = waiting_identity.lock_finalization().await;
            waiting_identity.accepts_new(&rotated)
        });

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut waiting)
                .await
                .is_err(),
            "a concurrent completion must wait for the first lane finalization"
        );
        identity
            .learn_after_finalization(first_instance)
            .expect("learn first finalized identity");
        drop(first_guard);

        assert!(
            !waiting
                .await
                .expect("second finalization task should finish")
                .expect("second proof should parse")
        );
    }

    #[test]
    fn external_sgx_aggregate_requires_one_expected_subproof_identity() {
        let first = proof(0, Address::repeat_byte(0x11));
        let same = proof(0, Address::repeat_byte(0x11));
        let rotated = proof(1, Address::repeat_byte(0x22));

        let mut unknown = ProofIdentityRegistry::default();
        unknown.insert_remote_sgx(PipelineKey::ShastaSgx, RemoteSgxIdentity::unknown());
        assert!(
            unknown
                .validate_external_aggregate_inputs(PipelineKey::ShastaSgx, &[first.clone(), same])
                .is_ok()
        );
        assert!(
            unknown
                .validate_external_aggregate_inputs(
                    PipelineKey::ShastaSgx,
                    &[first.clone(), rotated.clone()],
                )
                .is_err()
        );
        assert_eq!(
            unknown
                .remote_sgx(PipelineKey::ShastaSgx)
                .expect("remote SGX lane")
                .expected()
                .expect("read expected identity"),
            None,
            "external input must not teach an unknown lane"
        );

        let mut configured = ProofIdentityRegistry::default();
        configured.insert_remote_sgx(
            PipelineKey::ShastaSgx,
            RemoteSgxIdentity::configured(SgxInstanceIdentity {
                id: 0,
                address: Address::repeat_byte(0x11),
            }),
        );
        assert!(
            configured
                .validate_external_aggregate_inputs(PipelineKey::ShastaSgx, &[first])
                .is_ok()
        );
        assert!(
            configured
                .validate_external_aggregate_inputs(PipelineKey::ShastaSgx, &[rotated])
                .is_err()
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn risc0_cached_proposal_requires_the_current_local_image_id() {
        let identity = ZkProofIdentity::risc0(B256::repeat_byte(0x11), B256::repeat_byte(0x22));
        let matching = Proof {
            uuid: Some(format!("{:#x}", B256::repeat_byte(0x11))),
            proof: Some("0x01".to_string()),
            ..Proof::default()
        };
        let stale = Proof {
            uuid: Some(format!("{:#x}", B256::repeat_byte(0x33))),
            proof: Some("0x01".to_string()),
            ..Proof::default()
        };

        assert!(
            identity
                .matches_proposal_subproof(&matching)
                .expect("match current RISC0 proposal")
        );
        assert!(
            !identity
                .matches_proposal_subproof(&stale)
                .expect("reject stale RISC0 proposal")
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn sp1_cached_proposal_requires_the_current_local_verifying_key() {
        let identity = ZkProofIdentity::sp1(
            B256::repeat_byte(0x44),
            B256::repeat_byte(0x55),
            B256::repeat_byte(0x66),
        );
        let matching = Proof {
            uuid: Some(format!("{:#x}", B256::repeat_byte(0x44))),
            proof: Some("0x01".to_string()),
            ..Proof::default()
        };
        let stale = Proof {
            uuid: Some(format!("{:#x}", B256::repeat_byte(0x77))),
            proof: Some("0x01".to_string()),
            ..Proof::default()
        };

        assert!(
            identity
                .matches_proposal_subproof(&matching)
                .expect("match current SP1 proposal")
        );
        assert!(
            !identity
                .matches_proposal_subproof(&stale)
                .expect("reject stale SP1 proposal")
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn risc0_final_aggregate_requires_both_aggregate_and_subproof_image_ids() {
        let proposal_image_id = B256::repeat_byte(0x11);
        let aggregation_image_id = B256::repeat_byte(0x22);
        let identity = ZkProofIdentity::risc0(proposal_image_id, aggregation_image_id);
        for seal in [vec![0x99; 3], vec![0x99; 65]] {
            let matching = Proof {
                uuid: Some(format!("{aggregation_image_id:#x}")),
                proof: Some(risc0_aggregation_seal_payload(
                    &seal,
                    proposal_image_id,
                    aggregation_image_id,
                )),
                ..Proof::default()
            };
            let wrong_subproof = Proof {
                uuid: matching.uuid.clone(),
                proof: Some(risc0_aggregation_seal_payload(
                    &seal,
                    B256::repeat_byte(0x33),
                    aggregation_image_id,
                )),
                ..Proof::default()
            };
            let wrong_aggregation = Proof {
                uuid: matching.uuid.clone(),
                proof: Some(risc0_aggregation_seal_payload(
                    &seal,
                    proposal_image_id,
                    B256::repeat_byte(0x33),
                )),
                ..Proof::default()
            };

            assert!(
                identity
                    .matches_cached_final_aggregate(&matching)
                    .expect("match current RISC0 aggregate")
            );
            assert!(
                !identity
                    .matches_cached_final_aggregate(&wrong_subproof)
                    .expect("reject aggregate with a stale RISC0 subproof image")
            );
            assert!(
                !identity
                    .matches_cached_final_aggregate(&wrong_aggregation)
                    .expect("reject aggregate with a stale RISC0 aggregation image")
            );
        }
    }

    #[cfg(feature = "host")]
    #[test]
    fn risc0_mock_final_aggregate_requires_both_current_image_ids() {
        let proposal_image_id = B256::repeat_byte(0x11);
        let aggregation_image_id = B256::repeat_byte(0x22);
        let identity = ZkProofIdentity::risc0_mock(proposal_image_id, aggregation_image_id);
        let mock_aggregate = Proof {
            uuid: Some(format!("{aggregation_image_id:#x}")),
            // Fake receipts encode only the journal, so this deliberately lacks the seal suffix
            // that carries the proposal and aggregation program identifiers.
            proof: Some(format!("0x{}", hex::encode([0_u8; 64]))),
            extra_data: Some(serde_json::json!({
                "zkvm": "risc0",
                "mode": "mock",
                "fake_receipt": true,
                "image_id": format!("{aggregation_image_id:#x}"),
                "proposal_image_id": format!("{proposal_image_id:#x}"),
            })),
            ..Proof::default()
        };
        let wrong_image = Proof {
            uuid: Some(format!("{:#x}", B256::repeat_byte(0x33))),
            extra_data: Some(serde_json::json!({
                "zkvm": "risc0",
                "mode": "mock",
                "fake_receipt": true,
                "image_id": format!("{:#x}", B256::repeat_byte(0x33)),
                "proposal_image_id": format!("{proposal_image_id:#x}"),
            })),
            ..mock_aggregate.clone()
        };
        let wrong_subproof = Proof {
            extra_data: Some(serde_json::json!({
                "zkvm": "risc0",
                "mode": "mock",
                "fake_receipt": true,
                "image_id": format!("{aggregation_image_id:#x}"),
                "proposal_image_id": format!("{:#x}", B256::repeat_byte(0x33)),
            })),
            ..mock_aggregate.clone()
        };

        assert!(
            identity
                .matches_cached_final_aggregate(&mock_aggregate)
                .expect("accept the configured fake RISC0 aggregate")
        );
        assert!(
            !identity
                .matches_cached_final_aggregate(&wrong_image)
                .expect("reject a fake aggregate from another guest image")
        );
        assert!(
            !identity
                .matches_cached_final_aggregate(&wrong_subproof)
                .expect("reject a fake aggregate from another proposal guest image")
        );
        assert!(
            ZkProofIdentity::risc0(proposal_image_id, aggregation_image_id)
                .matches_cached_final_aggregate(&mock_aggregate)
                .is_err(),
            "non-mock RISC0 must reject a journal-only aggregate"
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn sp1_final_aggregate_requires_both_aggregate_and_subproof_verifying_keys() {
        let proposal_vkey_digest = B256::repeat_byte(0x44);
        let aggregation_vkey_digest = B256::repeat_byte(0x55);
        let aggregation_contract_vkey = B256::repeat_byte(0x66);
        let identity = ZkProofIdentity::sp1(
            proposal_vkey_digest,
            aggregation_vkey_digest,
            aggregation_contract_vkey,
        );
        let matching = Proof {
            uuid: Some(format!("{aggregation_vkey_digest:#x}")),
            proof: Some(format!(
                "0x{}{}",
                hex::encode(aggregation_contract_vkey),
                hex::encode(proposal_vkey_digest)
            )),
            ..Proof::default()
        };
        let wrong_subproof = Proof {
            uuid: matching.uuid.clone(),
            proof: Some(format!(
                "0x{}{}",
                hex::encode(aggregation_contract_vkey),
                hex::encode(B256::repeat_byte(0x77))
            )),
            ..Proof::default()
        };
        let wrong_aggregation = Proof {
            uuid: matching.uuid.clone(),
            proof: Some(format!(
                "0x{}{}",
                hex::encode(B256::repeat_byte(0x77)),
                hex::encode(proposal_vkey_digest)
            )),
            ..Proof::default()
        };

        assert!(
            identity
                .matches_cached_final_aggregate(&matching)
                .expect("match current SP1 aggregate")
        );
        assert!(
            !identity
                .matches_cached_final_aggregate(&wrong_subproof)
                .expect("reject aggregate with a stale SP1 subproof key")
        );
        assert!(
            !identity
                .matches_cached_final_aggregate(&wrong_aggregation)
                .expect("reject aggregate with a stale SP1 aggregation key")
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn cached_identity_gate_uses_proposal_identity_only_for_subproofs() {
        let proposal_image_id = B256::repeat_byte(0x11);
        let mut identities = ProofIdentityRegistry::default();
        identities.insert_zk(
            PipelineKey::ShastaRisc0Network,
            ZkProofIdentity::risc0(proposal_image_id, B256::repeat_byte(0x22)),
        );
        let matching_subproof = Proof {
            uuid: Some(format!("{proposal_image_id:#x}")),
            proof: Some("0x01".to_string()),
            ..Proof::default()
        };
        let stale_subproof = Proof {
            uuid: Some(format!("{:#x}", B256::repeat_byte(0x33))),
            proof: Some("0x01".to_string()),
            ..Proof::default()
        };
        let external_boundless_input = Proof {
            quote: Some("receipt-without-uuid".to_string()),
            extra_data: Some(
                encode_proof_carry_data(&ProofCarryData::default())
                    .expect("encode Boundless proof carry data"),
            ),
            ..Proof::default()
        };

        assert!(
            ProofArtifactPayload::AggregateInput
                .accepts(PipelineKey::ShastaRisc0Network, &external_boundless_input,)
        );

        assert!(
            identities
                .matches_cached_artifact(
                    PipelineKey::ShastaRisc0Network,
                    ProofArtifactPayload::Proposal,
                    &matching_subproof,
                )
                .expect("accept matching proposal subproof")
        );
        assert!(
            !identities
                .matches_cached_artifact(
                    PipelineKey::ShastaRisc0Network,
                    ProofArtifactPayload::Proposal,
                    &stale_subproof,
                )
                .expect("reject stale proposal subproof")
        );
        assert!(
            identities
                .matches_cached_artifact(
                    PipelineKey::ShastaRisc0Network,
                    ProofArtifactPayload::AggregateInput,
                    &external_boundless_input,
                )
                .expect("leave external Boundless input validation to the backend")
        );
    }
}
