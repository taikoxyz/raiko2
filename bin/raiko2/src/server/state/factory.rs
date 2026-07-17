use raiko2_pipeline::PipelineKey;
use std::collections::HashMap;
use std::sync::Arc;

use super::engine::EngineHandle;

/// Engine binding key for a concrete network pair and proving route.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct PipelineBindingKey {
    pub network_pair: String,
    pub pipeline: PipelineKey,
}

/// Pipeline factory for resolving engines.
pub trait PipelineFactory: Send + Sync {
    fn get(&self, network_pair: &str, key: PipelineKey) -> Option<Arc<dyn EngineHandle>>;

    fn queue_maintenance_ready(&self, _max_age: std::time::Duration) -> bool {
        true
    }
}

#[derive(Default)]
pub struct StaticPipelineFactory {
    engines: HashMap<PipelineBindingKey, Arc<dyn EngineHandle>>,
}

impl StaticPipelineFactory {
    pub fn insert(
        &mut self,
        network_pair: impl Into<String>,
        key: PipelineKey,
        engine: Arc<dyn EngineHandle>,
    ) {
        let network_pair = network_pair.into();
        self.engines.insert(
            PipelineBindingKey {
                network_pair,
                pipeline: key,
            },
            engine,
        );
    }
}

impl PipelineFactory for StaticPipelineFactory {
    fn get(&self, network_pair: &str, key: PipelineKey) -> Option<Arc<dyn EngineHandle>> {
        self.engines
            .get(&PipelineBindingKey {
                network_pair: network_pair.to_string(),
                pipeline: key,
            })
            .cloned()
    }

    fn queue_maintenance_ready(&self, max_age: std::time::Duration) -> bool {
        !self.engines.is_empty()
            && self
                .engines
                .values()
                .all(|engine| engine.queue_maintenance_ready(max_age))
    }
}
