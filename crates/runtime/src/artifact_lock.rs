use crate::ProofArtifactKey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Mutex as TokioMutex;

#[derive(Debug, Default)]
pub struct ArtifactLifecycleLocks {
    entries: Mutex<HashMap<ProofArtifactKey, Weak<TokioMutex<()>>>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArtifactLifecycleLockRegistryStats {
    pub live: usize,
    pub dead: usize,
    pub swept: usize,
}

impl ArtifactLifecycleLocks {
    pub fn resolve(&self, key: &ProofArtifactKey) -> Arc<TokioMutex<()>> {
        let mut entries = self
            .entries
            .lock()
            .expect("artifact lifecycle lock registry poisoned");
        if let Some(lock) = entries.get(key).and_then(Weak::upgrade) {
            return lock;
        }

        let lock = Arc::new(TokioMutex::new(()));
        entries.insert(key.clone(), Arc::downgrade(&lock));
        lock
    }

    pub fn sweep(&self) -> ArtifactLifecycleLockRegistryStats {
        let mut entries = self
            .entries
            .lock()
            .expect("artifact lifecycle lock registry poisoned");
        let before = entries.len();
        let dead = entries
            .values()
            .filter(|lock| lock.strong_count() == 0)
            .count();
        entries.retain(|_, lock| lock.strong_count() > 0);
        ArtifactLifecycleLockRegistryStats {
            live: entries.len(),
            dead,
            swept: before.saturating_sub(entries.len()),
        }
    }

    #[cfg(test)]
    pub fn entry_count(&self) -> usize {
        self.entries
            .lock()
            .expect("artifact lifecycle lock registry poisoned")
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::ArtifactLifecycleLocks;
    use crate::ProofArtifactKey;
    use raiko2_pipeline::PipelineKey;
    use std::sync::Arc;

    fn artifact_key(proof_ref: &str) -> ProofArtifactKey {
        let pipeline_key = PipelineKey::ShastaSp1;
        ProofArtifactKey {
            network_pair: "l1-l2".to_string(),
            pipeline_key,
            route: pipeline_key.route(),
            proof_ref: proof_ref.to_string(),
        }
    }

    #[test]
    fn same_key_resolves_to_the_same_live_mutex() {
        let registry = ArtifactLifecycleLocks::default();
        let key = artifact_key("proof-a");
        let first = registry.resolve(&key);
        let second = registry.resolve(&key);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn different_keys_do_not_share_a_mutex() {
        let registry = ArtifactLifecycleLocks::default();
        let first = registry.resolve(&artifact_key("proof-a"));
        let second = registry.resolve(&artifact_key("proof-b"));
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn waiter_keeps_the_registry_entry_live_during_sweep() {
        let registry = Arc::new(ArtifactLifecycleLocks::default());
        let key = artifact_key("proof-a");
        let first = registry.resolve(&key);
        let first_guard = Arc::clone(&first).lock_owned().await;
        let waiter_started = Arc::new(tokio::sync::Notify::new());
        let waiter = tokio::spawn({
            let registry = Arc::clone(&registry);
            let key = key.clone();
            let waiter_started = Arc::clone(&waiter_started);
            async move {
                let lock = registry.resolve(&key);
                let observed = Arc::clone(&lock);
                waiter_started.notify_one();
                let _guard = lock.lock_owned().await;
                observed
            }
        });

        waiter_started.notified().await;
        drop(first);
        drop(first_guard);
        assert_eq!(registry.sweep().swept, 0);
        let third = registry.resolve(&key);

        let waiter_lock = waiter.await.expect("waiter task");
        assert!(Arc::ptr_eq(&third, &waiter_lock));
    }

    #[test]
    fn dead_weak_entries_are_reclaimed() {
        let registry = ArtifactLifecycleLocks::default();
        let key = artifact_key("proof-a");
        drop(registry.resolve(&key));

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.sweep().swept, 1);
        assert_eq!(registry.entry_count(), 0);

        let replacement = registry.resolve(&key);
        assert_eq!(registry.entry_count(), 1);
        drop(replacement);
    }
}
