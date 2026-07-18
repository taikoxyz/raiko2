use raiko2_pipeline::PipelineKey;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
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

    fn start_workers(&self, _workers: usize, _maintenance_interval: std::time::Duration) {}

    fn queue_maintenance_ready(&self, _max_age: std::time::Duration) -> bool {
        true
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
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

    fn start_workers(&self, workers: usize, maintenance_interval: std::time::Duration) {
        for engine in self.engines.values() {
            engine.start_workers(workers, maintenance_interval);
        }
    }

    fn queue_maintenance_ready(&self, max_age: std::time::Duration) -> bool {
        self.engines
            .values()
            .all(|engine| engine.queue_maintenance_ready(max_age))
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            for engine in self.engines.values() {
                engine.shutdown().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::{EngineQueueTaskView, EngineStatusView};
    use raiko2_engine::{EngineExecutionPlan, EngineTaskId, EngineTaskKey};
    use raiko2_queue::TaskStoreError;
    use std::future::Future;
    use std::pin::Pin;

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    struct MaintenanceStateEngine {
        ready: bool,
    }

    impl EngineHandle for MaintenanceStateEngine {
        fn queue_maintenance_ready(&self, _max_age: std::time::Duration) -> bool {
            self.ready
        }

        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { panic!("unexpected status lookup") })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { panic!("unexpected task state lookup") })
        }

        fn attach_execution_plan(
            &self,
            _owner: raiko2_queue::RootOwner,
            _plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<raiko2_queue::AttachOutcome, TaskStoreError>> {
            Box::pin(async { panic!("unexpected execution attachment") })
        }

        fn detach_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
            mode: raiko2_queue::DetachMode,
        ) -> BoxFuture<'_, Result<raiko2_queue::DetachOutcome<EngineTaskKey>, TaskStoreError>>
        {
            Box::pin(async move { Ok(raiko2_queue::DetachOutcome::not_attached(mode)) })
        }
    }

    fn engine(ready: bool) -> Arc<dyn EngineHandle> {
        Arc::new(MaintenanceStateEngine { ready })
    }

    #[test]
    fn empty_factory_is_queue_ready() {
        let factory = StaticPipelineFactory::default();

        assert!(factory.queue_maintenance_ready(std::time::Duration::from_secs(1)));
    }

    #[test]
    fn factory_is_queue_ready_when_every_engine_has_a_fresh_tick() {
        let mut factory = StaticPipelineFactory::default();
        factory.insert("pair-a", PipelineKey::ShastaNative, engine(true));
        factory.insert("pair-a", PipelineKey::ShastaSp1, engine(true));

        assert!(factory.queue_maintenance_ready(std::time::Duration::from_secs(1)));
    }

    #[test]
    fn factory_is_not_queue_ready_when_any_engine_has_no_fresh_tick() {
        for (first_ready, second_ready) in [(false, true), (true, false), (false, false)] {
            let mut factory = StaticPipelineFactory::default();
            factory.insert("pair-a", PipelineKey::ShastaNative, engine(first_ready));
            factory.insert("pair-a", PipelineKey::ShastaSp1, engine(second_ready));

            assert!(
                !factory.queue_maintenance_ready(std::time::Duration::from_secs(1)),
                "factory unexpectedly ready for engine states ({first_ready}, {second_ready})"
            );
        }
    }
}
