use std::collections::BTreeMap;

use alloy_primitives::hex;
use raiko2_primitives::blob::util::{
    KzgCommitmentBytes, blob_to_commitment, blob_to_proof_of_equivalence,
    commitment_to_version_hash, verify_blob_proof_of_equivalence,
};
use raiko2_primitives::{ChainSpec, RaikoError, RaikoResult};
use raiko2_protocol::{BlobProofType, InputDataSource};
use raiko2_protocol_shasta::shasta::{DerivationSource, ShastaEventData};
use serde::Deserialize;

use super::NetworkProvider;

#[derive(Debug, Clone, Deserialize)]
struct BeaconBlobSidecar {
    blob: String,
    kzg_commitment: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BeaconBlobSidecarsResponse {
    data: Vec<BeaconBlobSidecar>,
}

fn decode_hex_bytes(value: &str, label: &str) -> RaikoResult<Vec<u8>> {
    hex::decode(value.trim_start_matches("0x")).map_err(|err| {
        RaikoError::RPC(format!(
            "failed to decode beacon {label} hex payload: {err}"
        ))
    })
}

fn decode_kzg_bytes(value: &str, label: &str) -> RaikoResult<KzgCommitmentBytes> {
    let bytes = decode_hex_bytes(value, label)?;
    if bytes.len() != 48 {
        return Err(RaikoError::RPC(format!(
            "invalid beacon {label} length: expected 48 bytes, got {}",
            bytes.len()
        )));
    }

    let mut output = [0u8; 48];
    output.copy_from_slice(&bytes);
    Ok(output)
}

fn timestamp_to_slot(timestamp: u64, chain_spec: &ChainSpec) -> RaikoResult<u64> {
    if chain_spec.seconds_per_slot == 0 {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "chain {} has invalid seconds_per_slot=0",
            chain_spec.name
        )));
    }
    if timestamp < chain_spec.genesis_time {
        return Err(RaikoError::Preflight(format!(
            "timestamp {timestamp} is before {} genesis_time {}",
            chain_spec.name, chain_spec.genesis_time
        )));
    }

    Ok((timestamp - chain_spec.genesis_time) / chain_spec.seconds_per_slot)
}

fn source_timestamp(source: &DerivationSource) -> u64 {
    source.blobSlice.timestamp.to::<u64>()
}

impl NetworkProvider {
    fn blob_proof_for_type(
        blob: &[u8],
        commitment: &KzgCommitmentBytes,
        _blob_proof_type: BlobProofType,
    ) -> RaikoResult<Vec<u8>> {
        let proof = blob_to_proof_of_equivalence(blob, commitment).map_err(|err| {
            RaikoError::Preflight(format!(
                "failed to compute proof-of-equivalence proof: {err}"
            ))
        })?;
        verify_blob_proof_of_equivalence(blob, commitment, &proof).map_err(|err| {
            RaikoError::Preflight(format!("invalid proof-of-equivalence proof: {err}"))
        })?;
        Ok(proof.to_vec())
    }

    async fn fetch_blob_sidecars(
        &self,
        beacon_rpc: &str,
        slot: u64,
    ) -> RaikoResult<Vec<BeaconBlobSidecar>> {
        let url = format!(
            "{}/eth/v1/beacon/blob_sidecars/{slot}",
            beacon_rpc.trim_end_matches('/')
        );
        let response = self.http_client.get(&url).send().await.map_err(|err| {
            RaikoError::RPC(format!(
                "failed to fetch beacon blob sidecars for slot {slot}: {err}"
            ))
        })?;

        if !response.status().is_success() {
            return Err(RaikoError::RPC(format!(
                "beacon blob sidecars request failed for slot {slot}: {}",
                response.status()
            )));
        }

        let payload = response
            .json::<BeaconBlobSidecarsResponse>()
            .await
            .map_err(|err| {
                RaikoError::RPC(format!(
                    "failed to decode beacon blob sidecars response for slot {slot}: {err}"
                ))
            })?;
        if payload.data.is_empty() {
            return Err(RaikoError::RPC(format!(
                "beacon blob sidecars response for slot {slot} was empty"
            )));
        }
        Ok(payload.data)
    }

    pub(crate) async fn fetch_proposal_data_sources(
        &self,
        l1_chain_spec: &ChainSpec,
        proposal_event: &ShastaEventData,
        blob_proof_type: BlobProofType,
    ) -> RaikoResult<Vec<InputDataSource>> {
        let beacon_rpc = l1_chain_spec.beacon_rpc.as_deref().ok_or_else(|| {
            RaikoError::InvalidRequestConfig(format!(
                "l1 chain {} is missing beacon_rpc; cannot resolve Shasta blob sidecars",
                l1_chain_spec.name
            ))
        })?;
        let proposal = &proposal_event.proposal;
        if proposal.sources.is_empty() {
            return Ok(Vec::new());
        }

        let mut sidecars_by_slot = BTreeMap::<u64, Vec<BeaconBlobSidecar>>::new();
        let mut data_sources = Vec::with_capacity(proposal.sources.len());
        for (source_idx, source) in proposal.sources.iter().enumerate() {
            let blob_hashes = source.blobSlice.blobHashes.as_slice();
            if blob_hashes.is_empty() {
                return Err(RaikoError::Preflight(format!(
                    "Shasta source {source_idx} has no blob hashes; calldata sources are not supported"
                )));
            }

            let slot = timestamp_to_slot(source_timestamp(source), l1_chain_spec)?;
            if let std::collections::btree_map::Entry::Vacant(entry) = sidecars_by_slot.entry(slot)
            {
                entry.insert(self.fetch_blob_sidecars(beacon_rpc, slot).await?);
            }
            let sidecars = sidecars_by_slot.get(&slot).ok_or_else(|| {
                RaikoError::RPC(format!("missing cached blob sidecars for slot {slot}"))
            })?;

            let mut tx_data_from_blob = Vec::with_capacity(blob_hashes.len());
            let mut blob_commitments = Vec::with_capacity(blob_hashes.len());
            let mut blob_proofs = Vec::with_capacity(blob_hashes.len());
            for (blob_idx, expected_hash) in blob_hashes.iter().enumerate() {
                let mut matched = None;
                for sidecar in sidecars {
                    let blob = decode_hex_bytes(&sidecar.blob, "blob")?;
                    let commitment = blob_to_commitment(&blob).map_err(|err| {
                        RaikoError::Preflight(format!(
                            "failed to compute blob commitment for source {source_idx}, blob {blob_idx}: {err}"
                        ))
                    })?;
                    if commitment_to_version_hash(&commitment) != *expected_hash {
                        continue;
                    }
                    let sidecar_commitment =
                        decode_kzg_bytes(&sidecar.kzg_commitment, "kzg_commitment")?;
                    if sidecar_commitment != commitment {
                        return Err(RaikoError::Preflight(format!(
                            "beacon commitment mismatch for source {source_idx}, blob {blob_idx}"
                        )));
                    }
                    let proof = Self::blob_proof_for_type(&blob, &commitment, blob_proof_type)
                        .map_err(|err| {
                            RaikoError::Preflight(format!(
                                "failed to resolve blob proof for source {source_idx}, blob {blob_idx}: {err}"
                            ))
                        })?;
                    matched = Some((blob, commitment.to_vec(), proof));
                    break;
                }

                let (blob, commitment, proof) = matched.ok_or_else(|| {
                    RaikoError::Preflight(format!(
                        "missing beacon blob sidecar for source {source_idx}, blob {blob_idx}, hash {expected_hash:?}"
                    ))
                })?;
                tx_data_from_blob.push(blob);
                blob_commitments.push(commitment);
                blob_proofs.push(proof);
            }

            data_sources.push(InputDataSource {
                tx_data_from_calldata: Vec::new(),
                tx_data_from_blob,
                blob_commitments,
                blob_proofs,
                is_forced_inclusion: source.isForcedInclusion,
            });
        }

        Ok(data_sources)
    }
}

#[cfg(test)]
mod tests {
    use super::{source_timestamp, timestamp_to_slot};
    use raiko2_primitives::ChainSpec;
    use raiko2_protocol_shasta::shasta::{BlobSlice, DerivationSource};

    fn derivation_source(is_forced_inclusion: bool, timestamp: u64) -> DerivationSource {
        DerivationSource {
            isForcedInclusion: is_forced_inclusion,
            blobSlice: BlobSlice {
                timestamp: timestamp.try_into().expect("timestamp fits uint48"),
                ..Default::default()
            },
        }
    }

    #[test]
    fn normal_source_uses_blob_slice_timestamp() {
        let source = derivation_source(false, 124);
        assert_eq!(source_timestamp(&source), 124);
    }

    #[test]
    fn forced_source_uses_blob_slice_timestamp() {
        let source = derivation_source(true, 136);
        assert_eq!(source_timestamp(&source), 136);
    }

    #[test]
    fn timestamp_to_slot_uses_genesis_offset() {
        let chain_spec = ChainSpec {
            name: "hoodi".to_string(),
            genesis_time: 100,
            seconds_per_slot: 12,
            ..Default::default()
        };

        let slot = timestamp_to_slot(124, &chain_spec).expect("slot");

        assert_eq!(slot, 2);
    }
}
