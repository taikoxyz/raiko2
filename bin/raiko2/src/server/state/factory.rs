use raiko2_pipeline::PipelineKey;
use std::collections::HashMap;
use std::sync::Arc;

use super::engine::EngineHandle;

/// Pipeline factory for resolving engines.
pub trait PipelineFactory: Send + Sync {
    fn get(&self, key: PipelineKey) -> Option<Arc<dyn EngineHandle>>;
}

#[derive(Default)]
pub struct StaticPipelineFactory {
    engines: HashMap<PipelineKey, Arc<dyn EngineHandle>>,
}

impl StaticPipelineFactory {
    pub fn insert(&mut self, key: PipelineKey, engine: Arc<dyn EngineHandle>) {
        self.engines.insert(key, engine);
    }
}

impl PipelineFactory for StaticPipelineFactory {
    fn get(&self, key: PipelineKey) -> Option<Arc<dyn EngineHandle>> {
        self.engines.get(&key).cloned()
    }
}
