use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use raiko2_primitives::Proof;
use raiko2_runtime::{ProofArtifactRecord, ProofArtifactRegistration, RuntimeManager};

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
    let object = match runtime
        .read_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref)
        .await
    {
        Ok(Some(object)) => object,
        Ok(None) => return Ok(None),
        Err(err) => return Err(err).context("failed to read proof artifact"),
    };
    if runtime
        .proof_artifact_is_invalidated(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            &object.content_hash,
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

    if proof.proof.is_none() {
        anyhow::bail!("proof artifact {} has no proof payload", object.proof_uri);
    }

    runtime
        .upsert_proof_artifact(ProofArtifactRegistration {
            network_pair: network_pair.to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key,
            route,
            proof_uri: object.proof_uri.clone(),
            content_hash: object.content_hash.clone(),
            generation: object.generation,
        })
        .await
        .context("failed to reconcile proof artifact registration")?;
    if runtime
        .proof_artifact_is_invalidated(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            &object.content_hash,
        )
        .await
        .context("failed to recheck proof artifact invalidation state")?
    {
        return Ok(None);
    }
    let record = runtime
        .get_proof_artifact_including_invalidated(network_pair, pipeline_key, route, proof_ref)
        .await
        .context("failed to load reconciled proof artifact registration")?;
    let Some(record) = record else {
        if runtime
            .proof_artifact_is_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &object.content_hash,
            )
            .await
            .context("failed to recheck missing proof artifact invalidation state")?
            || runtime
                .read_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref)
                .await
                .context("failed to recheck missing proof artifact object")?
                .is_none()
        {
            return Ok(None);
        }
        anyhow::bail!("reconciled proof artifact registration is missing");
    };
    Ok(Some(ProofArtifactMaterial { record, proof }))
}
