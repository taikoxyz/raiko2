use raiko2_pipeline::{PipelineSpec, PipelineStageResult, ProverBackend};
use raiko2_primitives::{GuestInput, ProofContext, ProofRequest, ProverConfig, RaikoResult};
use raiko2_provider::NetworkProvider;

#[async_trait::async_trait]
pub trait GuestInputBuilder<S, B>: Send + Sync
where
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    async fn preflight(&self, batch_id: u64) -> Result<PipelineStageResult<GuestInput>, String>;
    async fn validate(
        &self,
        batch_id: u64,
        input: GuestInput,
    ) -> Result<PipelineStageResult<GuestInput>, String>;
    async fn build_guest_input(&self, batch_id: u64) -> Result<GuestInput, String> {
        let preflight = self.preflight(batch_id).await?;
        let validated = self.validate(batch_id, preflight.output).await?;
        Ok(validated.output)
    }
}

pub struct DefaultGuestInputBuilder;

#[async_trait::async_trait]
impl<S, B> GuestInputBuilder<S, B> for DefaultGuestInputBuilder
where
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    async fn preflight(&self, _batch_id: u64) -> Result<PipelineStageResult<GuestInput>, String> {
        Ok(PipelineStageResult::new(
            raiko2_pipeline::PipelineStage::Preflight,
            GuestInput::default(),
        ))
    }

    async fn validate(
        &self,
        _batch_id: u64,
        input: GuestInput,
    ) -> Result<PipelineStageResult<GuestInput>, String> {
        Ok(PipelineStageResult::new(
            raiko2_pipeline::PipelineStage::Validation,
            input,
        ))
    }
}

pub struct NetworkGuestInputBuilder<S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    engine: crate::Engine<NetworkProvider, S, B>,
    l1_chain_id: u64,
    l2_chain_id: u64,
    proof_type: String,
    config: ProverConfig,
}

impl<S, B> NetworkGuestInputBuilder<S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    pub fn new(
        spec: S,
        backend: B,
        rpc_url: &str,
        l1_chain_id: u64,
        l2_chain_id: u64,
        proof_type: String,
        config: ProverConfig,
    ) -> RaikoResult<Self> {
        let provider = NetworkProvider::new(rpc_url)?;
        Ok(Self {
            engine: crate::Engine::new(spec, provider, backend),
            l1_chain_id,
            l2_chain_id,
            proof_type,
            config,
        })
    }

    fn build_context(&self, batch_id: u64) -> ProofContext {
        ProofContext::new(
            ProofRequest {
                l1_chain_id: self.l1_chain_id,
                l2_chain_id: self.l2_chain_id,
                batch_id,
                proof_type: self.proof_type.clone(),
                blob_proof_type: None,
                prover: None,
                graffiti: None,
            },
            self.config.clone(),
        )
    }
}

#[async_trait::async_trait]
impl<S, B> GuestInputBuilder<S, B> for NetworkGuestInputBuilder<S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
{
    async fn preflight(&self, batch_id: u64) -> Result<PipelineStageResult<GuestInput>, String> {
        let ctx = self.build_context(batch_id);
        self.engine.preflight(&ctx).await.map_err(|e| e.to_string())
    }

    async fn validate(
        &self,
        batch_id: u64,
        input: GuestInput,
    ) -> Result<PipelineStageResult<GuestInput>, String> {
        let ctx = self.build_context(batch_id);
        self.engine.validate(&ctx, input).map_err(|e| e.to_string())
    }

    async fn build_guest_input(&self, batch_id: u64) -> Result<GuestInput, String> {
        let ctx = self.build_context(batch_id);
        self.engine
            .create_input(&ctx)
            .await
            .map_err(|e| e.to_string())
    }
}
