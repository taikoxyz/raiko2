use alloy::{eips::BlockNumberOrTag, rlp::decode_exact};
use alloy_primitives::{Address, B256, keccak256, map::AddressMap};
use alloy_rpc_types_eth::EIP1186AccountProofResponse;
use alloy_trie::TrieAccount;
use raiko2_primitives::{ExecutionWitness, RaikoError, RaikoResult, WitnessStateNode};
use risc0_ethereum_trie::Trie;
use std::time::Instant;
use tracing::info;

use super::RpcL2Provider;

const DEFAULT_ACCOUNT_PROOF_BATCH_SIZE: usize = 250;

type AccountProofRequest = (usize, u64, Address);

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
}
