use async_trait::async_trait;
use raiko2_driver::Driver;
use raiko2_primitives::{GuestInput, ProofContext, ProofRequest, ProverConfig, RaikoResult};
use raiko2_provider::NetworkProvider;

#[async_trait]
pub trait GuestInputBuilder: Send + Sync {
    async fn build_guest_input(&self, batch_id: u64) -> Result<GuestInput, String>;
}

pub struct DefaultGuestInputBuilder;

#[async_trait]
impl GuestInputBuilder for DefaultGuestInputBuilder {
    async fn build_guest_input(&self, _batch_id: u64) -> Result<GuestInput, String> {
        Ok(GuestInput::default())
    }
}

pub struct NetworkGuestInputBuilder {
    engine: crate::Engine<NetworkProvider>,
    l1_chain_id: u64,
    l2_chain_id: u64,
    proof_type: String,
    config: ProverConfig,
}

impl NetworkGuestInputBuilder {
    pub fn new(
        rpc_url: &str,
        l1_chain_id: u64,
        l2_chain_id: u64,
        proof_type: String,
        config: ProverConfig,
    ) -> RaikoResult<Self> {
        let provider = NetworkProvider::new(rpc_url)?;
        Ok(Self {
            engine: crate::Engine::new(Driver::new(), provider),
            l1_chain_id,
            l2_chain_id,
            proof_type,
            config,
        })
    }
}

#[async_trait]
impl GuestInputBuilder for NetworkGuestInputBuilder {
    async fn build_guest_input(&self, batch_id: u64) -> Result<GuestInput, String> {
        let ctx = ProofContext::new(
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
        );

        self.engine
            .create_input(&ctx)
            .await
            .map_err(|e| e.to_string())
    }
}
