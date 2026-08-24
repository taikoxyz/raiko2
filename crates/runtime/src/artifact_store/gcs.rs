use super::{
    CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightDeleteResult, CanonicalPreflightDescriptor,
    CanonicalPreflightKeyV1, CanonicalPreflightObject, CanonicalPreflightPutResult,
    CanonicalPreflightStore, ExactDeleteResult, ProofArtifactConflict, ProofArtifactDeleteResult,
    ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject, ProofArtifactPrefix,
    ProofArtifactPutResult, ProofObjectStore, RuntimeStateObject, RuntimeStateStore,
    RuntimeStateWriteResult, RuntimeStoreScope, StartupCleanupMask, StartupCleanupReport,
    StartupCleanupScope, StartupCleanupScopeReport, content_hash, encode_component,
    validate_scope_component,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::{StreamExt, stream};
use google_cloud_storage::client::{Storage, StorageControl};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Deserialize, Serialize)]
struct ProofManifest {
    content_hash: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct CanonicalPreflightManifest {
    schema: u16,
    key_digest: alloy_primitives::B256,
    key: CanonicalPreflightKeyV1,
    content_hash: String,
    content_name: String,
}

#[derive(Debug)]
struct GcsObject {
    bytes: Vec<u8>,
    generation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GcsObjectMetadata {
    name: String,
    generation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GcsObjectPage {
    objects: Vec<GcsObjectMetadata>,
    next_page_token: Option<String>,
}

const RESET_DELETE_CONCURRENCY: usize = 16;
const RESET_LIST_PAGE_SIZE: i32 = 1_000;
const STARTUP_CLEANUP_DELETE_CONCURRENCY: usize = 64;

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
    async fn read(&self, name: &str) -> Result<Option<GcsObject>>;
    async fn list_prefix_page(
        &self,
        prefix: &str,
        page_token: Option<&str>,
    ) -> Result<GcsObjectPage>;
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
    async fn read(&self, name: &str) -> Result<Option<GcsObject>> {
        let mut response = match self
            .storage
            .read_object(&self.bucket_resource, name)
            .send()
            .await
        {
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

    async fn list_prefix_page(
        &self,
        prefix: &str,
        page_token: Option<&str>,
    ) -> Result<GcsObjectPage> {
        let mut request = self
            .control
            .list_objects()
            .set_parent(&self.bucket_resource)
            .set_prefix(prefix)
            .set_page_size(RESET_LIST_PAGE_SIZE);
        if let Some(page_token) = page_token {
            request = request.set_page_token(page_token);
        }
        let response = request
            .send()
            .await
            .context("failed to list GCS objects for runtime namespace reset")?;
        let objects = response
            .objects
            .into_iter()
            .map(|object| GcsObjectMetadata {
                name: object.name,
                generation: object.generation,
            })
            .collect();
        let next_page_token = if response.next_page_token.is_empty() {
            None
        } else {
            Some(response.next_page_token)
        };
        Ok(GcsObjectPage {
            objects,
            next_page_token,
        })
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

    fn runtime_state_name(&self) -> String {
        format!("{}/work/runtime-state.runtime.json", self.scope_prefix())
    }

    fn canonical_preflight_base_name(&self, key: &CanonicalPreflightKeyV1) -> Result<String> {
        Self::validate_canonical_preflight_key(key)?;
        Ok(format!(
            "{}/{:x}",
            self.canonical_preflight_version_prefix(key.schema),
            key.digest()?
        ))
    }

    fn canonical_preflight_version_prefix(&self, schema: u16) -> String {
        format!("{}/preflights/v{schema}", self.scope_prefix())
    }

    fn canonical_preflight_manifest_name(&self, key: &CanonicalPreflightKeyV1) -> Result<String> {
        Ok(format!(
            "{}/manifest.manifest.json",
            self.canonical_preflight_base_name(key)?
        ))
    }

    fn canonical_preflight_content_name(
        &self,
        key: &CanonicalPreflightKeyV1,
        hash: &str,
    ) -> Result<String> {
        Ok(format!(
            "{}/content/{}.preflight.bincode",
            self.canonical_preflight_base_name(key)?,
            encode_component(hash)
        ))
    }

    fn validate_canonical_preflight_key(key: &CanonicalPreflightKeyV1) -> Result<()> {
        anyhow::ensure!(
            key.schema == CANONICAL_PREFLIGHT_SCHEMA_V1,
            "unsupported canonical preflight key schema {}",
            key.schema
        );
        Ok(())
    }

    fn is_manifest_object(name: &str) -> bool {
        name.ends_with("/manifest.manifest.json")
    }

    fn validate_reset_object(scope_prefix: &str, object: &GcsObjectMetadata) -> Result<()> {
        anyhow::ensure!(
            object.name.starts_with(scope_prefix),
            "GCS namespace reset list returned object outside configured scope"
        );
        anyhow::ensure!(
            object.generation > 0,
            "GCS namespace reset list returned object without a generation"
        );
        Ok(())
    }

    async fn delete_reset_object(
        &self,
        scope_prefix: &str,
        object: GcsObjectMetadata,
    ) -> Result<()> {
        Self::validate_reset_object(scope_prefix, &object)?;
        match self
            .transport
            .delete_if_generation(&object.name, Some(object.generation))
            .await
            .with_context(|| {
                format!(
                    "failed to delete GCS runtime namespace object {}",
                    object.name
                )
            })? {
            ProofArtifactDeleteResult::Removed | ProofArtifactDeleteResult::Missing => Ok(()),
        }
    }

    async fn delete_reset_page(
        &self,
        scope_prefix: &str,
        phase: &'static str,
        objects: Vec<GcsObjectMetadata>,
    ) -> Result<usize> {
        if objects.is_empty() {
            return Ok(0);
        }
        let mut deletions = stream::iter(objects)
            .map(|object| self.delete_reset_object(scope_prefix, object))
            .buffer_unordered(RESET_DELETE_CONCURRENCY);
        let mut cleared = 0;
        while let Some(result) = deletions.next().await {
            result?;
            cleared += 1;
        }
        tracing::info!(
            phase,
            cleared,
            "cleared GCS runtime namespace reset objects"
        );
        Ok(cleared)
    }

    async fn clear_reset_prefix<F>(
        &self,
        scope_prefix: &str,
        list_prefix: &str,
        phase: &'static str,
        select: F,
    ) -> Result<usize>
    where
        F: Fn(&GcsObjectMetadata) -> bool,
    {
        let mut page_token = None;
        let mut cleared = 0;
        loop {
            let page = self
                .transport
                .list_prefix_page(list_prefix, page_token.as_deref())
                .await?;
            for object in &page.objects {
                Self::validate_reset_object(scope_prefix, object)?;
            }
            let objects = page.objects.into_iter().filter(&select).collect::<Vec<_>>();
            cleared += self.delete_reset_page(scope_prefix, phase, objects).await?;
            let Some(next_page_token) = page.next_page_token else {
                return Ok(cleared);
            };
            page_token = Some(next_page_token);
        }
    }

    async fn delete_cleanup_object(
        &self,
        scope_prefix: &str,
        object: GcsObjectMetadata,
    ) -> Result<ProofArtifactDeleteResult> {
        Self::validate_reset_object(scope_prefix, &object)?;
        self.transport
            .delete_if_generation(&object.name, Some(object.generation))
            .await
            .with_context(|| format!("failed to delete startup cleanup object {}", object.name))
    }

    async fn clear_cleanup_prefix<F>(
        &self,
        scope_prefix: &str,
        list_prefix: &str,
        select: F,
    ) -> Result<(usize, usize)>
    where
        F: Fn(&GcsObjectMetadata) -> bool,
    {
        let mut page_token = None;
        let mut matched = 0;
        let mut removed = 0;
        loop {
            let page = self
                .transport
                .list_prefix_page(list_prefix, page_token.as_deref())
                .await?;
            for object in &page.objects {
                Self::validate_reset_object(scope_prefix, object)?;
            }
            let objects = page.objects.into_iter().filter(&select).collect::<Vec<_>>();
            matched += objects.len();
            let mut deletions = stream::iter(objects)
                .map(|object| self.delete_cleanup_object(scope_prefix, object))
                .buffer_unordered(STARTUP_CLEANUP_DELETE_CONCURRENCY);
            while let Some(result) = deletions.next().await {
                if result? == ProofArtifactDeleteResult::Removed {
                    removed += 1;
                }
            }
            let Some(next_page_token) = page.next_page_token else {
                return Ok((matched, removed));
            };
            page_token = Some(next_page_token);
        }
    }

    async fn read_named(&self, name: &str, uri: String) -> Result<Option<ProofArtifactObject>> {
        let Some(object) = self.transport.read(name).await? else {
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

    async fn conflict_result(
        &self,
        key: &ProofArtifactKey,
        descriptor: ProofArtifactDescriptor,
    ) -> Result<ProofArtifactPutResult> {
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

    async fn read_canonical_preflight_manifest(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<(CanonicalPreflightManifest, i64)>> {
        Self::validate_canonical_preflight_key(key)?;
        let name = self.canonical_preflight_manifest_name(key)?;
        let Some(object) = self.transport.read(&name).await? else {
            return Ok(None);
        };
        let key_digest = key.digest()?;
        let validation = (|| -> Result<CanonicalPreflightManifest> {
            let manifest: CanonicalPreflightManifest = serde_json::from_slice(&object.bytes)
                .context("invalid canonical preflight manifest")?;
            anyhow::ensure!(
                manifest.schema == CANONICAL_PREFLIGHT_SCHEMA_V1
                    && manifest.key.schema == CANONICAL_PREFLIGHT_SCHEMA_V1,
                "canonical preflight manifest schema mismatch"
            );
            anyhow::ensure!(manifest.key == *key, "canonical preflight key mismatch");
            anyhow::ensure!(
                manifest.key_digest == key_digest && manifest.key.digest()? == key_digest,
                "canonical preflight key digest mismatch"
            );
            anyhow::ensure!(
                manifest.content_name
                    == self.canonical_preflight_content_name(key, &manifest.content_hash)?,
                "canonical preflight content object mismatch"
            );
            Ok(manifest)
        })();
        match validation {
            Ok(manifest) => Ok(Some((manifest, object.generation))),
            Err(error) => {
                self.remove_corrupt_canonical_preflight_manifest(&name, object.generation, &error)
                    .await?;
                Err(error)
            }
        }
    }

    async fn read_canonical_preflight_object(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<CanonicalPreflightObject>> {
        let Some((manifest, generation)) = self.read_canonical_preflight_manifest(key).await?
        else {
            return Ok(None);
        };
        let object = self.transport.read(&manifest.content_name).await?;
        let validation = (|| -> Result<GcsObject> {
            let object =
                object.context("canonical preflight manifest references missing content")?;
            anyhow::ensure!(
                content_hash(&object.bytes) == manifest.content_hash,
                "canonical preflight content hash mismatch"
            );
            Ok(object)
        })();
        let object = match validation {
            Ok(object) => object,
            Err(error) => {
                self.remove_corrupt_canonical_preflight_manifest(
                    &self.canonical_preflight_manifest_name(key)?,
                    generation,
                    &error,
                )
                .await?;
                return Err(error);
            }
        };
        Ok(Some(CanonicalPreflightObject {
            key_digest: manifest.key_digest,
            content_hash: manifest.content_hash,
            generation: Some(generation),
            bytes: object.bytes,
        }))
    }

    async fn remove_corrupt_canonical_preflight_manifest(
        &self,
        name: &str,
        generation: i64,
        validation_error: &anyhow::Error,
    ) -> Result<()> {
        self.transport
            .delete_if_generation(name, Some(generation))
            .await
            .with_context(|| {
                format!(
                    "failed to CAS-remove corrupt canonical preflight manifest after validation error: {validation_error:#}"
                )
            })?;
        Ok(())
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
impl CanonicalPreflightStore for GcsProofArtifactStore {
    async fn get_canonical_preflight(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<CanonicalPreflightObject>> {
        self.read_canonical_preflight_object(key).await
    }

    async fn put_canonical_preflight_if_absent(
        &self,
        key: &CanonicalPreflightKeyV1,
        bytes: &[u8],
    ) -> Result<CanonicalPreflightPutResult> {
        Self::validate_canonical_preflight_key(key)?;
        let key_digest = key.digest()?;
        let hash = content_hash(bytes);

        if let Some((manifest, generation)) = self.read_canonical_preflight_manifest(key).await? {
            let descriptor = CanonicalPreflightDescriptor {
                key_digest,
                content_hash: manifest.content_hash.clone(),
                generation: Some(generation),
            };
            if manifest.content_hash != hash {
                return Ok(CanonicalPreflightPutResult::Conflict(descriptor));
            }
            let content_name = self.canonical_preflight_content_name(key, &hash)?;
            self.transport
                .create(&content_name, bytes)
                .await
                .context("failed to repair immutable GCS canonical preflight content")?;
            let existing = self.read_canonical_preflight_object(key).await?.context(
                "canonical preflight manifest exists but content is missing after repair",
            )?;
            return Ok(CanonicalPreflightPutResult::AlreadyExists(existing));
        }

        let content_name = self.canonical_preflight_content_name(key, &hash)?;
        let content_creation = self
            .transport
            .create(&content_name, bytes)
            .await
            .context("failed to publish immutable GCS canonical preflight content")?;
        if content_creation == GcsCreateResult::AlreadyExists {
            let existing = self.transport.read(&content_name).await?.context(
                "immutable GCS canonical preflight content disappeared after create conflict",
            )?;
            anyhow::ensure!(
                content_hash(&existing.bytes) == hash,
                "immutable GCS canonical preflight content hash mismatch"
            );
        }

        let manifest = serde_json::to_vec(&CanonicalPreflightManifest {
            schema: CANONICAL_PREFLIGHT_SCHEMA_V1,
            key_digest,
            key: key.clone(),
            content_hash: hash.clone(),
            content_name,
        })
        .context("failed to serialize canonical preflight manifest")?;
        let manifest_name = self.canonical_preflight_manifest_name(key)?;
        match self
            .transport
            .create(&manifest_name, &manifest)
            .await
            .context("failed to publish GCS canonical preflight manifest")?
        {
            GcsCreateResult::Created(generation) => Ok(CanonicalPreflightPutResult::Created(
                CanonicalPreflightObject {
                    key_digest,
                    content_hash: hash,
                    generation: Some(generation),
                    bytes: bytes.to_vec(),
                },
            )),
            GcsCreateResult::AlreadyExists => {
                let existing = self.read_canonical_preflight_object(key).await?.context(
                    "GCS canonical preflight manifest precondition failed but manifest is missing",
                )?;
                if existing.content_hash == hash {
                    Ok(CanonicalPreflightPutResult::AlreadyExists(existing))
                } else {
                    Ok(CanonicalPreflightPutResult::Conflict(existing.descriptor()))
                }
            }
        }
    }

    async fn delete_canonical_preflight_exact(
        &self,
        key: &CanonicalPreflightKeyV1,
        descriptor: &CanonicalPreflightDescriptor,
    ) -> Result<CanonicalPreflightDeleteResult> {
        let key_digest = key.digest()?;
        if descriptor.key_digest != key_digest {
            return Ok(CanonicalPreflightDeleteResult::Stale);
        }
        let Some((manifest, generation)) = self.read_canonical_preflight_manifest(key).await?
        else {
            return Ok(CanonicalPreflightDeleteResult::Missing);
        };
        let current = CanonicalPreflightDescriptor {
            key_digest,
            content_hash: manifest.content_hash,
            generation: Some(generation),
        };
        if current != *descriptor {
            return Ok(CanonicalPreflightDeleteResult::Stale);
        }

        let manifest_name = self.canonical_preflight_manifest_name(key)?;
        match self
            .transport
            .delete_if_generation(&manifest_name, descriptor.generation)
            .await
        {
            Ok(ProofArtifactDeleteResult::Removed) => Ok(CanonicalPreflightDeleteResult::Removed),
            Ok(ProofArtifactDeleteResult::Missing) => Ok(CanonicalPreflightDeleteResult::Missing),
            Err(delete_error) => match self.read_canonical_preflight_manifest(key).await {
                Ok(None) => Ok(CanonicalPreflightDeleteResult::Removed),
                Ok(Some((observed, observed_generation))) => {
                    let observed = CanonicalPreflightDescriptor {
                        key_digest,
                        content_hash: observed.content_hash,
                        generation: Some(observed_generation),
                    };
                    if observed == *descriptor {
                        Err(delete_error).context(
                            "canonical preflight manifest delete failed before commit; exact deletion can be retried",
                        )
                    } else {
                        Ok(CanonicalPreflightDeleteResult::Stale)
                    }
                }
                Err(read_error) => Err(delete_error).context(format!(
                    "canonical preflight manifest delete outcome is unknown and read-back failed: {read_error:#}"
                )),
            },
        }
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
                return self.conflict_result(key, descriptor).await;
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
        let content_creation = self
            .transport
            .create(&content_name, bytes)
            .await
            .context("failed to publish immutable GCS proof content")?;
        if content_creation == GcsCreateResult::AlreadyExists {
            let existing = self
                .read_named(&content_name, self.content_uri(key, &hash))
                .await?
                .context("immutable GCS proof content disappeared after create conflict")?;
            anyhow::ensure!(
                existing.content_hash == hash,
                "immutable GCS proof content hash mismatch"
            );
        }

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
                    self.conflict_result(key, descriptor).await
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
        let Some(object) = self.read_manifest_object(key).await? else {
            return Ok(None);
        };
        Ok(Some(ProofArtifactPrefix {
            proof_uri: object.proof_uri,
            generation: object.generation,
            bytes: object.bytes.into_iter().take(max_bytes).collect(),
        }))
    }

    async fn delete_exact(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ExactDeleteResult> {
        let Some((manifest, current_generation)) = self.read_manifest(key).await? else {
            return Ok(ExactDeleteResult::Missing);
        };
        let current = ProofArtifactDescriptor {
            proof_uri: self.content_uri(key, &manifest.content_hash),
            content_hash: manifest.content_hash,
            generation: Some(current_generation),
        };
        if current != *descriptor {
            return Ok(ExactDeleteResult::Stale);
        }

        match self
            .delete_named(&self.manifest_name(key), Some(current_generation))
            .await
        {
            Ok(ProofArtifactDeleteResult::Removed) => Ok(ExactDeleteResult::Removed),
            Ok(ProofArtifactDeleteResult::Missing) => Ok(ExactDeleteResult::Missing),
            Err(delete_error) => match self.read_manifest(key).await {
                Ok(None) => Ok(ExactDeleteResult::Removed),
                Ok(Some((manifest, generation))) => {
                    let observed = ProofArtifactDescriptor {
                        proof_uri: self.content_uri(key, &manifest.content_hash),
                        content_hash: manifest.content_hash,
                        generation: Some(generation),
                    };
                    if observed == *descriptor {
                        Err(delete_error).context(
                            "proof manifest delete failed before commit; exact deletion can be retried",
                        )
                    } else {
                        Ok(ExactDeleteResult::Stale)
                    }
                }
                Err(read_error) => Err(delete_error).context(format!(
                    "proof manifest delete outcome is unknown and read-back failed: {read_error:#}"
                )),
            },
        }
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

    async fn cleanup_before_start(
        &self,
        scopes: StartupCleanupMask,
    ) -> Result<StartupCleanupReport> {
        let scope_prefix = format!("{}/", self.scope_prefix());
        let mut report = StartupCleanupReport::default();
        if scopes.contains(StartupCleanupMask::PROOF) {
            let started_at = Instant::now();
            let mut matched = 0;
            let mut removed = 0;
            let runtime_state_name = self.runtime_state_name();
            if let Some(runtime_state) = self.transport.read(&runtime_state_name).await? {
                matched += 1;
                if self
                    .transport
                    .delete_if_generation(&runtime_state_name, Some(runtime_state.generation))
                    .await
                    .context("failed to delete runtime state during proof startup cleanup")?
                    == ProofArtifactDeleteResult::Removed
                {
                    removed += 1;
                }
            }
            let proofs_prefix = format!("{scope_prefix}proofs/");
            let (manifest_matched, manifest_removed) = self
                .clear_cleanup_prefix(&scope_prefix, &proofs_prefix, |object| {
                    Self::is_manifest_object(&object.name)
                })
                .await?;
            matched += manifest_matched;
            removed += manifest_removed;
            report.scopes.push(StartupCleanupScopeReport {
                scope: StartupCleanupScope::Proof,
                matched,
                removed,
                failed: 0,
                duration: started_at.elapsed(),
            });
        }
        if scopes.contains(StartupCleanupMask::PREFLIGHT) {
            let started_at = Instant::now();
            let preflights_prefix = format!("{scope_prefix}preflights/");
            let (matched, removed) = self
                .clear_cleanup_prefix(&scope_prefix, &preflights_prefix, |object| {
                    Self::is_manifest_object(&object.name)
                })
                .await?;
            report.scopes.push(StartupCleanupScopeReport {
                scope: StartupCleanupScope::Preflight,
                matched,
                removed,
                failed: 0,
                duration: started_at.elapsed(),
            });
        }
        Ok(report)
    }

    async fn reset_namespace(&self) -> Result<usize> {
        let scope_prefix = format!("{}/", self.scope_prefix());
        let work_prefix = format!("{scope_prefix}work/");
        let proofs_prefix = format!("{scope_prefix}proofs/");

        // Remove the authoritative state before proof objects. If a later phase
        // fails, no live task record can reference an object it has already removed.
        let mut cleared = self
            .clear_reset_prefix(&scope_prefix, &work_prefix, "runtime_state", |_| true)
            .await?;
        cleared += self
            .clear_reset_prefix(&scope_prefix, &proofs_prefix, "proof_manifests", |object| {
                Self::is_manifest_object(&object.name)
            })
            .await?;
        cleared += self
            .clear_reset_prefix(&scope_prefix, &scope_prefix, "remaining_objects", |_| true)
            .await?;
        Ok(cleared)
    }
}

#[cfg(test)]
#[path = "gcs_tests.rs"]
mod tests;
