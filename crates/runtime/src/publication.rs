use crate::{ProofArtifactPutResult, ProofArtifactRegistration, RuntimeManager};
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
    /// Atomically advances a checkpointed proof through durable publication and registration.
    ///
    /// The canonical object remains first-write-wins. Publication is rejected when invalidation
    /// races either side of the local registration, and the shared outbox is cleared only after
    /// the canonical artifact is durable and registered.
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
            .publish_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref, proof_bytes)
            .await
            .context("failed to publish proof artifact")?;
        let artifact = publication.object();
        validate_canonical_proof(&artifact.bytes)?;

        if self
            .proof_artifact_is_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &artifact.content_hash,
            )
            .await
            .context("failed to check published proof invalidation state")?
        {
            return Err(ProofArtifactPublicationInvalidated {
                proof_ref: proof_ref.to_string(),
            }
            .into());
        }

        self.upsert_proof_artifact(ProofArtifactRegistration {
            network_pair: network_pair.to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key,
            route,
            proof_uri: artifact.proof_uri.clone(),
            content_hash: artifact.content_hash.clone(),
            generation: artifact.generation,
        })
        .await
        .context("failed to register proof artifact")?;

        if self
            .proof_artifact_is_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &artifact.content_hash,
            )
            .await
            .context("failed to recheck published proof invalidation state")?
        {
            self.mark_proof_artifact_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &artifact.content_hash,
            )
            .await
            .context("failed to retain local proof invalidation state")?;
            return Err(ProofArtifactPublicationInvalidated {
                proof_ref: proof_ref.to_string(),
            }
            .into());
        }

        self.remove_committed_pending_proof_publication(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
        .await
        .context("failed to clear pending proof publication")?;

        Ok(publication)
    }
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
            .upsert_pending_proof_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?;
        let first = runtime
            .commit_proof_artifact_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?
            .object()
            .clone();
        runtime
            .invalidate_pending_proof_publication(network_pair, pipeline, route, proof_ref)
            .await?;
        assert!(
            runtime
                .read_proof_artifact_bytes(network_pair, pipeline, route, proof_ref)
                .await?
                .is_none()
        );

        runtime
            .upsert_pending_proof_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?;
        let second = runtime
            .commit_proof_artifact_publication(network_pair, pipeline, route, proof_ref, proof)
            .await?
            .object()
            .clone();

        assert_ne!(first.generation, second.generation);
        assert_eq!(first.content_hash, second.content_hash);
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline, route, proof_ref)
                .await?
                .is_some()
        );
        Ok(())
    }
}
