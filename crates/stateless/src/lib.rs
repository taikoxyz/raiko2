#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko2 Stateless Validation
//!
//! This crate provides stateless block validation using execution witnesses.
//! It validates blocks without requiring access to the full node state by
//! using state proofs and witness data.

mod analysis;
mod sparse;
mod trie;
mod validation;
mod witness_db;

pub use crate::analysis::{WitnessMaterializationStats, analyze_block_with_witness_resources};
pub use crate::trie::StatelessTrie;
pub use alethia_reth_block::filtered_block::FilteredBlockExecutionOutcome;
pub use raiko2_primitives::{ExecutionWitness, StatelessValidationError};
pub use validation::{
    reconstruct_block_from_transactions_with_witness_resources, validate_block,
    validate_block_with_ancestor_headers, validate_block_with_witness_resources,
};

#[cfg(test)]
mod tests {
    use alloy_consensus::{Header, TrieAccount};
    use alloy_primitives::{Address, B256, U256, map::AddressMap};

    #[test]
    fn test_trie_account_creation() {
        let account = TrieAccount {
            nonce: 1,
            balance: U256::from(100),
            storage_root: B256::ZERO,
            code_hash: B256::ZERO,
        };
        assert_eq!(account.nonce, 1);
        assert_eq!(account.balance, U256::from(100));
    }

    #[test]
    fn test_address_map_operations() {
        let mut accounts: AddressMap<TrieAccount> = AddressMap::default();
        let addr = Address::ZERO;
        let account = TrieAccount {
            nonce: 0,
            balance: U256::ZERO,
            storage_root: B256::ZERO,
            code_hash: B256::ZERO,
        };
        accounts.insert(addr, account);
        assert!(accounts.contains_key(&addr));
        assert_eq!(accounts.len(), 1);
    }

    #[test]
    fn test_header_default() {
        let header = Header::default();
        assert_eq!(header.number, 0);
        assert_eq!(header.gas_limit, 0);
    }

    #[test]
    fn test_header_with_values() {
        let header = Header {
            number: 12345,
            gas_limit: 30_000_000,
            timestamp: 1700000000,
            ..Default::default()
        };

        assert_eq!(header.number, 12345);
        assert_eq!(header.gas_limit, 30_000_000);
        assert_eq!(header.timestamp, 1700000000);
    }

    #[test]
    fn test_trie_account_default() {
        let account = TrieAccount::default();
        assert_eq!(account.nonce, 0);
        assert_eq!(account.balance, U256::ZERO);
    }
}
