use alloy::{eips::BlockNumberOrTag, rlp::decode_exact};
use alloy_primitives::{Address, B256, keccak256, map::AddressMap};
use alloy_rpc_types_eth::EIP1186AccountProofResponse;
use alloy_trie::TrieAccount;
use raiko2_primitives::{RaikoError, RaikoResult};
use risc0_ethereum_trie::Trie;

use super::NetworkProvider;

const ACCOUNT_PROOF_BATCH_SIZE: usize = 250;

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
            let parent_block_number = block_number.checked_sub(1).ok_or_else(|| {
                RaikoError::RPC(format!(
                    "cannot fetch pre-state signer accounts for genesis block {block_number}"
                ))
            })?;

            for chunk in addresses.chunks(ACCOUNT_PROOF_BATCH_SIZE) {
                let mut batch = self.l2_client.new_batch();
                let mut requests = Vec::with_capacity(chunk.len());
                for &address in chunk {
                    requests.push((
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
                    RaikoError::RPC(format!(
                        "error sending eth_getProof batch for parent of block {block_number}: {e}"
                    ))
                })?;

                for (address, request) in requests {
                    let proof = request.await.map_err(|e| {
                        RaikoError::RPC(format!(
                            "error collecting eth_getProof for address {address} at parent of block {block_number}: {e}"
                        ))
                    })?;
                    let account = decode_account_from_proof(&proof)?;
                    accounts.insert(address, account);
                }
            }

            result.push(accounts);
        }

        Ok(result)
    }
}
