use crate::{ProofArtifactPutResult, ProofArtifactRegistration, RuntimeManager};
use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};

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
        publication_generation: &str,
        proof_bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let publication = self
            .publish_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref, proof_bytes)
            .await
            .context("failed to publish proof artifact")?;
        let artifact = publication.object();

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

        self.remove_pending_proof_publication(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            publication_generation,
        )
        .await
        .context("failed to clear pending proof publication")?;

        Ok(publication)
    }
}
