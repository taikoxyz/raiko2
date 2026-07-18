use super::{
    ProofArtifactDeleteResult, ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject,
    ProofArtifactPrefix, ProofArtifactPutResult, ProofArtifactStore, RuntimeStateObject,
    RuntimeStateWriteResult, content_hash, encode_component, validate_scope_component,
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

#[async_trait]
impl ProofArtifactStore for GcsProofArtifactStore {
    fn environment(&self) -> &str {
        &self.environment
    }

    fn namespace(&self) -> &str {
        &self.namespace
    }

    fn backend_name(&self) -> &'static str {
        "gcs"
    }

    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let hash = content_hash(bytes);
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
                let existing = self
                    .read_manifest_object(key)
                    .await?
                    .context("GCS manifest precondition failed but manifest is missing")?;
                Ok(if existing.content_hash == hash {
                    ProofArtifactPutResult::AlreadyExists(existing)
                } else {
                    ProofArtifactPutResult::Conflict(existing)
                })
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
        let content_name = self.content_name(key, &manifest.content_hash);
        self.transport
            .read(&content_name, Some(1))
            .await?
            .context("proof manifest references missing content")?;
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

    async fn mark_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<()> {
        let name = self.invalidation_name(key, generation, content_hash);
        self.transport
            .create(&name, &[])
            .await
            .context("failed to publish GCS invalidation marker")?;
        Ok(())
    }

    async fn is_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<bool> {
        let name = self.invalidation_name(key, generation, content_hash);
        Ok(self.transport.read(&name, None).await?.is_some())
    }

    async fn delete(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<ProofArtifactDeleteResult> {
        let Some((manifest, current_generation)) = self.read_manifest(key).await? else {
            return Ok(ProofArtifactDeleteResult::Missing);
        };
        anyhow::ensure!(
            manifest.content_hash == expected_content_hash,
            "proof artifact content changed before conditional delete"
        );
        anyhow::ensure!(
            generation == Some(current_generation),
            "proof artifact generation changed before conditional delete"
        );
        self.delete_named(&self.manifest_name(key), Some(current_generation))
            .await
    }

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
mod tests {
    use super::*;
    use raiko2_pipeline::PipelineKey;
    use std::{collections::BTreeMap, sync::Mutex};

    #[derive(Debug, Default)]
    struct FakeGcsTransport {
        objects: Mutex<BTreeMap<String, GcsObject>>,
        next_generation: Mutex<i64>,
    }

    impl FakeGcsTransport {
        fn generation(&self) -> Result<i64> {
            let mut next = self
                .next_generation
                .lock()
                .map_err(|_| anyhow::anyhow!("fake generation lock poisoned"))?;
            *next += 1;
            Ok(*next)
        }

        fn remove(&self, name: &str) -> Result<()> {
            self.objects
                .lock()
                .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?
                .remove(name);
            Ok(())
        }
    }

    #[async_trait]
    impl GcsTransport for FakeGcsTransport {
        async fn read(&self, name: &str, prefix_bytes: Option<u64>) -> Result<Option<GcsObject>> {
            let objects = self
                .objects
                .lock()
                .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
            let Some(object) = objects.get(name) else {
                return Ok(None);
            };
            let mut bytes = object.bytes.clone();
            if let Some(limit) = prefix_bytes {
                bytes.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            }
            Ok(Some(GcsObject {
                bytes,
                generation: object.generation,
            }))
        }

        async fn create(&self, name: &str, bytes: &[u8]) -> Result<GcsCreateResult> {
            let mut objects = self
                .objects
                .lock()
                .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
            if objects.contains_key(name) {
                return Ok(GcsCreateResult::AlreadyExists);
            }
            let generation = self.generation()?;
            objects.insert(
                name.to_string(),
                GcsObject {
                    bytes: bytes.to_vec(),
                    generation,
                },
            );
            Ok(GcsCreateResult::Created(generation))
        }

        async fn write_if_generation(
            &self,
            name: &str,
            bytes: &[u8],
            expected_generation: Option<i64>,
        ) -> Result<GcsWriteResult> {
            let mut objects = self
                .objects
                .lock()
                .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
            let current = objects.get(name).map(|object| object.generation);
            if current != expected_generation {
                return Ok(GcsWriteResult::Conflict);
            }
            let generation = self.generation()?;
            objects.insert(
                name.to_string(),
                GcsObject {
                    bytes: bytes.to_vec(),
                    generation,
                },
            );
            Ok(GcsWriteResult::Stored(generation))
        }

        async fn delete_if_generation(
            &self,
            name: &str,
            generation: Option<i64>,
        ) -> Result<ProofArtifactDeleteResult> {
            let mut objects = self
                .objects
                .lock()
                .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
            let Some(current) = objects.get(name) else {
                return Ok(ProofArtifactDeleteResult::Missing);
            };
            anyhow::ensure!(
                generation.is_none_or(|generation| generation == current.generation),
                "fake GCS generation precondition failed"
            );
            objects.remove(name);
            Ok(ProofArtifactDeleteResult::Removed)
        }
    }

    fn key() -> ProofArtifactKey {
        ProofArtifactKey {
            network_pair: "taiko_dev/ethereum".to_string(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            proof_ref: "proposal-1".to_string(),
        }
    }

    fn store(transport: Arc<FakeGcsTransport>) -> Result<GcsProofArtifactStore> {
        GcsProofArtifactStore::with_transport(
            "test".to_string(),
            "gcs-seam".to_string(),
            "bucket".to_string(),
            "runtime",
            transport,
        )
    }

    #[tokio::test]
    async fn same_hash_publication_repairs_missing_content_through_gcs_seam() -> Result<()> {
        let transport = Arc::new(FakeGcsTransport::default());
        let store = store(Arc::clone(&transport))?;
        let key = key();
        let proof = br#"{"proof":"0x01"}"#;
        let first = store.put_if_absent(&key, proof).await?.object().clone();

        transport.remove(&store.content_name(&key, &first.content_hash))?;
        let repaired = store.put_if_absent(&key, proof).await?;

        assert!(matches!(repaired, ProofArtifactPutResult::AlreadyExists(_)));
        assert_eq!(store.get(&key).await?.expect("repaired proof").bytes, proof);
        Ok(())
    }

    #[tokio::test]
    async fn prefix_read_rejects_manifest_with_missing_content() -> Result<()> {
        let transport = Arc::new(FakeGcsTransport::default());
        let store = store(Arc::clone(&transport))?;
        let key = key();
        let proof = br#"{"proof":"0x01"}"#;
        let first = store.put_if_absent(&key, proof).await?.object().clone();

        transport.remove(&store.content_name(&key, &first.content_hash))?;

        let error = store
            .get_prefix(&key, 64)
            .await
            .expect_err("dangling manifest must be reported as corruption");
        assert!(error.to_string().contains("missing content"));
        Ok(())
    }

    #[tokio::test]
    async fn generation_scoped_invalidation_allows_identical_republication() -> Result<()> {
        let transport = Arc::new(FakeGcsTransport::default());
        let store = store(transport)?;
        let key = key();
        let proof = br#"{"proof":"0x01"}"#;
        let first = store.put_if_absent(&key, proof).await?.object().clone();

        store
            .mark_invalidated(&key, first.generation, &first.content_hash)
            .await?;
        store
            .delete(&key, first.generation, &first.content_hash)
            .await?;
        let second = store.put_if_absent(&key, proof).await?.object().clone();

        assert_ne!(first.generation, second.generation);
        assert!(
            store
                .is_invalidated(&key, first.generation, &first.content_hash)
                .await?
        );
        assert!(
            !store
                .is_invalidated(&key, second.generation, &second.content_hash)
                .await?
        );
        assert!(
            store
                .delete(&key, first.generation, &first.content_hash)
                .await
                .is_err(),
            "a stale generation must not delete the replacement manifest"
        );
        assert_eq!(
            store
                .get(&key)
                .await?
                .expect("replacement proof")
                .generation,
            second.generation
        );
        Ok(())
    }

    #[tokio::test]
    async fn delete_reports_removed_then_missing_through_gcs_seam() -> Result<()> {
        let transport = Arc::new(FakeGcsTransport::default());
        let store = store(transport)?;
        let key = key();
        let object = store
            .put_if_absent(&key, br#"{"proof":"0x01"}"#)
            .await?
            .object()
            .clone();

        assert_eq!(
            store
                .delete(&key, object.generation, &object.content_hash)
                .await?,
            ProofArtifactDeleteResult::Removed
        );
        assert_eq!(
            store
                .delete(&key, object.generation, &object.content_hash)
                .await?,
            ProofArtifactDeleteResult::Missing
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_state_conflict_reads_back_the_observed_generation() -> Result<()> {
        let transport = Arc::new(FakeGcsTransport::default());
        let store = store(Arc::clone(&transport))?;
        let first = store.store_runtime_state(b"first", None).await?;
        let RuntimeStateWriteResult::Stored {
            generation: Some(first_generation),
        } = first
        else {
            anyhow::bail!("initial runtime state was not stored");
        };
        let concurrent = transport
            .write_if_generation(
                &store.runtime_state_name(),
                b"concurrent",
                Some(first_generation),
            )
            .await?;
        assert!(matches!(concurrent, GcsWriteResult::Stored(_)));

        let conflict = store
            .store_runtime_state(b"stale", Some(first_generation))
            .await?;
        let RuntimeStateWriteResult::Conflict(Some(observed)) = conflict else {
            anyhow::bail!("stale runtime state write did not return observed conflict");
        };
        assert_eq!(observed.bytes, b"concurrent");
        assert_ne!(observed.generation, Some(first_generation));
        Ok(())
    }
}
