use crate::{
    ProofArtifactLifecycle, ProofArtifactPutResult, ProofArtifactRegistration, RuntimeManager,
};
use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use raiko2_primitives::Proof;

#[derive(Debug)]
pub struct ProofArtifactPublicationInvalidated {
    proof_ref: String,
}

impl std::fmt::Display for ProofArtifactPublicationInvalidated {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "canonical proof artifact {} was invalidated during publication",
            self.proof_ref
        )
    }
}

impl std::error::Error for ProofArtifactPublicationInvalidated {}

impl RuntimeManager {
    /// Publishes a checkpointed proof and records a durable, unreadable pending lifecycle.
    ///
    /// The canonical object remains first-write-wins. Publication is rejected when invalidation
    /// races either side of the local registration. Root success activates the exact descriptor
    /// in a later single runtime-state CAS; this method never clears the shared outbox.
    ///
    /// # Errors
    ///
    /// Returns [`ProofArtifactPublicationInvalidated`] when invalidation wins the publication
    /// race, or the underlying storage/database error when the commit can be retried.
    pub async fn commit_proof_artifact_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof_bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        validate_canonical_proof(proof_bytes)?;
        let publication = self
            .publish_proof_artifact_bytes_locked(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                proof_bytes,
            )
            .await
            .context("failed to publish proof artifact")?;
        let artifact = publication
            .try_object()
            .context("canonical proof conflict references missing content")?;
        let descriptor = artifact.descriptor();
        validate_canonical_proof(&artifact.bytes)?;

        if self
            .proof_artifact_descriptor_is_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &descriptor,
            )
            .await
            .context("failed to check published proof invalidation state")?
        {
            return Err(ProofArtifactPublicationInvalidated {
                proof_ref: proof_ref.to_string(),
            }
            .into());
        }

        let registration = ProofArtifactRegistration {
            network_pair: network_pair.to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key,
            route,
            proof_uri: artifact.proof_uri.clone(),
            content_hash: artifact.content_hash.clone(),
            generation: artifact.generation,
        };
        self.register_publication_lifecycle(&publication, &registration)
            .await?;

        if self
            .proof_artifact_descriptor_is_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &descriptor,
            )
            .await
            .context("failed to recheck published proof invalidation state")?
        {
            self.mark_proof_artifact_descriptor_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &descriptor,
            )
            .await
            .context("failed to retain local proof invalidation state")?;
            return Err(publication_invalidated(proof_ref));
        }

        Ok(publication)
    }

    async fn register_publication_lifecycle(
        &self,
        publication: &ProofArtifactPutResult,
        registration: &ProofArtifactRegistration,
    ) -> Result<()> {
        let descriptor = registration.descriptor();
        let network_pair = &registration.network_pair;
        let pipeline_key = registration.pipeline_key;
        let route = registration.route;
        let proof_ref = &registration.proof_ref;
        match publication {
            ProofArtifactPutResult::Created(_) => {
                let lifecycle = self
                    .register_pending_proof_artifact(registration.clone())
                    .await
                    .context("failed to register pending proof artifact")?;
                if lifecycle == ProofArtifactLifecycle::Invalidated {
                    self.finalize_proof_artifact_invalidation(
                        network_pair,
                        pipeline_key,
                        route,
                        proof_ref,
                        descriptor.generation,
                        &descriptor.content_hash,
                    )
                    .await
                    .context("failed to invalidate detached proof manifest")?;
                    return Err(publication_invalidated(proof_ref));
                }
            }
            ProofArtifactPutResult::AlreadyExists(_) | ProofArtifactPutResult::Conflict(_) => {
                let registered = self
                    .get_proof_artifact_including_invalidated(
                        network_pair,
                        pipeline_key,
                        route,
                        proof_ref,
                    )
                    .await?
                    .is_some_and(|record| {
                        record.descriptor() == descriptor
                            && matches!(
                                record.lifecycle,
                                ProofArtifactLifecycle::Pending | ProofArtifactLifecycle::Active
                            )
                    });
                if !registered {
                    let recoverable_outbox = self
                        .get_recoverable_pending_proof_publication(
                            network_pair,
                            pipeline_key,
                            route,
                            proof_ref,
                        )
                        .await?
                        .is_some_and(|pending| {
                            crate::artifact_store::content_hash(&pending.bytes)
                                == descriptor.content_hash
                        });
                    if recoverable_outbox {
                        let lifecycle = self
                            .register_pending_proof_artifact(registration.clone())
                            .await
                            .context("failed to recover pending proof artifact lifecycle")?;
                        if lifecycle != ProofArtifactLifecycle::Invalidated {
                            return Ok(());
                        }
                    }
                    self.register_invalidated_proof_artifact(registration.clone())
                        .await
                        .context("failed to record orphan proof invalidation")?;
                    self.finalize_proof_artifact_invalidation(
                        network_pair,
                        pipeline_key,
                        route,
                        proof_ref,
                        descriptor.generation,
                        &descriptor.content_hash,
                    )
                    .await
                    .context("failed to invalidate orphan proof manifest")?;
                    return Err(publication_invalidated(proof_ref));
                }
            }
        }
        Ok(())
    }
}

fn publication_invalidated(proof_ref: &str) -> anyhow::Error {
    ProofArtifactPublicationInvalidated {
        proof_ref: proof_ref.to_string(),
    }
    .into()
}

fn validate_canonical_proof(bytes: &[u8]) -> Result<()> {
    let proof = serde_json::from_slice::<Proof>(bytes)
        .context("canonical proof artifact is not a valid normalized proof")?;
    anyhow::ensure!(
        proof.proof.is_some(),
        "canonical proof artifact has no proof payload"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryProofArtifactStore;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn invalid_canonical_conflict_does_not_commit_or_clear_outbox() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "runtime-invalid-canonical-conflict-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let runtime = RuntimeManager::new(root)?;
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-invalid-canonical";
        let valid_proof = br#"{"proof":"0xnew"}"#;

        runtime
            .upsert_pending_proof_publication(network_pair, pipeline, route, proof_ref, valid_proof)
            .await?;
        runtime
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, b"not-json")
            .await?;

        let error = runtime
            .commit_proof_artifact_publication(
                network_pair,
                pipeline,
                route,
                proof_ref,
                valid_proof,
            )
            .await
            .expect_err("invalid canonical proof must be rejected before commit");

        assert!(error.to_string().contains("canonical proof artifact"));
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline, route, proof_ref)
                .await?
                .is_none(),
            "invalid canonical proof must not be registered"
        );
        assert_eq!(
            runtime
                .get_pending_proof_publication(network_pair, pipeline, route, proof_ref,)
                .await?,
            Some(valid_proof.to_vec()),
            "retryable proof must remain in the publication outbox"
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalid_candidate_cannot_claim_canonical_manifest() -> Result<()> {
        let runtime = RuntimeManager::new("invalid-candidate")?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();

        runtime
            .commit_proof_artifact_publication(
                "taiko_dev/ethereum",
                pipeline,
                route,
                "proposal-invalid-candidate",
                b"not-json",
            )
            .await
            .expect_err("invalid proof must fail before publishing");

        assert!(
            runtime
                .read_proof_artifact_bytes(
                    "taiko_dev/ethereum",
                    pipeline,
                    route,
                    "proposal-invalid-candidate",
                )
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn restart_recovers_manifest_from_matching_durable_outbox() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".to_string(),
            "recover-manifest-outbox".to_string(),
        )?);
        let first = RuntimeManager::with_store(store.clone())?;
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-recover-manifest";
        let proof = br#"{"proof":"0x01"}"#;
        first
            .register_task(crate::TaskRegistration {
                task_id: "root-recover-manifest".into(),
                pipeline_key: Some(pipeline),
                route,
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: vec![proof_ref.into()],
                metadata: serde_json::json!({ "network_pair": network_pair }),
                request_fingerprint: None,
            })
            .await?;
        let owner = first
            .get_task("root-recover-manifest")
            .await?
            .expect("runtime owner")
            .incarnation_id;
        assert!(
            first
                .checkpoint_pending_proof_publication(
                    network_pair,
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    proof,
                )
                .await?
        );
        let created = first
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, proof)
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        assert!(
            first
                .get_proof_artifact_including_invalidated(network_pair, pipeline, route, proof_ref,)
                .await?
                .is_none()
        );

        let restarted = RuntimeManager::with_store(store)?;
        restarted.initialize().await?;
        let recovered = restarted
            .commit_proof_artifact_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();

        assert_eq!(recovered.generation, created.generation);
        let record = restarted
            .get_proof_artifact_including_invalidated(network_pair, pipeline, route, proof_ref)
            .await?
            .expect("recovered pending lifecycle");
        assert_eq!(record.lifecycle, ProofArtifactLifecycle::Pending);
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_committed_publication_can_be_reproved_identically() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "runtime-cancelled-committed-publication-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let runtime = RuntimeManager::new(root)?;
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-cancelled-after-commit";
        let proof = br#"{"proof":"0x01"}"#;

        runtime
            .register_task(crate::TaskRegistration {
                task_id: "root-cancelled-after-commit".into(),
                pipeline_key: Some(pipeline),
                route,
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: vec![proof_ref.into()],
                metadata: serde_json::json!({ "network_pair": network_pair }),
                request_fingerprint: None,
            })
            .await?;

        runtime
            .upsert_pending_proof_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?;
        let first = runtime
            .commit_proof_artifact_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        runtime
            .sync_status(
                "root-cancelled-after-commit",
                crate::RunnerStatus::Cancelled,
                None,
                None,
            )
            .await?;
        runtime
            .invalidate_pending_proof_publication(network_pair, pipeline, route, proof_ref)
            .await?;
        assert!(
            runtime
                .read_proof_artifact_bytes(network_pair, pipeline, route, proof_ref)
                .await?
                .is_none()
        );

        runtime.remove_task("root-cancelled-after-commit").await?;
        runtime
            .register_task(crate::TaskRegistration {
                task_id: "replacement-root".into(),
                pipeline_key: Some(pipeline),
                route,
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: vec![proof_ref.into()],
                metadata: serde_json::json!({ "network_pair": network_pair }),
                request_fingerprint: None,
            })
            .await?;

        runtime
            .upsert_pending_proof_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?;
        let second = runtime
            .commit_proof_artifact_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();

        assert_ne!(first.generation, second.generation);
        assert_eq!(first.content_hash, second.content_hash);
        let record = runtime
            .get_proof_artifact_including_invalidated(network_pair, pipeline, route, proof_ref)
            .await?
            .expect("republication lifecycle");
        assert_eq!(record.lifecycle, ProofArtifactLifecycle::Pending);
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline, route, proof_ref)
                .await?
                .is_none(),
            "republished proof remains unreadable until root success activates it"
        );
        Ok(())
    }
}
