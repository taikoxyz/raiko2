use super::{
    ExactInvalidationResult, ProofArtifactConflict, ProofArtifactDeleteResult,
    ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject, ProofArtifactPrefix,
    ProofArtifactPutResult, ProofObjectStore, RuntimeStateObject, RuntimeStateStore,
    RuntimeStateWriteResult, RuntimeStoreScope, content_hash, encode_component,
    validate_scope_component,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use google_cloud_storage::client::{Storage, StorageControl};
use google_cloud_storage::model_ext::ReadRange;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize, Serialize)]
struct ProofManifest {
    content_hash: String,
}

#[derive(Debug)]
struct GcsObject {
    bytes: Vec<u8>,
    generation: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GcsCreateResult {
    Created(i64),
    AlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GcsWriteResult {
    Stored(i64),
    Conflict,
}

#[async_trait]
trait GcsTransport: std::fmt::Debug + Send + Sync {
    async fn read(&self, name: &str, prefix_bytes: Option<u64>) -> Result<Option<GcsObject>>;
    async fn create(&self, name: &str, bytes: &[u8]) -> Result<GcsCreateResult>;
    async fn write_if_generation(
        &self,
        name: &str,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<GcsWriteResult>;
    async fn delete_if_generation(
        &self,
        name: &str,
        generation: Option<i64>,
    ) -> Result<ProofArtifactDeleteResult>;
}

#[derive(Debug)]
struct GoogleGcsTransport {
    bucket_resource: String,
    storage: Storage,
    control: StorageControl,
}

#[derive(Debug)]
pub struct GcsProofArtifactStore {
    environment: String,
    namespace: String,
    bucket_id: String,
    prefix: String,
    transport: Arc<dyn GcsTransport>,
}

#[async_trait]
impl GcsTransport for GoogleGcsTransport {
    async fn read(&self, name: &str, prefix_bytes: Option<u64>) -> Result<Option<GcsObject>> {
        let mut request = self.storage.read_object(&self.bucket_resource, name);
        if let Some(length) = prefix_bytes {
            request = request.set_read_range(ReadRange::segment(0, length));
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.http_status_code() == Some(404) => return Ok(None),
            Err(error) => return Err(error).context("failed to read GCS object"),
        };
        let generation = response.object().generation;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.next().await {
            bytes.extend_from_slice(&chunk.context("failed to stream GCS object")?);
        }
        Ok(Some(GcsObject { bytes, generation }))
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<GcsCreateResult> {
        let upload = self
            .storage
            .write_object(
                &self.bucket_resource,
                name,
                bytes::Bytes::copy_from_slice(bytes),
            )
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(upload).await {
            Ok(object) => Ok(GcsCreateResult::Created(object.generation)),
            Err(error) if error.http_status_code() == Some(412) => {
                Ok(GcsCreateResult::AlreadyExists)
            }
            Err(error) => Err(error).context("failed to create GCS object"),
        }
    }

    async fn write_if_generation(
        &self,
        name: &str,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<GcsWriteResult> {
        let upload = self
            .storage
            .write_object(
                &self.bucket_resource,
                name,
                bytes::Bytes::copy_from_slice(bytes),
            )
            .set_if_generation_match(expected_generation.unwrap_or(0))
            .send_buffered();
        match Box::pin(upload).await {
            Ok(object) => Ok(GcsWriteResult::Stored(object.generation)),
            Err(error) if error.http_status_code() == Some(412) => Ok(GcsWriteResult::Conflict),
            Err(error) => Err(error).context("failed to conditionally write GCS object"),
        }
    }

    async fn delete_if_generation(
        &self,
        name: &str,
        generation: Option<i64>,
    ) -> Result<ProofArtifactDeleteResult> {
        let mut request = self
            .control
            .delete_object()
            .set_bucket(&self.bucket_resource)
            .set_object(name);
        if let Some(generation) = generation {
            request = request.set_if_generation_match(generation);
        }
        match request.send().await {
            Ok(()) => Ok(ProofArtifactDeleteResult::Removed),
            Err(error) if error.http_status_code() == Some(404) => {
                Ok(ProofArtifactDeleteResult::Missing)
            }
            Err(error) => Err(error).context("failed to conditionally delete GCS object"),
        }
    }
}

impl GcsProofArtifactStore {
    pub async fn new(
        environment: String,
        namespace: String,
        bucket_id: String,
        prefix: String,
    ) -> Result<Self> {
        validate_scope_component("runtime.environment", &environment)?;
        validate_scope_component("runtime.namespace", &namespace)?;
        anyhow::ensure!(
            !bucket_id.trim().is_empty(),
            "runtime.store.bucket must not be empty"
        );
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let storage = Storage::builder()
            .build()
            .await
            .context("failed to create GCS storage client")?;
        let control = StorageControl::builder()
            .build()
            .await
            .context("failed to create GCS control client")?;
        let bucket_resource = format!("projects/_/buckets/{bucket_id}");
        Ok(Self {
            environment,
            namespace,
            bucket_id,
            prefix: prefix.trim_matches('/').to_string(),
            transport: Arc::new(GoogleGcsTransport {
                bucket_resource,
                storage,
                control,
            }),
        })
    }

    #[cfg(test)]
    fn with_transport(
        environment: String,
        namespace: String,
        bucket_id: String,
        prefix: &str,
        transport: Arc<dyn GcsTransport>,
    ) -> Result<Self> {
        validate_scope_component("runtime.environment", &environment)?;
        validate_scope_component("runtime.namespace", &namespace)?;
        Ok(Self {
            environment,
            namespace,
            bucket_id,
            prefix: prefix.trim_matches('/').to_string(),
            transport,
        })
    }

    fn scope_prefix(&self) -> String {
        let scope = format!(
            "{}/{}",
            encode_component(&self.environment),
            encode_component(&self.namespace)
        );
        if self.prefix.is_empty() {
            scope
        } else {
            format!("{}/{scope}", self.prefix)
        }
    }

    fn artifact_base_name(&self, key: &ProofArtifactKey) -> String {
        format!(
            "{}/proofs/{}/{}/{}/{}",
            self.scope_prefix(),
            encode_component(key.pipeline_key.as_str()),
            encode_component(&key.route.to_string()),
            encode_component(&key.network_pair),
            encode_component(&key.proof_ref),
        )
    }

    fn manifest_name(&self, key: &ProofArtifactKey) -> String {
        format!("{}/manifest.manifest.json", self.artifact_base_name(key))
    }

    fn content_name(&self, key: &ProofArtifactKey, hash: &str) -> String {
        format!(
            "{}/content/{}.proof.json",
            self.artifact_base_name(key),
            encode_component(hash)
        )
    }

    fn content_uri(&self, key: &ProofArtifactKey, hash: &str) -> String {
        format!("gs://{}/{}", self.bucket_id, self.content_name(key, hash))
    }

    fn invalidation_name(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        hash: &str,
    ) -> String {
        format!(
            "{}/invalidated/{}-{}.tombstone",
            self.artifact_base_name(key),
            generation.map_or_else(|| "none".to_string(), |value| value.to_string()),
            encode_component(hash)
        )
    }

    fn runtime_state_name(&self) -> String {
        format!("{}/work/runtime-state.runtime.json", self.scope_prefix())
    }

    async fn read_named(&self, name: &str, uri: String) -> Result<Option<ProofArtifactObject>> {
        let Some(object) = self.transport.read(name, None).await? else {
            return Ok(None);
        };
        Ok(Some(ProofArtifactObject {
            proof_uri: uri,
            content_hash: content_hash(&object.bytes),
            generation: Some(object.generation),
            bytes: object.bytes,
        }))
    }

    async fn delete_named(
        &self,
        name: &str,
        generation: Option<i64>,
    ) -> Result<ProofArtifactDeleteResult> {
        self.transport.delete_if_generation(name, generation).await
    }

    async fn read_manifest(&self, key: &ProofArtifactKey) -> Result<Option<(ProofManifest, i64)>> {
        let name = self.manifest_name(key);
        let Some(object) = self
            .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
            .await?
        else {
            return Ok(None);
        };
        let manifest = serde_json::from_slice(&object.bytes).context("invalid proof manifest")?;
        Ok(Some((
            manifest,
            object
                .generation
                .context("GCS proof manifest has no generation")?,
        )))
    }

    async fn read_manifest_object(
        &self,
        key: &ProofArtifactKey,
    ) -> Result<Option<ProofArtifactObject>> {
        let Some((manifest, manifest_generation)) = self.read_manifest(key).await? else {
            return Ok(None);
        };
        let name = self.content_name(key, &manifest.content_hash);
        let mut object = self
            .read_named(&name, self.content_uri(key, &manifest.content_hash))
            .await?
            .context("proof manifest references missing content")?;
        anyhow::ensure!(
            object.content_hash == manifest.content_hash,
            "proof manifest content hash mismatch"
        );
        object.generation = Some(manifest_generation);
        Ok(Some(object))
    }
}

impl RuntimeStoreScope for GcsProofArtifactStore {
    fn environment(&self) -> &str {
        &self.environment
    }

    fn namespace(&self) -> &str {
        &self.namespace
    }

    fn backend_name(&self) -> &'static str {
        "gcs"
    }
}

#[async_trait]
impl ProofObjectStore for GcsProofArtifactStore {
    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let hash = content_hash(bytes);
        if let Some((existing_manifest, generation)) = self.read_manifest(key).await? {
            let descriptor = ProofArtifactDescriptor {
                proof_uri: self.content_uri(key, &existing_manifest.content_hash),
                content_hash: existing_manifest.content_hash,
                generation: Some(generation),
            };
            if descriptor.content_hash != hash {
                let content_name = self.content_name(key, &descriptor.content_hash);
                let object = self
                    .read_named(&content_name, descriptor.proof_uri.clone())
                    .await?
                    .map(|mut object| {
                        object.generation = descriptor.generation;
                        object
                    });
                if let Some(object) = object.as_ref() {
                    anyhow::ensure!(
                        object.content_hash == descriptor.content_hash,
                        "proof manifest content hash mismatch"
                    );
                }
                return Ok(ProofArtifactPutResult::Conflict(ProofArtifactConflict {
                    descriptor,
                    object,
                }));
            }

            let content_name = self.content_name(key, &hash);
            self.transport
                .create(&content_name, bytes)
                .await
                .context("failed to repair immutable GCS proof content")?;
            let existing = self
                .read_manifest_object(key)
                .await?
                .context("GCS manifest exists but content is missing after repair")?;
            return Ok(ProofArtifactPutResult::AlreadyExists(existing));
        }

        let content_name = self.content_name(key, &hash);
        self.transport
            .create(&content_name, bytes)
            .await
            .context("failed to publish immutable GCS proof content")?;

        let manifest = serde_json::to_vec(&ProofManifest {
            content_hash: hash.clone(),
        })
        .context("failed to serialize proof manifest")?;
        let manifest_name = self.manifest_name(key);
        match self
            .transport
            .create(&manifest_name, &manifest)
            .await
            .context("failed to publish GCS proof manifest")?
        {
            GcsCreateResult::Created(generation) => {
                Ok(ProofArtifactPutResult::Created(ProofArtifactObject {
                    proof_uri: self.content_uri(key, &hash),
                    content_hash: hash,
                    generation: Some(generation),
                    bytes: bytes.to_vec(),
                }))
            }
            GcsCreateResult::AlreadyExists => {
                let (existing_manifest, generation) = self
                    .read_manifest(key)
                    .await?
                    .context("GCS manifest precondition failed but manifest is missing")?;
                let descriptor = ProofArtifactDescriptor {
                    proof_uri: self.content_uri(key, &existing_manifest.content_hash),
                    content_hash: existing_manifest.content_hash,
                    generation: Some(generation),
                };
                if descriptor.content_hash == hash {
                    let existing = self
                        .read_manifest_object(key)
                        .await?
                        .context("GCS manifest precondition failed but manifest is missing")?;
                    Ok(ProofArtifactPutResult::AlreadyExists(existing))
                } else {
                    let content_name = self.content_name(key, &descriptor.content_hash);
                    let object = self
                        .read_named(&content_name, descriptor.proof_uri.clone())
                        .await?
                        .map(|mut object| {
                            object.generation = descriptor.generation;
                            object
                        });
                    if let Some(object) = object.as_ref() {
                        anyhow::ensure!(
                            object.content_hash == descriptor.content_hash,
                            "proof manifest content hash mismatch"
                        );
                    }
                    Ok(ProofArtifactPutResult::Conflict(ProofArtifactConflict {
                        descriptor,
                        object,
                    }))
                }
            }
        }
    }

    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        self.read_manifest_object(key).await
    }

    async fn get_descriptor(
        &self,
        key: &ProofArtifactKey,
    ) -> Result<Option<ProofArtifactDescriptor>> {
        let Some((manifest, generation)) = self.read_manifest(key).await? else {
            return Ok(None);
        };
        Ok(Some(ProofArtifactDescriptor {
            proof_uri: self.content_uri(key, &manifest.content_hash),
            content_hash: manifest.content_hash,
            generation: Some(generation),
        }))
    }

    async fn get_prefix(
        &self,
        key: &ProofArtifactKey,
        max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>> {
        anyhow::ensure!(
            max_bytes > 0,
            "proof artifact prefix limit must be positive"
        );
        let Some((manifest, manifest_generation)) = self.read_manifest(key).await? else {
            return Ok(None);
        };
        let length = u64::try_from(max_bytes).context("proof prefix limit is too large")?;
        let content_name = self.content_name(key, &manifest.content_hash);
        let object = self
            .transport
            .read(&content_name, Some(length))
            .await?
            .context("proof manifest references missing content")?;
        Ok(Some(ProofArtifactPrefix {
            proof_uri: self.content_uri(key, &manifest.content_hash),
            generation: Some(manifest_generation),
            bytes: object.bytes,
        }))
    }

    async fn invalidate_exact(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ExactInvalidationResult> {
        let Some((manifest, generation)) = self.read_manifest(key).await? else {
            return Ok(if self.is_invalidated(key, descriptor).await? {
                ExactInvalidationResult::AlreadyInvalidated
            } else {
                ExactInvalidationResult::Missing
            });
        };
        let current = ProofArtifactDescriptor {
            proof_uri: self.content_uri(key, &manifest.content_hash),
            content_hash: manifest.content_hash,
            generation: Some(generation),
        };
        if current != *descriptor {
            return Ok(ExactInvalidationResult::Stale);
        }
        let name = self.invalidation_name(key, descriptor.generation, &descriptor.content_hash);
        self.transport
            .create(&name, &[])
            .await
            .context("failed to publish GCS invalidation marker")?;
        match self
            .delete_named(&self.manifest_name(key), descriptor.generation)
            .await
        {
            Ok(deleted) => Ok(ExactInvalidationResult::Invalidated(deleted)),
            Err(delete_error) => match self.read_manifest(key).await {
                Ok(None) => Ok(ExactInvalidationResult::AlreadyInvalidated),
                Ok(Some((manifest, generation))) => {
                    let observed = ProofArtifactDescriptor {
                        proof_uri: self.content_uri(key, &manifest.content_hash),
                        content_hash: manifest.content_hash,
                        generation: Some(generation),
                    };
                    if observed == *descriptor {
                        Err(delete_error).context(
                            "proof manifest delete failed before commit; exact invalidation can be retried",
                        )
                    } else {
                        Ok(ExactInvalidationResult::Stale)
                    }
                }
                Err(read_error) => Err(delete_error).context(format!(
                    "proof manifest delete outcome is unknown and read-back failed: {read_error:#}"
                )),
            },
        }
    }

    async fn is_invalidated(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<bool> {
        let name = self.invalidation_name(key, descriptor.generation, &descriptor.content_hash);
        Ok(self.transport.read(&name, None).await?.is_some())
    }

    async fn delete_exact(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ProofArtifactDeleteResult> {
        let Some((manifest, current_generation)) = self.read_manifest(key).await? else {
            return Ok(ProofArtifactDeleteResult::Missing);
        };
        anyhow::ensure!(
            manifest.content_hash == descriptor.content_hash
                && self.content_uri(key, &manifest.content_hash) == descriptor.proof_uri,
            "proof artifact content changed before conditional delete"
        );
        anyhow::ensure!(
            descriptor.generation == Some(current_generation),
            "proof artifact generation changed before conditional delete"
        );
        self.delete_named(&self.manifest_name(key), Some(current_generation))
            .await
    }
}

#[async_trait]
impl RuntimeStateStore for GcsProofArtifactStore {
    async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
        let name = self.runtime_state_name();
        Ok(self
            .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
            .await?
            .map(|object| RuntimeStateObject {
                bytes: object.bytes,
                generation: object.generation,
            }))
    }

    async fn store_runtime_state(
        &self,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<RuntimeStateWriteResult> {
        match self
            .transport
            .write_if_generation(&self.runtime_state_name(), bytes, expected_generation)
            .await
            .context("failed to persist GCS runtime state; outcome is unknown")?
        {
            GcsWriteResult::Stored(generation) => Ok(RuntimeStateWriteResult::Stored {
                generation: Some(generation),
            }),
            GcsWriteResult::Conflict => Ok(RuntimeStateWriteResult::Conflict(
                self.load_runtime_state().await?,
            )),
        }
    }
}

#[cfg(test)]
#[path = "gcs_tests.rs"]
mod tests;
