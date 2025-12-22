use raiko2_hardfork::{HardforkSpec, Preflight, Validation};
use raiko2_primitives::{GuestInput, ProofContext, RaikoResult};
use raiko2_provider::Provider;

/// Hardfork-agnostic pipeline for building guest inputs.
pub(crate) struct Pipeline<'a, F: HardforkSpec> {
    spec: &'a F,
}

impl<'a, F: HardforkSpec> Pipeline<'a, F> {
    /// Create a new pipeline using the provided hardfork spec.
    pub const fn new(spec: &'a F) -> Self {
        Self { spec }
    }

    /// Build a guest input by running the unified pipeline steps.
    pub async fn build_guest_input<P>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<GuestInput>
    where
        P: Provider,
        F::Preflight: Preflight,
        F::Validation: Validation,
    {
        let input = self.spec.preflight().preflight(ctx, provider).await?;
        self.spec.validation().validate(ctx, &input)?;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_hardfork::{
        HardforkSpec, NoopManifestBuilder, NoopValidation, Preflight, ProofStage, ProverBackend,
    };
    use raiko2_primitives::{ProofRequest, ProverConfig};

    struct EmptySpec;
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

    impl HardforkSpec for EmptySpec {
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

        fn elf(&self, _backend: ProverBackend, _stage: ProofStage) -> RaikoResult<&'static [u8]> {
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
        let pipeline = Pipeline::new(&spec);
        let ctx = ProofContext::new(ProofRequest::default(), ProverConfig::default());
        let input = pipeline
            .build_guest_input(&ctx, &EmptyProvider)
            .await
            .expect("pipeline should succeed");

        assert!(input.witnesses.is_empty());
        assert_eq!(input.taiko.batch_id, 0);
    }
}
