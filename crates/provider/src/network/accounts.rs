use alloy::{eips::BlockNumberOrTag, providers::Provider as AlloyProvider, rlp::decode_exact};
use alloy_primitives::{Address, keccak256, map::AddressMap};
use alloy_rpc_types_eth::EIP1186AccountProofResponse;
use alloy_trie::TrieAccount;
use raiko2_primitives::{RaikoError, RaikoResult};
use risc0_ethereum_trie::Trie;

use super::NetworkProvider;

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

impl NetworkProvider {
    pub(crate) async fn fetch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<TrieAccount>>> {
        let mut result = Vec::with_capacity(block_numbers.len());
        for (block_number, addresses) in block_numbers.iter().zip(addresses.iter()) {
            let mut accounts = AddressMap::default();
            let block_id = BlockNumberOrTag::from(*block_number);
            // Load the block once so account proofs can be queried against the parent state root.
            let block = self
                .l2_provider
                .get_block(block_id.into())
                .await
                .map_err(|e| {
                    RaikoError::RPC(format!(
                        "eth_getBlockByNumber failed for block {block_number}: {e}"
                    ))
                })?
                .ok_or_else(|| RaikoError::RPC(format!("Block {block_number} not found")))?;
            let parent_block_hash = block.header.parent_hash;

            for address in addresses {
                // Execution witnesses contain pre-state, so signer accounts must come from the
                // parent block state rather than the post-state of the current block.
                let proof = self
                    .l2_provider
                    .get_proof(*address, vec![])
                    .hash(parent_block_hash)
                    .await
                    .map_err(|e| {
                        RaikoError::RPC(format!(
                            "eth_getProof failed for address {address} at parent of block {block_number}: {e}"
                        ))
                    })?;
                let account = decode_account_from_proof(&proof)?;
                accounts.insert(*address, account);
            }
            result.push(accounts);
        }

        Ok(result)
    }
}
