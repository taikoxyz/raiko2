use crate::{PipelineSpec, PipelineStage, PipelineStageResult, Preflight, Validation};
use raiko2_primitives::{ProofContext, RaikoResult};
/// Pipeline-agnostic builder for guest inputs.
pub struct Pipeline<'a, S>
where
    S: PipelineSpec,
{
    spec: &'a S,
}

impl<'a, S> Pipeline<'a, S>
where
    S: PipelineSpec,
{
    /// Create a new pipeline using the provided pipeline spec.
    pub const fn new(spec: &'a S) -> Self {
        Self { spec }
    }

    /// Run the preflight stage.
    pub async fn preflight(
        &self,
        ctx: &ProofContext,
    ) -> RaikoResult<PipelineStageResult<S::GuestInput>>
    where
        S::Preflight: Preflight,
    {
        let input = self
            .spec
            .preflight()
            .preflight(ctx, self.spec.provider())
            .await?;
        Ok(PipelineStageResult::new(PipelineStage::Preflight, input))
    }

    /// Run the validation stage.
    pub fn validate(
        &self,
        ctx: &ProofContext,
        input: S::GuestInput,
    ) -> RaikoResult<PipelineStageResult<S::GuestInput>>
    where
        S::Validation: Validation,
    {
        self.spec.validation().validate(ctx, &input)?;
        Ok(PipelineStageResult::new(PipelineStage::Validation, input))
    }

    /// Build a guest input by running the unified pipeline steps.
    pub async fn build_guest_input(
        &self,
        ctx: &ProofContext,
    ) -> RaikoResult<PipelineStageResult<S::GuestInput>>
    where
        S::Preflight: Preflight,
        S::Validation: Validation,
    {
        let preflight = self.preflight(ctx).await?;
        self.validate(ctx, preflight.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        NativeBackend, NoopManifestBuilder, NoopValidation, PipelineKey, PipelineSpec, Preflight,
    };
    use raiko2_primitives::{GuestInput, ProofRequest, ProverConfig};
    use raiko2_provider::Provider;

    struct EmptySpec {
        prover: (),
        backend: NativeBackend,
        provider: EmptyProvider,
    }
    const NOOP_VALIDATION: NoopValidation<GuestInput> = NoopValidation::new();
    const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

    #[async_trait::async_trait]
    impl Preflight for EmptySpec {
        type Input = GuestInput;

        async fn preflight<P: Provider>(
            &self,
            _ctx: &ProofContext,
            _provider: &P,
        ) -> RaikoResult<GuestInput> {
            Ok(GuestInput::default())
        }
    }

    impl PipelineSpec for EmptySpec {
        type GuestInput = GuestInput;
        type Preflight = Self;
        type Validation = NoopValidation<GuestInput>;
        type ManifestBuilder = NoopManifestBuilder;
        type Prover = ();
        type Backend = NativeBackend;
        type Provider = EmptyProvider;

        fn pipeline_key(&self) -> PipelineKey {
            PipelineKey::ShastaNative
        }

        fn prover(&self) -> &Self::Prover {
            &self.prover
        }

        fn backend(&self) -> &Self::Backend {
            &self.backend
        }

        fn provider(&self) -> &Self::Provider {
            &self.provider
        }

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

    struct EmptyProvider;

    #[async_trait::async_trait]
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
        let spec = EmptySpec {
            prover: (),
            backend: NativeBackend,
            provider: EmptyProvider,
        };
        let pipeline = Pipeline::new(&spec);
        let ctx = ProofContext::new(ProofRequest::default(), ProverConfig::default());
        let input = pipeline
            .build_guest_input(&ctx)
            .await
            .expect("pipeline should succeed");

        assert!(input.output.witnesses.is_empty());
        assert_eq!(input.output.taiko.proposal_id, 0);
    }
}
