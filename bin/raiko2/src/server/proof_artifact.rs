use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use raiko2_primitives::Proof;
use raiko2_runtime::{ProofArtifactRecord, RuntimeManager};

pub(crate) struct ProofArtifactMaterial {
    pub(crate) record: ProofArtifactRecord,
    pub(crate) proof: Proof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProofArtifactPayload {
    Proposal,
    AggregateInput,
    Final,
}

impl ProofArtifactPayload {
    pub(crate) fn accepts(self, pipeline_key: PipelineKey, proof: &Proof) -> bool {
        match self {
            Self::Proposal => {
                proof.proof.is_some()
                    || (matches!(pipeline_key, PipelineKey::ShastaSp1)
                        && proof.quote.is_some()
                        && proof.input.is_some()
                        && proof.uuid.is_some()
                        && proof.extra_data.is_some())
            }
            Self::AggregateInput => raiko2_prover::validate_external_aggregate_proofs(
                pipeline_key,
                std::slice::from_ref(proof),
            )
            .is_ok(),
            Self::Final => proof.proof.is_some(),
        }
    }
}

pub(crate) async fn load_proof_artifact_material(
    runtime: &RuntimeManager,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
    expected_payload: ProofArtifactPayload,
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
    let proof: Proof = match serde_json::from_slice(&object.bytes) {
        Ok(proof) => proof,
        Err(err) => {
            return Err(err)
                .with_context(|| format!("proof artifact {} is invalid JSON", object.proof_uri));
        }
    };

    if !expected_payload.accepts(pipeline_key, &proof) {
        anyhow::bail!(
            "proof artifact {} is not a valid {expected_payload:?} payload for {pipeline_key}",
            object.proof_uri
        );
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
    use raiko2_runtime::test_support::{
        MemoryProofArtifactStore, ProofObjectStore, RuntimeStateObject, RuntimeStateStore,
        RuntimeStateWriteResult, RuntimeStoreScope,
    };
    use raiko2_runtime::{
        ExactDeleteResult, ProofArtifactKey, ProofArtifactObject, ProofArtifactPrefix,
        ProofArtifactPutResult, ProofArtifactRegistration,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn artifact_payload_contract_is_derived_from_task_kind() {
        let compressed_sp1 = Proof {
            input: Some(alloy_primitives::B256::ZERO),
            quote: Some(r#"{"Compressed":{}}"#.to_string()),
            uuid: Some("sp1-verifying-key".to_string()),
            extra_data: Some(serde_json::json!({ "shasta": {} })),
            ..Proof::default()
        };
        let final_proof = Proof {
            proof: Some("0xproof".to_string()),
            ..Proof::default()
        };

        assert!(ProofArtifactPayload::Proposal.accepts(PipelineKey::ShastaSp1, &compressed_sp1));
        assert!(
            ProofArtifactPayload::AggregateInput.accepts(PipelineKey::ShastaSp1, &compressed_sp1)
        );
        assert!(!ProofArtifactPayload::Final.accepts(PipelineKey::ShastaSp1, &compressed_sp1));
        assert!(!ProofArtifactPayload::Proposal.accepts(PipelineKey::ShastaRisc0, &compressed_sp1));
        assert!(ProofArtifactPayload::Proposal.accepts(PipelineKey::ShastaSp1, &final_proof));
        assert!(ProofArtifactPayload::Final.accepts(PipelineKey::ShastaSp1, &final_proof));
    }

    #[derive(Debug)]
    struct PauseAfterArtifactReadStore {
        inner: MemoryProofArtifactStore,
        checks: AtomicUsize,
        check_completed: tokio::sync::Notify,
        allow_return: tokio::sync::Notify,
    }

    #[async_trait]
    impl RuntimeStoreScope for PauseAfterArtifactReadStore {
        fn environment(&self) -> &str {
            self.inner.environment()
        }

        fn namespace(&self) -> &str {
            self.inner.namespace()
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }
    }

    #[async_trait]
    impl ProofObjectStore for PauseAfterArtifactReadStore {
        async fn put_if_absent(
            &self,
            key: &ProofArtifactKey,
            bytes: &[u8],
        ) -> Result<ProofArtifactPutResult> {
            self.inner.put_if_absent(key, bytes).await
        }

        async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
            let object = self.inner.get(key).await?;
            if self.checks.fetch_add(1, Ordering::SeqCst) == 0 {
                self.check_completed.notify_one();
                self.allow_return.notified().await;
            }
            Ok(object)
        }

        async fn get_descriptor(
            &self,
            key: &ProofArtifactKey,
        ) -> Result<Option<raiko2_runtime::ProofArtifactDescriptor>> {
            self.inner.get_descriptor(key).await
        }

        async fn get_prefix(
            &self,
            key: &ProofArtifactKey,
            max_bytes: usize,
        ) -> Result<Option<ProofArtifactPrefix>> {
            self.inner.get_prefix(key, max_bytes).await
        }

        async fn delete_exact(
            &self,
            key: &ProofArtifactKey,
            descriptor: &raiko2_runtime::ProofArtifactDescriptor,
        ) -> Result<ExactDeleteResult> {
            self.inner.delete_exact(key, descriptor).await
        }
    }

    #[async_trait]
    impl RuntimeStateStore for PauseAfterArtifactReadStore {
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
        let store = Arc::new(PauseAfterArtifactReadStore {
            inner: MemoryProofArtifactStore::new("test".to_string(), namespace.clone())?,
            checks: AtomicUsize::new(0),
            check_completed: tokio::sync::Notify::new(),
            allow_return: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
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
            .try_object()
            .expect("proof publication should materialize content")
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
                ProofArtifactPayload::Final,
            )
            .await
        });
        store.check_completed.notified().await;

        let record = runtime
            .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
            .await?
            .context("old proof record")?;
        let prepared = runtime.prepare_artifact_retention_batch(&[record]).await?;
        assert_eq!(
            runtime
                .finalize_proof_artifact_invalidation(&prepared.artifact_invalidations[0])
                .await?,
            ExactDeleteResult::Removed
        );
        runtime
            .finalize_terminal_task_retention_batch(&[], &prepared.artifact_invalidations, &[])
            .await?;

        let new_bytes = serde_json::to_vec(&Proof {
            proof: Some("0x02".to_string()),
            ..Proof::default()
        })?;
        let replacement = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref, &new_bytes)
            .await?
            .try_object()
            .expect("proof publication should materialize content")
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
