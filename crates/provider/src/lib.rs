#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

pub mod network;
pub mod on_the_spot_witness;
pub mod rpc;

use alloy::consensus::Header;
use alloy_primitives::{Address, map::AddressMap};
use alloy_trie::TrieAccount;
use raiko2_primitives::{ChainSpec, ExecutionWitness, RaikoResult};
use raiko2_protocol::{BlobProofType, InputDataSource};
use raiko2_protocol_shasta::shasta::ShastaEventData;
use reth_ethereum_primitives::Block;

pub use network::{L2ProviderKind, NetworkProvider};
pub use rpc::{RpcClientConfig, RpcRetryConfig};

/// The `Provider` trait defines asynchronous methods for batch retrieval of blockchain data.
///
/// Implementors of this trait are responsible for providing access to blocks, accounts, and execution witnesses
/// for given block numbers and account addresses.
///
/// # Methods
///
/// - [`batch_blocks`]: Fetches a batch of blocks corresponding to the provided block numbers.
/// - [`batch_accounts`]: Fetches account state data for multiple blocks and sets of addresses.
/// - [`batch_witnesses`]: Fetches execution witnesses for a batch of blocks.
/// - [`batch_l1_headers`]: Fetches L1 headers required for Shasta anchor linkage validation.
///
/// All methods return a [`RaikoResult`] wrapping the respective data type.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn batch_blocks(&self, blocks: &[u64]) -> RaikoResult<Vec<Block>>;

    async fn batch_accounts(
        &self,
        blocks: &[u64],
        accounts: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<TrieAccount>>>;

    async fn batch_witnesses(&self, blocks: &[u64]) -> RaikoResult<Vec<ExecutionWitness>>;

    async fn batch_l1_headers(&self, blocks: &[u64]) -> RaikoResult<Vec<Header>>;

    async fn shasta_proposal_event(
        &self,
        _l1_contract: Address,
        _l1_inclusion_block_number: u64,
        _proposal_id: u64,
    ) -> RaikoResult<ShastaEventData> {
        Err(raiko2_primitives::RaikoError::InvalidRequestConfig(
            "provider does not support Shasta proposal event lookup".to_string(),
        ))
    }

    async fn shasta_data_sources(
        &self,
        _l1_chain_spec: &ChainSpec,
        _proposal_event: &ShastaEventData,
        _blob_proof_type: BlobProofType,
    ) -> RaikoResult<Vec<InputDataSource>> {
        Err(raiko2_primitives::RaikoError::InvalidRequestConfig(
            "provider does not support canonical Shasta data source lookup".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    /// Mock provider for testing.
    pub(crate) struct MockProvider {
        pub blocks: Vec<Block>,
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<Block>> {
            Ok(self
                .blocks
                .iter()
                .filter(|b| block_numbers.contains(&b.header.number))
                .cloned()
                .collect())
        }

        async fn batch_accounts(
            &self,
            _blocks: &[u64],
            _accounts: &[Vec<Address>],
        ) -> RaikoResult<Vec<AddressMap<TrieAccount>>> {
            Ok(vec![])
        }

        async fn batch_witnesses(&self, _blocks: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
            Ok(vec![])
        }

        async fn batch_l1_headers(&self, blocks: &[u64]) -> RaikoResult<Vec<Header>> {
            Ok(self
                .blocks
                .iter()
                .filter(|b| blocks.contains(&b.header.number))
                .map(|b| b.header.clone())
                .collect())
        }
    }

    #[test]
    fn test_trie_account_fields() {
        let account = TrieAccount {
            nonce: 1,
            balance: U256::from(100),
            storage_root: Default::default(),
            code_hash: Default::default(),
        };

        assert_eq!(account.nonce, 1);
        assert_eq!(account.balance, U256::from(100));
    }

    #[tokio::test]
    async fn test_mock_provider_empty() -> Result<(), Box<dyn std::error::Error>> {
        let provider = MockProvider { blocks: vec![] };
        let blocks = provider.batch_blocks(&[1, 2, 3]).await?;
        assert!(blocks.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_mock_provider_accounts() -> Result<(), Box<dyn std::error::Error>> {
        let provider = MockProvider { blocks: vec![] };
        let accounts = provider
            .batch_accounts(&[1], &[vec![Address::ZERO]])
            .await?;
        assert!(accounts.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_mock_provider_witnesses() -> Result<(), Box<dyn std::error::Error>> {
        let provider = MockProvider { blocks: vec![] };
        let witnesses = provider.batch_witnesses(&[1]).await?;
        assert!(witnesses.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_mock_provider_l1_headers() -> Result<(), Box<dyn std::error::Error>> {
        let provider = MockProvider { blocks: vec![] };
        let headers = provider.batch_l1_headers(&[1]).await?;
        assert!(headers.is_empty());
        Ok(())
    }
}
