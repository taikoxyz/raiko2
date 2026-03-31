use std::collections::BTreeMap;

use alloy_primitives::hex;
use raiko2_primitives::blob::util::{
    KzgCommitmentBytes, blob_to_commitment, blob_to_proof, commitment_to_version_hash,
    verify_blob_kzg_proof,
};
use raiko2_primitives::{ChainSpec, RaikoError, RaikoResult};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_shasta::shasta::ShastaEventData;
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

impl NetworkProvider {
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

    pub(crate) async fn fetch_shasta_data_sources(
        &self,
        l1_chain_spec: &ChainSpec,
        proposal_event: &ShastaEventData,
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

            let timestamp = if source.isForcedInclusion {
                source.blobSlice.timestamp.to::<u64>()
            } else {
                proposal.timestamp.to::<u64>()
            };
            let slot = timestamp_to_slot(timestamp, l1_chain_spec)?;
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
                    let proof = blob_to_proof(&blob, &commitment).map_err(|err| {
                        RaikoError::Preflight(format!(
                            "failed to compute blob proof for source {source_idx}, blob {blob_idx}: {err}"
                        ))
                    })?;
                    verify_blob_kzg_proof(&blob, &commitment, &proof).map_err(|err| {
                        RaikoError::Preflight(format!(
                            "invalid beacon blob sidecar for source {source_idx}, blob {blob_idx}: {err}"
                        ))
                    })?;
                    matched = Some((blob, commitment.to_vec(), proof.to_vec()));
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
    use super::timestamp_to_slot;
    use raiko2_primitives::ChainSpec;

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
