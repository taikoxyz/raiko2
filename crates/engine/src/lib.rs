//! Raiko2 Engine - core proving engine for batch proofs.
//!
//! This module provides the main `Engine` type that orchestrates:
//! - Input creation from RPC providers
//! - Local validation of inputs
//! - Proof generation using zkVM provers
//!
//! ## Module Structure
//!
//! - `queue` - Task queue and scheduling for proof jobs
//! - `worker` - Supervised worker management with auto-restart
//! - `tasks` - Task types and outputs
//! - `input_builder` - Guest input construction
//!
//! Prover integration is provided via `raiko2-prover` and wired through
//! `queue::EngineQueue` / `tasks::EngineTask`.
//! TaikoEvmConfig integration pending alethia-reth-block dependency.

use alloy_consensus::transaction::SignerRecoverable;
use raiko2_driver::Driver;
use raiko2_primitives::{
    ChainSpec, GuestInput, GuestOutput, ProofContext, RaikoError, RaikoResult, StatelessInput,
    SupportedChainSpecs,
};
use raiko2_provider::Provider;
use tracing::info;

pub mod input_builder;
pub mod queue;
pub mod tasks;
pub mod worker;

/// The main proving engine.
#[derive(Debug)]
pub struct Engine<P: Provider> {
    driver: Driver,
    provider: P,
}

impl<P: Provider> Engine<P> {
    /// Create a new engine with the given provider.
    pub fn new(driver: Driver, provider: P) -> Self {
        Self { driver, provider }
    }

    /// Create guest input from the proof context.
    pub async fn create_input(&self, ctx: &ProofContext) -> RaikoResult<GuestInput> {
        info!("Creating input for batch {}", ctx.request.batch_id);

        // Get block numbers from the request
        let block_numbers: Vec<u64> = vec![ctx.request.batch_id]; // Simplified for now

        let blocks = self.provider.batch_blocks(&block_numbers).await?;
        let taiko_manifest = self.driver.taiko_manifest(ctx, &blocks).await?;
        let witnesses = self.provider.batch_witnesses(&block_numbers).await?;

        // Collect signers for each block
        let mut all_signers = Vec::with_capacity(blocks.len());
        for block in blocks.iter() {
            let mut signers = Vec::new();
            for tx in block.body.transactions() {
                if let Ok(signer) = tx.recover_signer() {
                    signers.push(signer);
                }
            }
            all_signers.push(signers);
        }

        let accounts = self
            .provider
            .batch_accounts(&block_numbers, &all_signers)
            .await?;

        let chain_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(ctx.request.l2_chain_id)
            .unwrap_or_else(|| ChainSpec {
                name: "unknown".to_string(),
                chain_id: ctx.request.l2_chain_id,
                ..Default::default()
            });

        Ok(GuestInput {
            taiko: taiko_manifest,
            witnesses: blocks
                .into_iter()
                .zip(witnesses)
                .zip(accounts)
                .map(|((block, witness), accounts)| StatelessInput {
                    block,
                    chain_spec: chain_spec.clone(),
                    witness,
                    accounts,
                })
                .collect(),
        })
    }

    /// Generate output from the input (for verification).
    pub fn generate_output(&self, input: &GuestInput) -> RaikoResult<GuestOutput> {
        let last_block = input
            .witnesses
            .last()
            .ok_or_else(|| RaikoError::InvalidRequestConfig("No blocks in input".to_string()))?;

        Ok(GuestOutput {
            header: last_block.block.header.clone(),
            hash: last_block.block.header.state_root,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_output_empty_input() {
        use alloy_primitives::{Address, map::AddressMap};
        use alloy_trie::TrieAccount;
        use raiko2_primitives::RaikoResult;
        use reth_ethereum_primitives::Block;
        use reth_stateless::ExecutionWitness;

        // Create a mock provider
        struct MockProvider;

        impl Provider for MockProvider {
            async fn batch_blocks(&self, _blocks: &[u64]) -> RaikoResult<Vec<Block>> {
                Ok(vec![])
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
        }

        let driver = Driver;
        let provider = MockProvider;
        let engine = Engine::new(driver, provider);

        let empty_input = GuestInput::default();
        let result = engine.generate_output(&empty_input);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No blocks in input")
        );
    }
}
