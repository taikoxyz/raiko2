use std::collections::BTreeMap;

use alloy_primitives::{B256, hex};
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
struct BeaconBlobsResponse {
    data: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedBeaconBlob {
    blob: Vec<u8>,
    commitment: KzgCommitmentBytes,
}

fn decode_hex_bytes(value: &str, label: &str) -> RaikoResult<Vec<u8>> {
    hex::decode(value.trim_start_matches("0x")).map_err(|err| {
        RaikoError::RPC(format!(
            "failed to decode beacon {label} hex payload: {err}"
        ))
    })
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

fn beacon_blobs_url(
    beacon_rpc: &str,
    slot: u64,
    versioned_hashes: &[B256],
) -> RaikoResult<reqwest::Url> {
    let mut url = reqwest::Url::parse(beacon_rpc).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("invalid beacon_rpc URL: {err}"))
    })?;
    url.set_query(None);
    url.set_fragment(None);

    let slot = slot.to_string();
    let mut segments = url.path_segments_mut().map_err(|()| {
        RaikoError::InvalidRequestConfig(
            "beacon_rpc URL cannot be used as a hierarchical path base".to_string(),
        )
    })?;
    segments.pop_if_empty();
    segments.extend(["eth", "v1", "beacon", "blobs", &slot]);
    drop(segments);

    for versioned_hash in versioned_hashes {
        url.query_pairs_mut()
            .append_pair("versioned_hashes", &versioned_hash.to_string());
    }
    Ok(url)
}

fn beacon_blobs_compatibility_hint(slot: u64) -> String {
    format!(
        "verify beacon_rpc supports GET /eth/v1/beacon/blobs/{slot}?versioned_hashes=... and, when using Prysm, deploy Prysm >= 7.1.8"
    )
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

    async fn fetch_beacon_blobs(
        &self,
        beacon_rpc: &str,
        slot: u64,
        versioned_hashes: &[B256],
    ) -> RaikoResult<BTreeMap<B256, ResolvedBeaconBlob>> {
        let url = beacon_blobs_url(beacon_rpc, slot, versioned_hashes)?;
        let endpoint_route = format!("/eth/v1/beacon/blobs/{slot}");
        let compatibility_hint = beacon_blobs_compatibility_hint(slot);
        let response = self
            .http_client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|err| {
                let err = err.without_url();
                RaikoError::RPC(format!(
                    "failed to fetch beacon blobs for slot {slot} with GET {endpoint_route}: {err}; {compatibility_hint}"
                ))
            })?;

        if !response.status().is_success() {
            return Err(RaikoError::RPC(format!(
                "beacon blobs request for slot {slot} with GET {endpoint_route} failed with {}; {compatibility_hint}",
                response.status(),
            )));
        }

        let payload = response
            .json::<BeaconBlobsResponse>()
            .await
            .map_err(|err| {
                let err = err.without_url();
                RaikoError::RPC(format!(
                    "failed to decode beacon blobs response for slot {slot} from GET {endpoint_route}: {err}; {compatibility_hint}"
                ))
            })?;
        if payload.data.is_empty() {
            return Err(RaikoError::RPC(format!(
                "beacon blobs response for slot {slot} was empty from GET {endpoint_route}; {compatibility_hint}"
            )));
        }

        let mut blobs_by_hash = BTreeMap::new();
        for (response_idx, encoded_blob) in payload.data.iter().enumerate() {
            let blob = decode_hex_bytes(encoded_blob, "blob").map_err(|err| {
                RaikoError::Preflight(format!(
                    "malformed beacon blob at response index {response_idx} for slot {slot} from GET {endpoint_route}: {err}; {compatibility_hint}"
                ))
            })?;
            let commitment = blob_to_commitment(&blob).map_err(|err| {
                RaikoError::Preflight(format!(
                    "malformed beacon blob at response index {response_idx} for slot {slot} from GET {endpoint_route}: failed to compute KZG commitment: {err}; {compatibility_hint}"
                ))
            })?;
            let versioned_hash = commitment_to_version_hash(&commitment);
            if versioned_hashes.contains(&versioned_hash) {
                blobs_by_hash
                    .entry(versioned_hash)
                    .or_insert(ResolvedBeaconBlob { blob, commitment });
            }
        }
        Ok(blobs_by_hash)
    }

    pub(crate) async fn fetch_shasta_data_sources(
        &self,
        l1_chain_spec: &ChainSpec,
        proposal_event: &ShastaEventData,
        blob_proof_type: BlobProofType,
    ) -> RaikoResult<Vec<InputDataSource>> {
        let beacon_rpc = l1_chain_spec.beacon_rpc.as_deref().ok_or_else(|| {
            RaikoError::InvalidRequestConfig(format!(
                "l1 chain {} is missing beacon_rpc; cannot resolve Shasta blobs",
                l1_chain_spec.name
            ))
        })?;
        let proposal = &proposal_event.proposal;
        if proposal.sources.is_empty() {
            return Ok(Vec::new());
        }

        let mut slots_by_source = Vec::with_capacity(proposal.sources.len());
        let mut requested_hashes_by_slot = BTreeMap::<u64, Vec<B256>>::new();
        for (source_idx, source) in proposal.sources.iter().enumerate() {
            let blob_hashes = source.blobSlice.blobHashes.as_slice();
            if blob_hashes.is_empty() {
                return Err(RaikoError::Preflight(format!(
                    "Shasta source {source_idx} has no blob hashes; calldata sources are not supported"
                )));
            }

            let slot = timestamp_to_slot(source_timestamp(source), l1_chain_spec)?;
            slots_by_source.push(slot);
            let requested_hashes = requested_hashes_by_slot.entry(slot).or_default();
            for versioned_hash in blob_hashes {
                if !requested_hashes.contains(versioned_hash) {
                    requested_hashes.push(*versioned_hash);
                }
            }
        }

        let mut blobs_by_slot = BTreeMap::new();
        for (slot, versioned_hashes) in &requested_hashes_by_slot {
            blobs_by_slot.insert(
                *slot,
                self.fetch_beacon_blobs(beacon_rpc, *slot, versioned_hashes)
                    .await?,
            );
        }

        let mut data_sources = Vec::with_capacity(proposal.sources.len());
        for (source_idx, source) in proposal.sources.iter().enumerate() {
            let slot = slots_by_source[source_idx];
            let blobs_by_hash = blobs_by_slot.get(&slot).ok_or_else(|| {
                RaikoError::RPC(format!("missing fetched beacon blobs for slot {slot}"))
            })?;
            let blob_hashes = source.blobSlice.blobHashes.as_slice();

            let mut tx_data_from_blob = Vec::with_capacity(blob_hashes.len());
            let mut blob_commitments = Vec::with_capacity(blob_hashes.len());
            let mut blob_proofs = Vec::with_capacity(blob_hashes.len());
            for (blob_idx, expected_hash) in blob_hashes.iter().enumerate() {
                let resolved = blobs_by_hash.get(expected_hash).ok_or_else(|| {
                    RaikoError::Preflight(format!(
                        "beacon blobs response for slot {slot} is missing requested versioned hash {expected_hash} for source {source_idx}, blob {blob_idx} after recomputing returned KZG commitments; {}",
                        beacon_blobs_compatibility_hint(slot)
                    ))
                })?;
                let proof = Self::blob_proof_for_type(
                    &resolved.blob,
                    &resolved.commitment,
                    blob_proof_type,
                )
                .map_err(|err| {
                    RaikoError::Preflight(format!(
                        "failed to resolve blob proof for source {source_idx}, blob {blob_idx}: {err}"
                    ))
                })?;
                tx_data_from_blob.push(resolved.blob.clone());
                blob_commitments.push(resolved.commitment.to_vec());
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
    use alloy_primitives::{B256, hex};
    use raiko2_primitives::{ChainSpec, RaikoError, blob::util::blob_to_commitment};
    use raiko2_protocol::BlobProofType;
    use raiko2_protocol_shasta::shasta::{BlobSlice, DerivationSource, Proposal, ShastaEventData};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    use crate::network::NetworkProvider;

    const BLOB_SIZE: usize = 131_072;

    async fn serve_once(status: &'static str, body: String) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let (target_tx, target_rx) = oneshot::channel();

        tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("test request should connect");
            let mut request = Vec::new();
            loop {
                let mut buffer = [0u8; 1024];
                let read = stream
                    .read(&mut buffer)
                    .await
                    .expect("test request should be readable");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            let request = String::from_utf8(request).expect("test request should be UTF-8");
            let target = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("test request target")
                .to_string();
            let _ = target_tx.send(target);

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("test response should be writable");
        });

        (format!("http://{address}/beacon-api/proxy"), target_rx)
    }

    fn test_blob(byte: u8) -> (Vec<u8>, [u8; 48], B256) {
        let blob = vec![byte; BLOB_SIZE];
        let commitment = blob_to_commitment(&blob).expect("test blob should be canonical");
        let hash = raiko2_primitives::blob::util::commitment_to_version_hash(&commitment);
        (blob, commitment, hash)
    }

    fn blobs_response(blobs: &[&[u8]]) -> String {
        let data = blobs
            .iter()
            .map(|blob| format!("0x{}", hex::encode(blob)))
            .collect::<Vec<_>>();
        serde_json::json!({ "data": data }).to_string()
    }

    fn source(timestamp: u64, blob_hashes: Vec<B256>) -> DerivationSource {
        DerivationSource {
            blobSlice: BlobSlice {
                timestamp: timestamp.try_into().expect("timestamp fits uint48"),
                blobHashes: blob_hashes,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn proposal_event(sources: Vec<DerivationSource>) -> ShastaEventData {
        ShastaEventData {
            proposal: Proposal {
                sources,
                ..Default::default()
            },
        }
    }

    fn chain_spec(beacon_rpc: String) -> ChainSpec {
        ChainSpec {
            name: "test-beacon".to_string(),
            genesis_time: 100,
            seconds_per_slot: 12,
            beacon_rpc: Some(beacon_rpc),
            ..Default::default()
        }
    }

    fn provider() -> NetworkProvider {
        NetworkProvider::new("http://127.0.0.1:1").expect("test provider should build")
    }

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

    #[tokio::test]
    async fn fetches_unique_hashes_once_and_restores_source_order_and_duplicates() {
        let (first_blob, first_commitment, first_hash) = test_blob(0x11);
        let (second_blob, second_commitment, second_hash) = test_blob(0x22);
        // The beacon API is allowed to return block order instead of request order.
        let body = blobs_response(&[&first_blob, &second_blob]);
        let (beacon_rpc, target_rx) = serve_once("200 OK", body).await;
        let event = proposal_event(vec![
            source(124, vec![second_hash, first_hash, second_hash]),
            source(124, vec![first_hash]),
        ]);

        let data_sources = provider()
            .fetch_shasta_data_sources(
                &chain_spec(beacon_rpc),
                &event,
                BlobProofType::ProofOfEquivalence,
            )
            .await
            .expect("beacon blobs should resolve");

        assert_eq!(data_sources.len(), 2);
        assert_eq!(
            data_sources[0].tx_data_from_blob,
            vec![second_blob.clone(), first_blob.clone(), second_blob]
        );
        assert_eq!(
            data_sources[0].blob_commitments,
            vec![
                second_commitment.to_vec(),
                first_commitment.to_vec(),
                second_commitment.to_vec()
            ]
        );
        assert_eq!(data_sources[1].tx_data_from_blob, vec![first_blob]);
        assert_eq!(
            data_sources[1].blob_commitments,
            vec![first_commitment.to_vec()]
        );

        let target = target_rx.await.expect("beacon request target");
        let url = reqwest::Url::parse(&format!("http://beacon.test{target}"))
            .expect("request target should be a URL");
        assert_eq!(url.path(), "/beacon-api/proxy/eth/v1/beacon/blobs/2");
        let requested_hashes = url
            .query_pairs()
            .filter(|(key, _)| key == "versioned_hashes")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            requested_hashes,
            vec![second_hash.to_string(), first_hash.to_string()]
        );
    }

    #[tokio::test]
    async fn missing_requested_blob_reports_the_recomputed_hash_mismatch() {
        let (returned_blob, _, returned_hash) = test_blob(0x11);
        let requested_hash = B256::repeat_byte(0x42);
        assert_ne!(returned_hash, requested_hash);
        let (beacon_rpc, _) = serve_once("200 OK", blobs_response(&[&returned_blob])).await;
        let event = proposal_event(vec![source(124, vec![requested_hash])]);

        let err = provider()
            .fetch_shasta_data_sources(
                &chain_spec(beacon_rpc),
                &event,
                BlobProofType::ProofOfEquivalence,
            )
            .await
            .expect_err("a blob with the wrong recomputed hash should be rejected");
        assert!(matches!(err, RaikoError::Preflight(_)), "{err:?}");
        let message = err.to_string();

        assert!(
            message.contains("missing requested versioned hash"),
            "{message}"
        );
        assert!(message.contains(&requested_hash.to_string()), "{message}");
        assert!(message.contains("Prysm >= 7.1.8"), "{message}");
    }

    #[tokio::test]
    async fn empty_blob_response_remains_retryable() {
        let requested_hash = B256::repeat_byte(0x42);
        let (beacon_rpc, _) = serve_once("200 OK", blobs_response(&[])).await;
        let event = proposal_event(vec![source(124, vec![requested_hash])]);

        let err = provider()
            .fetch_shasta_data_sources(
                &chain_spec(beacon_rpc),
                &event,
                BlobProofType::ProofOfEquivalence,
            )
            .await
            .expect_err("an omitted requested blob should be rejected");
        assert!(matches!(err, RaikoError::RPC(_)), "{err:?}");
        let message = err.to_string();

        assert!(
            message.contains("response for slot 2 was empty"),
            "{message}"
        );
        assert!(message.contains("Prysm >= 7.1.8"), "{message}");
    }

    #[tokio::test]
    async fn malformed_blob_response_reports_the_slot_and_operational_hint() {
        let requested_hash = B256::repeat_byte(0x42);
        let (beacon_rpc, _) = serve_once("200 OK", r#"{"data":["0x0102"]}"#.to_string()).await;
        let event = proposal_event(vec![source(124, vec![requested_hash])]);

        let err = provider()
            .fetch_shasta_data_sources(
                &chain_spec(beacon_rpc),
                &event,
                BlobProofType::ProofOfEquivalence,
            )
            .await
            .expect_err("a malformed blob should be rejected");
        assert!(matches!(err, RaikoError::Preflight(_)), "{err:?}");
        let message = err.to_string();

        assert!(message.contains("malformed beacon blob"), "{message}");
        assert!(message.contains("slot 2"), "{message}");
        assert!(message.contains("Prysm >= 7.1.8"), "{message}");
    }

    #[tokio::test]
    async fn invalid_blob_hex_fails_fast() {
        let requested_hash = B256::repeat_byte(0x42);
        let (beacon_rpc, _) = serve_once("200 OK", r#"{"data":["0xzz"]}"#.to_string()).await;
        let event = proposal_event(vec![source(124, vec![requested_hash])]);

        let err = provider()
            .fetch_shasta_data_sources(
                &chain_spec(beacon_rpc),
                &event,
                BlobProofType::ProofOfEquivalence,
            )
            .await
            .expect_err("invalid blob hex should be rejected");

        assert!(matches!(err, RaikoError::Preflight(_)), "{err:?}");
        let message = err.to_string();
        assert!(message.contains("malformed beacon blob"), "{message}");
        assert!(message.contains("slot 2"), "{message}");
        assert!(message.contains("Prysm >= 7.1.8"), "{message}");
    }

    #[tokio::test]
    async fn malformed_json_response_reports_the_slot_and_operational_hint() {
        let requested_hash = B256::repeat_byte(0x42);
        let (beacon_rpc, _) = serve_once("200 OK", r#"{"data":[null]}"#.to_string()).await;
        let event = proposal_event(vec![source(124, vec![requested_hash])]);

        let err = provider()
            .fetch_shasta_data_sources(
                &chain_spec(beacon_rpc),
                &event,
                BlobProofType::ProofOfEquivalence,
            )
            .await
            .expect_err("malformed JSON should be rejected");
        let message = err.to_string();

        assert!(
            message.contains("failed to decode beacon blobs response"),
            "{message}"
        );
        assert!(message.contains("slot 2"), "{message}");
        assert!(message.contains("Prysm >= 7.1.8"), "{message}");
    }

    #[tokio::test]
    async fn non_success_response_reports_status_endpoint_and_operational_hint() {
        let requested_hash = B256::repeat_byte(0x42);
        let (beacon_rpc, _) = serve_once("503 Service Unavailable", String::new()).await;
        let event = proposal_event(vec![source(124, vec![requested_hash])]);

        let err = provider()
            .fetch_shasta_data_sources(
                &chain_spec(beacon_rpc),
                &event,
                BlobProofType::ProofOfEquivalence,
            )
            .await
            .expect_err("a non-success response should be rejected");
        let message = err.to_string();

        assert!(message.contains("503 Service Unavailable"), "{message}");
        assert!(message.contains("/eth/v1/beacon/blobs/2"), "{message}");
        assert!(message.contains("Prysm >= 7.1.8"), "{message}");
    }
}
