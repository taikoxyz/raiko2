use anyhow::{Context, Result};
use raiko2_primitives::Proof;
use raiko2_runtime::{ProofArtifactRecord, RuntimeManager};
use tokio::fs;
use tracing::warn;

pub(crate) struct ProofArtifactMaterial {
    pub(crate) record: ProofArtifactRecord,
    pub(crate) proof: Proof,
}

pub(crate) async fn load_proof_artifact_material(
    runtime: &RuntimeManager,
    network_pair: &str,
    proof_ref: &str,
) -> Result<Option<ProofArtifactMaterial>> {
    let Some(record) = runtime
        .get_proof_artifact(network_pair, proof_ref)
        .await
        .context("failed to load proof artifact")?
    else {
        return Ok(None);
    };

    let bytes = match fs::read(&record.proof_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(
                network_pair = record.network_pair,
                proof_ref = record.proof_ref,
                proof_path = record.proof_path,
                error = %err,
                "proof artifact file cannot be read; treating it as a cache miss"
            );
            return Ok(None);
        }
    };

    let proof = match serde_json::from_slice(&bytes) {
        Ok(proof) => proof,
        Err(err) => {
            warn!(
                network_pair = record.network_pair,
                proof_ref = record.proof_ref,
                proof_path = record.proof_path,
                error = %err,
                "proof artifact file is invalid JSON; treating it as a cache miss"
            );
            return Ok(None);
        }
    };

    Ok(Some(ProofArtifactMaterial { record, proof }))
}
