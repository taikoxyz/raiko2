use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use raiko2_primitives::Proof;
use raiko2_runtime::{ProofArtifactRecord, RuntimeManager};

pub(crate) struct ProofArtifactMaterial {
    pub(crate) record: ProofArtifactRecord,
    pub(crate) proof: Proof,
}

pub(crate) async fn load_proof_artifact_material(
    runtime: &RuntimeManager,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> Result<Option<ProofArtifactMaterial>> {
    load_proof_artifact_material_inner(runtime, network_pair, pipeline_key, route, proof_ref, true)
        .await
}

pub(crate) async fn load_aggregate_input_artifact_material(
    runtime: &RuntimeManager,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> Result<Option<ProofArtifactMaterial>> {
    load_proof_artifact_material_inner(runtime, network_pair, pipeline_key, route, proof_ref, false)
        .await
}

async fn load_proof_artifact_material_inner(
    runtime: &RuntimeManager,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
    require_proof_payload: bool,
) -> Result<Option<ProofArtifactMaterial>> {
    let Some(record) = runtime
        .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
        .await
        .context("failed to load active proof artifact registration")?
    else {
        return Ok(None);
    };
    let object = match runtime
        .read_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref)
        .await
    {
        Ok(Some(object)) => object,
        Ok(None) => return Ok(None),
        Err(err) => return Err(err).context("failed to read proof artifact"),
    };
    let descriptor = object.descriptor();
    if record.descriptor() != descriptor {
        return Ok(None);
    }
    if runtime
        .proof_artifact_descriptor_is_invalidated(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            &descriptor,
        )
        .await
        .context("failed to check proof artifact invalidation state")?
    {
        return Ok(None);
    }

    let proof: Proof = match serde_json::from_slice(&object.bytes) {
        Ok(proof) => proof,
        Err(err) => {
            return Err(err)
                .with_context(|| format!("proof artifact {} is invalid JSON", object.proof_uri));
        }
    };

    if require_proof_payload && proof.proof.is_none() {
        anyhow::bail!("proof artifact {} has no proof payload", object.proof_uri);
    }

    let still_active = runtime
        .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
        .await
        .context("failed to recheck active proof artifact registration")?
        .is_some_and(|current| current.descriptor() == descriptor);
    if !still_active {
        return Ok(None);
    }

    Ok(Some(ProofArtifactMaterial { record, proof }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raiko2_runtime::{
        MemoryProofArtifactStore, ProofArtifactDeleteResult, ProofArtifactKey, ProofArtifactObject,
        ProofArtifactPrefix, ProofArtifactPutResult, ProofArtifactRegistration, ProofArtifactStore,
        RuntimeStateObject, RuntimeStateWriteResult,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct PauseAfterInvalidationCheckStore {
        inner: MemoryProofArtifactStore,
        checks: AtomicUsize,
        check_completed: tokio::sync::Notify,
        allow_return: tokio::sync::Notify,
    }

    #[async_trait]
    impl ProofArtifactStore for PauseAfterInvalidationCheckStore {
        fn environment(&self) -> &str {
            self.inner.environment()
        }

        fn namespace(&self) -> &str {
            self.inner.namespace()
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }

        async fn put_if_absent(
            &self,
            key: &ProofArtifactKey,
            bytes: &[u8],
        ) -> Result<ProofArtifactPutResult> {
            self.inner.put_if_absent(key, bytes).await
        }

        async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
            self.inner.get(key).await
        }

        async fn get_prefix(
            &self,
            key: &ProofArtifactKey,
            max_bytes: usize,
        ) -> Result<Option<ProofArtifactPrefix>> {
            self.inner.get_prefix(key, max_bytes).await
        }

        async fn mark_invalidated(
            &self,
            key: &ProofArtifactKey,
            generation: Option<i64>,
            content_hash: &str,
        ) -> Result<()> {
            self.inner
                .mark_invalidated(key, generation, content_hash)
                .await
        }

        async fn is_invalidated(
            &self,
            key: &ProofArtifactKey,
            generation: Option<i64>,
            content_hash: &str,
        ) -> Result<bool> {
            let invalidated = self
                .inner
                .is_invalidated(key, generation, content_hash)
                .await?;
            if self.checks.fetch_add(1, Ordering::SeqCst) == 0 {
                self.check_completed.notify_one();
                self.allow_return.notified().await;
            }
            Ok(invalidated)
        }

        async fn delete(
            &self,
            key: &ProofArtifactKey,
            generation: Option<i64>,
            expected_content_hash: &str,
        ) -> Result<ProofArtifactDeleteResult> {
            self.inner
                .delete(key, generation, expected_content_hash)
                .await
        }

        async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
            self.inner.load_runtime_state().await
        }

        async fn store_runtime_state(
            &self,
            bytes: &[u8],
            expected_generation: Option<i64>,
        ) -> Result<RuntimeStateWriteResult> {
            self.inner
                .store_runtime_state(bytes, expected_generation)
                .await
        }
    }

    #[tokio::test]
    async fn stale_reconciliation_preserves_replacement_registration() -> Result<()> {
        let namespace = format!("proof-artifact-reconciliation-{}", uuid::Uuid::new_v4());
        let store = Arc::new(PauseAfterInvalidationCheckStore {
            inner: MemoryProofArtifactStore::new("test".to_string(), namespace.clone())?,
            checks: AtomicUsize::new(0),
            check_completed: tokio::sync::Notify::new(),
            allow_return: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::new_with_artifact_store(
            std::env::temp_dir().join(namespace),
            store.clone(),
        )?);
        let network_pair = "taiko_dev/ethereum";
        let pipeline_key = PipelineKey::ShastaNative;
        let route = pipeline_key.route();
        let proof_ref = "proof-ref";
        let old_bytes = serde_json::to_vec(&Proof {
            proof: Some("0x01".to_string()),
            ..Proof::default()
        })?;
        let old = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref, &old_bytes)
            .await?
            .object()
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key,
                route,
                proof_uri: old.proof_uri.clone(),
                content_hash: old.content_hash.clone(),
                generation: old.generation,
            })
            .await?;

        let loading_runtime = Arc::clone(&runtime);
        let loading = tokio::spawn(async move {
            load_proof_artifact_material(
                &loading_runtime,
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            )
            .await
        });
        store.check_completed.notified().await;

        runtime
            .mark_proof_artifact_descriptor_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &old.descriptor(),
            )
            .await?;
        runtime
            .delete_proof_artifact(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                old.generation,
                &old.content_hash,
            )
            .await?;
        runtime
            .remove_proof_artifact(network_pair, pipeline_key, route, proof_ref)
            .await?;

        let new_bytes = serde_json::to_vec(&Proof {
            proof: Some("0x02".to_string()),
            ..Proof::default()
        })?;
        let replacement = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref, &new_bytes)
            .await?
            .object()
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key,
                route,
                proof_uri: replacement.proof_uri.clone(),
                content_hash: replacement.content_hash.clone(),
                generation: replacement.generation,
            })
            .await?;

        store.allow_return.notify_one();
        assert!(loading.await??.is_none());
        assert_eq!(
            runtime
                .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
                .await?
                .map(|record| record.descriptor()),
            Some(replacement.descriptor())
        );
        Ok(())
    }
}
