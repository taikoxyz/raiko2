use axum::{Json, extract::State};
use raiko2_pipeline::PipelineKey;
use serde::Serialize;

use super::super::state::AppState;

/// Server info response.
#[derive(Serialize)]
pub struct InfoResponse {
    pub version: &'static str,
    pub prover: String,
    pub supported_provers: Vec<&'static str>,
}

const PROVER_PIPELINE_BINDINGS: [(PipelineKey, &str); 6] = [
    (PipelineKey::ShastaRisc0, "risc0"),
    (PipelineKey::ShastaSp1, "sp1"),
    (PipelineKey::ShastaNative, "native"),
    (PipelineKey::ShastaAgentRisc0, "agent-risc0"),
    (PipelineKey::ShastaTdx, "tdx"),
    (PipelineKey::ShastaAzureTdx, "azure-tdx"),
];

fn supported_provers(state: &AppState) -> Vec<&'static str> {
    PROVER_PIPELINE_BINDINGS
        .into_iter()
        .filter_map(|(key, prover)| state.pipelines.get(key).map(|_| prover))
        .collect()
}

/// Get server info.
pub async fn get_info(State(state): State<AppState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        version: env!("CARGO_PKG_VERSION"),
        prover: format!("{:?}", state.config.prover.prover_type),
        supported_provers: supported_provers(&state),
    })
}

#[cfg(test)]
mod tests {
    use super::get_info;
    use crate::config::Config;
    use crate::server::state::{AppState, StaticPipelineFactory};
    use axum::extract::State;
    use raiko2_engine::Engine;
    use raiko2_pipeline::{NativeBackend, PipelineKey, forks::shasta::ShastaSpec};
    use raiko2_primitives::{ProofContext, ProofRequest, RaikoResult};
    use raiko2_prover::native::NativeProver;
    use raiko2_provider::Provider;
    use reth_ethereum_primitives::Block;
    use reth_stateless::ExecutionWitness;
    use std::sync::Arc;

    #[derive(Clone)]
    struct EmptyProvider;

    #[async_trait::async_trait]
    impl Provider for EmptyProvider {
        async fn batch_blocks(&self, _blocks: &[u64]) -> RaikoResult<Vec<Block>> {
            Ok(vec![])
        }

        async fn batch_accounts(
            &self,
            _blocks: &[u64],
            _accounts: &[Vec<alloy_primitives::Address>],
        ) -> RaikoResult<Vec<alloy_primitives::map::AddressMap<alloy_trie::TrieAccount>>> {
            Ok(vec![])
        }

        async fn batch_witnesses(&self, _blocks: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
            Ok(vec![])
        }
    }

    fn dummy_engine(
        key: PipelineKey,
    ) -> Engine<ShastaSpec<NativeProver, NativeBackend, EmptyProvider>> {
        let spec = ShastaSpec::new(key, NativeProver, NativeBackend, EmptyProvider);
        let context = ProofContext::new(
            ProofRequest::default(),
            raiko2_primitives::ProverConfig::default(),
        );
        Engine::new(spec, context)
    }

    fn state_with_pipelines(keys: &[PipelineKey]) -> AppState {
        let mut factory = StaticPipelineFactory::default();
        for key in keys {
            factory.insert(*key, Arc::new(dummy_engine(*key)));
        }
        AppState {
            config: Arc::new(Config::default()),
            pipelines: Arc::new(factory),
        }
    }

    #[tokio::test]
    async fn info_returns_only_registered_provers_in_canonical_order() {
        let state = state_with_pipelines(&[
            PipelineKey::ShastaAgentRisc0,
            PipelineKey::ShastaNative,
            PipelineKey::ShastaRisc0,
        ]);

        let axum::Json(response) = get_info(State(state)).await;
        assert_eq!(
            response.supported_provers,
            vec!["risc0", "native", "agent-risc0"]
        );
    }

    #[tokio::test]
    async fn info_returns_empty_supported_provers_when_no_pipeline_registered() {
        let state = state_with_pipelines(&[]);

        let axum::Json(response) = get_info(State(state)).await;
        assert!(response.supported_provers.is_empty());
    }
}
