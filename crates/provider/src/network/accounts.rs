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
            // Get block hash once for all addresses in this block
            let block = self
                .provider
                .get_block(block_id.into())
                .await
                .map_err(|e| {
                    RaikoError::RPC(format!(
                        "eth_getBlockByNumber failed for block {block_number}: {e}"
                    ))
                })?
                .ok_or_else(|| RaikoError::RPC(format!("Block {block_number} not found")))?;
            let block_hash = block.header.hash_slow();

            for address in addresses {
                // Use eth_getProof to get account information (standard Ethereum RPC method)
                let proof = self
                    .provider
                    .get_proof(*address, vec![])
                    .hash(block_hash)
                    .await
                    .map_err(|e| {
                        RaikoError::RPC(format!(
                            "eth_getProof failed for address {address} at block {block_number}: {e}"
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
