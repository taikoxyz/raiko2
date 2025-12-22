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

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

use raiko2_pipeline::{Pipeline, PipelineSpec, PipelineStageResult, ProverBackend};
use raiko2_primitives::{GuestInput, GuestOutput, ProofContext, RaikoError, RaikoResult};
use raiko2_provider::Provider;
use tracing::info;

pub mod input_builder;
pub mod queue;
pub mod tasks;
pub mod worker;

/// The main proving engine.
#[derive(Debug)]
pub struct Engine<P, S, B>
where
    P: Provider,
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    spec: S,
    provider: P,
    backend: B,
}

impl<P, S, B> Engine<P, S, B>
where
    P: Provider,
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    /// Create a new engine with the given provider.
    pub const fn new(spec: S, provider: P, backend: B) -> Self {
        Self {
            spec,
            provider,
            backend,
        }
    }

    /// Access the hardfork specification.
    pub const fn spec(&self) -> &S {
        &self.spec
    }

    /// Access the prover backend configuration.
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Access the provider used by this engine.
    pub const fn provider(&self) -> &P {
        &self.provider
    }

    /// Run preflight via the pipeline.
    pub async fn preflight(
        &self,
        ctx: &ProofContext,
    ) -> RaikoResult<PipelineStageResult<GuestInput>> {
        let pipeline = Pipeline::new(&self.spec, &self.backend);
        pipeline.preflight(ctx, &self.provider).await
    }

    /// Run validation via the pipeline.
    pub fn validate(
        &self,
        ctx: &ProofContext,
        input: GuestInput,
    ) -> RaikoResult<PipelineStageResult<GuestInput>> {
        let pipeline = Pipeline::new(&self.spec, &self.backend);
        pipeline.validate(ctx, input)
    }

    /// Create guest input from the proof context.
    pub async fn create_input(&self, ctx: &ProofContext) -> RaikoResult<GuestInput> {
        info!("Creating input for batch {}", ctx.request.batch_id);

        let pipeline = Pipeline::new(&self.spec, &self.backend);
        let input = pipeline.build_guest_input(ctx, &self.provider).await?;
        Ok(input.output)
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
        use raiko2_pipeline::{
            NoopManifestBuilder, NoopValidation, PipelineSpec, Preflight, Risc0Backend,
        };
        use raiko2_primitives::RaikoResult;
        use reth_ethereum_primitives::Block;
        use reth_stateless::ExecutionWitness;

        struct TestSpec;
        const NOOP_VALIDATION: NoopValidation = NoopValidation;
        const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

        #[async_trait::async_trait]
        impl Preflight for TestSpec {
            async fn preflight<P: raiko2_provider::Provider>(
                &self,
                _ctx: &ProofContext,
                _provider: &P,
            ) -> RaikoResult<GuestInput> {
                Ok(GuestInput::default())
            }
        }

        impl PipelineSpec<Risc0Backend> for TestSpec {
            type Preflight = Self;
            type Validation = NoopValidation;
            type ManifestBuilder = NoopManifestBuilder;

            fn preflight(&self) -> &Self::Preflight {
                self
            }

            fn validation(&self) -> &Self::Validation {
                &NOOP_VALIDATION
            }

            fn manifest_builder(&self) -> &Self::ManifestBuilder {
                &NOOP_MANIFEST
            }
        }

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

        let provider = MockProvider;
        let backend = Risc0Backend::new(&[], &[]);
        let engine = Engine::new(TestSpec, provider, backend);

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
