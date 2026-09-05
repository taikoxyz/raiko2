use alloy::{eips::BlockNumberOrTag, rlp::decode_exact};
use alloy_primitives::{Address, B256, keccak256, map::AddressMap};
use alloy_rpc_types_eth::{EIP1186AccountProofResponse, EIP1186StorageProof};
use alloy_trie::TrieAccount;
use raiko2_primitives::{ExecutionWitness, RaikoError, RaikoResult, WitnessStateNode};
use risc0_ethereum_trie::Trie;
use std::time::Instant;
use tracing::info;

use crate::StorageProofTargets;

use super::RpcL2Provider;

const DEFAULT_ACCOUNT_PROOF_BATCH_SIZE: usize = 250;

type AccountProofRequest = (usize, u64, Address);
type StorageProofRequest = (usize, u64, u64, Address, Vec<B256>);

/// Decode account from EIP-1186 proof response
fn decode_account_from_proof(
    proof_response: &EIP1186AccountProofResponse,
) -> Result<TrieAccount, RaikoError> {
    let trie = Trie::from_rlp(&proof_response.account_proof)
        .map_err(|e| RaikoError::RPC(format!("Failed to decode account proof: {e}")))?;
    match trie.get(keccak256(proof_response.address)) {
        None => {
            // Account doesn't exist - return zero account
            Ok(TrieAccount {
                nonce: 0,
                balance: alloy_primitives::U256::ZERO,
                storage_root: alloy_primitives::B256::ZERO,
                code_hash: alloy_primitives::B256::ZERO,
            })
        }
        Some(rlp) => {
            let account: TrieAccount = decode_exact(rlp)
                .map_err(|e| RaikoError::RPC(format!("Failed to decode account RLP: {e}")))?;
            Ok(account)
        }
    }
}

fn dedup_addresses(addresses: &[Address]) -> Vec<Address> {
    let mut deduped = addresses.to_vec();
    deduped.sort_unstable();
    deduped.dedup();
    deduped
}

fn build_account_proof_requests(
    block_numbers: &[u64],
    addresses: &[Vec<Address>],
) -> RaikoResult<Vec<AccountProofRequest>> {
    if block_numbers.len() != addresses.len() {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "account proof block count ({}) does not match address list count ({})",
            block_numbers.len(),
            addresses.len()
        )));
    }

    let mut requests = Vec::new();

    for (block_idx, (block_number, addresses)) in block_numbers.iter().zip(addresses).enumerate() {
        let parent_block_number = block_number.checked_sub(1).ok_or_else(|| {
            RaikoError::RPC(format!(
                "cannot fetch pre-state signer accounts for genesis block {block_number}"
            ))
        })?;

        requests.extend(
            dedup_addresses(addresses)
                .into_iter()
                .map(|address| (block_idx, parent_block_number, address)),
        );
    }

    Ok(requests)
}

fn build_storage_proof_requests(
    block_numbers: &[u64],
    targets: &StorageProofTargets,
) -> RaikoResult<Vec<StorageProofRequest>> {
    if block_numbers.len() != targets.len() {
        return Err(RaikoError::Provider(format!(
            "storage proof target count ({}) does not match block count ({})",
            targets.len(),
            block_numbers.len()
        )));
    }

    let mut requests = Vec::new();
    for (block_idx, (block_number, block_targets)) in block_numbers.iter().zip(targets).enumerate()
    {
        let parent_block_number = block_number.checked_sub(1).ok_or_else(|| {
            RaikoError::Provider(format!(
                "cannot fetch parent storage proof for genesis block {block_number}"
            ))
        })?;
        for (address, storage_keys) in block_targets {
            if storage_keys.is_empty() {
                continue;
            }
            requests.push((
                block_idx,
                *block_number,
                parent_block_number,
                *address,
                storage_keys.clone(),
            ));
        }
    }

    Ok(requests)
}

/// Rejects an `eth_getProof` response that does not carry exactly one storage proof per requested
/// key. Absent (zero-valued) slots are the entries a non-conforming client or proxy is most likely
/// to drop, and a witness missing their exclusion proofs would only surface later as an unresolved
/// trie node inside guest validation instead of failing the preflight here.
fn ensure_storage_proof_covers_keys(
    address: Address,
    parent_block_number: u64,
    requested_keys: &[B256],
    storage_proofs: &[EIP1186StorageProof],
) -> RaikoResult<()> {
    let returned_keys = storage_proofs
        .iter()
        .map(|storage_proof| storage_proof.key.as_b256())
        .collect::<Vec<_>>();
    let missing_keys = requested_keys
        .iter()
        .filter(|key| !returned_keys.contains(key))
        .collect::<Vec<_>>();
    if storage_proofs.len() != requested_keys.len() || !missing_keys.is_empty() {
        return Err(RaikoError::RPC(format!(
            "storage eth_getProof for address {address} at parent block {parent_block_number} returned {} storage proofs for {} requested keys (missing {missing_keys:?})",
            storage_proofs.len(),
            requested_keys.len()
        )));
    }
    Ok(())
}

fn account_proof_batch_size() -> usize {
    std::env::var("ACCOUNT_PROOF_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_ACCOUNT_PROOF_BATCH_SIZE)
}

impl RpcL2Provider {
    async fn fetch_account_proofs(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<Vec<EIP1186AccountProofResponse>>> {
        let started_at = Instant::now();
        let mut result = vec![Vec::new(); block_numbers.len()];
        let requests = build_account_proof_requests(block_numbers, addresses)?;
        let batch_size = account_proof_batch_size();

        for chunk in requests.chunks(batch_size) {
            let mut batch = self.client.new_batch();
            let mut pending = Vec::with_capacity(chunk.len());

            for &(block_idx, parent_block_number, address) in chunk {
                pending.push((
                    block_idx,
                    parent_block_number,
                    address,
                    Box::pin(
                        batch
                            .add_call::<_, EIP1186AccountProofResponse>(
                                "eth_getProof",
                                &(
                                    address,
                                    Vec::<B256>::new(),
                                    BlockNumberOrTag::from(parent_block_number),
                                ),
                            )
                            .map_err(|_| {
                                RaikoError::RPC(
                                    "failed adding eth_getProof call to batch".to_owned(),
                                )
                            })?,
                    ),
                ));
            }

            batch.send().await.map_err(|e| {
                let blocks = chunk
                    .iter()
                    .map(|(_, parent_block_number, _)| parent_block_number + 1)
                    .collect::<Vec<_>>();
                RaikoError::RPC(format!(
                    "error sending eth_getProof batch for blocks {blocks:?}: {e}"
                ))
            })?;

            for (block_idx, parent_block_number, address, request) in pending {
                let proof = request.await.map_err(|e| {
                    RaikoError::RPC(format!(
                        "error collecting eth_getProof for address {address} at parent block {parent_block_number}: {e}"
                    ))
                })?;
                result[block_idx].push(proof);
            }
        }

        info!(
            block_count = block_numbers.len(),
            proof_requests = requests.len(),
            batch_size,
            elapsed_ms = started_at.elapsed().as_millis(),
            "fetched proposal-window account proofs"
        );
        Ok(result)
    }

    pub(super) async fn fetch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<TrieAccount>>> {
        let proofs = self.fetch_account_proofs(block_numbers, addresses).await?;
        let mut result = vec![AddressMap::default(); block_numbers.len()];
        for (block_idx, block_proofs) in proofs.iter().enumerate() {
            for proof in block_proofs {
                let account = decode_account_from_proof(proof)?;
                result[block_idx].insert(proof.address, account);
            }
        }
        Ok(result)
    }

    pub(super) async fn fetch_accounts_with_proof_witnesses(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<(Vec<AddressMap<TrieAccount>>, Vec<Vec<WitnessStateNode>>)> {
        let proofs = self.fetch_account_proofs(block_numbers, addresses).await?;
        let mut accounts = vec![AddressMap::default(); block_numbers.len()];
        let mut witness_nodes = vec![Vec::new(); block_numbers.len()];

        for (block_idx, block_proofs) in proofs.iter().enumerate() {
            for proof in block_proofs {
                let account = decode_account_from_proof(proof)?;
                accounts[block_idx].insert(proof.address, account);
                witness_nodes[block_idx].extend(
                    proof
                        .account_proof
                        .iter()
                        .cloned()
                        .map(WitnessStateNode::from_bytes),
                );
            }
            witness_nodes[block_idx] = ExecutionWitness::canonicalize_state_nodes(std::mem::take(
                &mut witness_nodes[block_idx],
            ));
        }

        Ok((accounts, witness_nodes))
    }

    pub(super) async fn fetch_storage_proof_witnesses(
        &self,
        block_numbers: &[u64],
        targets: &StorageProofTargets,
    ) -> RaikoResult<Vec<Vec<WitnessStateNode>>> {
        let requests = build_storage_proof_requests(block_numbers, targets)?;
        let mut result = vec![Vec::new(); block_numbers.len()];
        if requests.is_empty() {
            return Ok(result);
        }

        let started_at = Instant::now();
        let batch_size = account_proof_batch_size();
        for chunk in requests.chunks(batch_size) {
            let mut batch = self.witness_client.new_batch();
            let mut pending = Vec::with_capacity(chunk.len());
            for (block_idx, block_number, parent_block_number, address, storage_keys) in chunk {
                pending.push((
                    *block_idx,
                    *block_number,
                    *parent_block_number,
                    *address,
                    storage_keys,
                    Box::pin(
                        batch
                            .add_call::<_, EIP1186AccountProofResponse>(
                                "eth_getProof",
                                &(
                                    *address,
                                    storage_keys.clone(),
                                    BlockNumberOrTag::from(*parent_block_number),
                                ),
                            )
                            .map_err(|_| {
                                RaikoError::RPC(
                                    "failed adding storage eth_getProof call to batch".to_owned(),
                                )
                            })?,
                    ),
                ));
            }

            batch.send().await.map_err(|e| {
                let blocks = chunk
                    .iter()
                    .map(|(_, block_number, _, _, _)| *block_number)
                    .collect::<Vec<_>>();
                RaikoError::RPC(format!(
                    "error sending storage eth_getProof batch for blocks {blocks:?}: {e}"
                ))
            })?;

            for (block_idx, block_number, parent_block_number, address, storage_keys, request) in
                pending
            {
                let proof = request.await.map_err(|e| {
                    RaikoError::RPC(format!(
                        "error collecting storage eth_getProof for address {address} at parent block {parent_block_number} (block {block_number}): {e}"
                    ))
                })?;
                ensure_storage_proof_covers_keys(
                    address,
                    parent_block_number,
                    storage_keys,
                    &proof.storage_proof,
                )?;
                result[block_idx].extend(
                    proof
                        .account_proof
                        .iter()
                        .cloned()
                        .map(WitnessStateNode::from_bytes),
                );
                for storage_proof in proof.storage_proof {
                    result[block_idx].extend(
                        storage_proof
                            .proof
                            .into_iter()
                            .map(WitnessStateNode::from_bytes),
                    );
                }
            }
        }

        for nodes in &mut result {
            *nodes = ExecutionWitness::canonicalize_state_nodes(std::mem::take(nodes));
        }
        info!(
            block_count = block_numbers.len(),
            proof_requests = requests.len(),
            batch_size,
            elapsed_ms = started_at.elapsed().as_millis(),
            "fetched proposal-window storage proofs"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_proof_requests_reject_mismatched_lengths() {
        let err = build_account_proof_requests(&[1, 2], &[vec![Address::ZERO]])
            .expect_err("mismatched account proof inputs should fail");

        assert!(matches!(err, RaikoError::InvalidRequestConfig(_)));
        assert!(
            err.to_string()
                .contains("account proof block count (2) does not match address list count (1)")
        );
    }

    fn storage_proof(key: B256) -> EIP1186StorageProof {
        EIP1186StorageProof {
            key: key.into(),
            value: alloy_primitives::U256::ZERO,
            proof: Vec::new(),
        }
    }

    #[test]
    fn storage_proof_coverage_accepts_every_requested_key_in_any_order() {
        let keys = [B256::from([0x01; 32]), B256::from([0x02; 32])];

        ensure_storage_proof_covers_keys(
            Address::ZERO,
            9,
            &keys,
            &[storage_proof(keys[1]), storage_proof(keys[0])],
        )
        .expect("zero-valued proofs for every requested key are complete");
    }

    #[test]
    fn storage_proof_coverage_rejects_dropped_keys() {
        let keys = [B256::from([0x01; 32]), B256::from([0x02; 32])];

        let err =
            ensure_storage_proof_covers_keys(Address::ZERO, 9, &keys, &[storage_proof(keys[0])])
                .expect_err("a dropped key must fail fast");

        assert!(matches!(err, RaikoError::RPC(_)));
        assert!(
            err.to_string()
                .contains("returned 1 storage proofs for 2 requested keys")
        );
    }

    #[test]
    fn storage_proof_coverage_rejects_substituted_keys() {
        let keys = [B256::from([0x01; 32]), B256::from([0x02; 32])];

        let err = ensure_storage_proof_covers_keys(
            Address::ZERO,
            9,
            &keys,
            &[
                storage_proof(keys[0]),
                storage_proof(B256::from([0x03; 32])),
            ],
        )
        .expect_err("a substituted key must fail fast");

        assert!(err.to_string().contains("missing"));
    }
}
