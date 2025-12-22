use crate::{
    PipelineSpec, PipelineStage, PipelineStageResult, Preflight, ProverBackend, Validation,
};
use raiko2_primitives::{GuestInput, ProofContext, RaikoResult};
use raiko2_provider::Provider;

/// Pipeline-agnostic builder for guest inputs.
pub struct Pipeline<'a, S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    spec: &'a S,
    backend: &'a B,
}

impl<'a, S, B> Pipeline<'a, S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    /// Create a new pipeline using the provided pipeline spec.
    pub const fn new(spec: &'a S, backend: &'a B) -> Self {
        Self { spec, backend }
    }

    /// Run the preflight stage.
    pub async fn preflight<P>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<PipelineStageResult<GuestInput>>
    where
        P: Provider,
        S::Preflight: Preflight,
    {
        let input = self.spec.preflight().preflight(ctx, provider).await?;
        Ok(PipelineStageResult::new(PipelineStage::Preflight, input))
    }

    /// Run the validation stage.
    pub fn validate(
        &self,
        ctx: &ProofContext,
        input: GuestInput,
    ) -> RaikoResult<PipelineStageResult<GuestInput>>
    where
        S::Validation: Validation,
    {
        self.spec.validation().validate(ctx, &input)?;
        Ok(PipelineStageResult::new(PipelineStage::Validation, input))
    }

    /// Build a guest input by running the unified pipeline steps.
    pub async fn build_guest_input<P>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<PipelineStageResult<GuestInput>>
    where
        P: Provider,
        S::Preflight: Preflight,
        S::Validation: Validation,
    {
        let preflight = self.preflight(ctx, provider).await?;
        self.validate(ctx, preflight.output)
    }

    /// Access the prover backend used by this pipeline.
    pub const fn backend(&self) -> &B {
        self.backend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        NoopManifestBuilder, NoopValidation, PipelineSpec, Preflight, ProofStage, ProverBackend,
    };
    use raiko2_primitives::{ProofRequest, ProverConfig};

    struct EmptySpec;
    struct TestBackend;
    const NOOP_VALIDATION: NoopValidation = NoopValidation;
    const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

    #[async_trait::async_trait]
    impl Preflight for EmptySpec {
        async fn preflight<P: Provider>(
            &self,
            _ctx: &ProofContext,
            _provider: &P,
        ) -> RaikoResult<GuestInput> {
            Ok(GuestInput::default())
        }
    }

    impl PipelineSpec<TestBackend> for EmptySpec {
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

    impl ProverBackend for TestBackend {
        fn elf(&self, _stage: ProofStage) -> RaikoResult<&'static [u8]> {
            Ok(&[])
        }
    }

    struct EmptyProvider;

    impl Provider for EmptyProvider {
        async fn batch_blocks(
            &self,
            _blocks: &[u64],
        ) -> RaikoResult<Vec<reth_ethereum_primitives::Block>> {
            Ok(vec![])
        }

        async fn batch_accounts(
            &self,
            _blocks: &[u64],
            _accounts: &[Vec<alloy_primitives::Address>],
        ) -> RaikoResult<Vec<alloy_primitives::map::AddressMap<alloy_trie::TrieAccount>>> {
            Ok(vec![])
        }

        async fn batch_witnesses(
            &self,
            _blocks: &[u64],
        ) -> RaikoResult<Vec<reth_stateless::ExecutionWitness>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_pipeline_empty_input() {
        let spec = EmptySpec;
        let backend = TestBackend;
        let pipeline = Pipeline::new(&spec, &backend);
        let ctx = ProofContext::new(ProofRequest::default(), ProverConfig::default());
        let input = pipeline
            .build_guest_input(&ctx, &EmptyProvider)
            .await
            .expect("pipeline should succeed");

        assert!(input.output.witnesses.is_empty());
        assert_eq!(input.output.taiko.batch_id, 0);
    }
}
