use super::{
    NamespaceOwnerLease, ProofArtifactKey, ProofArtifactObject, ProofArtifactPrefix,
    ProofArtifactPutResult, ProofArtifactStore, RuntimeStateObject, RuntimeStateWriteResult,
    content_hash, encode_component, validate_scope_component,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use google_cloud_storage::client::{Storage, StorageControl};
use google_cloud_storage::model_ext::ReadRange;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
struct ProofManifest {
    content_hash: String,
}

#[derive(Debug)]
pub struct GcsProofArtifactStore {
    environment: String,
    namespace: String,
    bucket_id: String,
    bucket_resource: String,
    prefix: String,
    storage: Storage,
    control: StorageControl,
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
        Ok(Self {
            environment,
            namespace,
            bucket_resource: format!("projects/_/buckets/{bucket_id}"),
            bucket_id,
            prefix: prefix.trim_matches('/').to_string(),
            storage,
            control,
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

    fn owner_name(&self) -> String {
        format!("{}/owner.owner.json", self.scope_prefix())
    }

    async fn read_named(&self, name: &str, uri: String) -> Result<Option<ProofArtifactObject>> {
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
        Ok(Some(ProofArtifactObject {
            proof_uri: uri,
            content_hash: content_hash(&bytes),
            generation: Some(generation),
            bytes,
        }))
    }

    async fn delete_named(&self, name: &str, generation: Option<i64>) -> Result<()> {
        let mut request = self
            .control
            .delete_object()
            .set_bucket(&self.bucket_resource)
            .set_object(name);
        if let Some(generation) = generation {
            request = request.set_if_generation_match(generation);
        }
        match request.send().await {
            Ok(()) => Ok(()),
            Err(error) if error.http_status_code() == Some(404) => Ok(()),
            Err(error) => Err(error).context("failed to conditionally delete GCS object"),
        }
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
        let content_upload = self
            .storage
            .write_object(
                &self.bucket_resource,
                &content_name,
                bytes::Bytes::copy_from_slice(bytes),
            )
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(content_upload).await {
            Ok(_) => {}
            Err(error) if error.http_status_code() == Some(412) => {}
            Err(error) => {
                return Err(error).context("failed to publish immutable GCS proof content");
            }
        }

        let manifest = serde_json::to_vec(&ProofManifest {
            content_hash: hash.clone(),
        })
        .context("failed to serialize proof manifest")?;
        let manifest_name = self.manifest_name(key);
        let manifest_upload = self
            .storage
            .write_object(
                &self.bucket_resource,
                &manifest_name,
                bytes::Bytes::from(manifest),
            )
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(manifest_upload).await {
            Ok(manifest) => Ok(ProofArtifactPutResult::Created(ProofArtifactObject {
                proof_uri: self.content_uri(key, &hash),
                content_hash: hash,
                generation: Some(manifest.generation),
                bytes: bytes.to_vec(),
            })),
            Err(error) if error.http_status_code() == Some(412) => {
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
            Err(error) => Err(error).context("failed to publish GCS proof manifest"),
        }
    }

    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        self.read_manifest_object(key).await
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
        let mut response = match self
            .storage
            .read_object(&self.bucket_resource, &content_name)
            .set_read_range(ReadRange::segment(0, length))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if error.http_status_code() == Some(404) => return Ok(None),
            Err(error) => return Err(error).context("failed to read GCS proof prefix"),
        };
        let mut bytes = Vec::with_capacity(max_bytes);
        while let Some(chunk) = response.next().await {
            bytes.extend_from_slice(&chunk.context("failed to stream GCS proof prefix")?);
        }
        Ok(Some(ProofArtifactPrefix {
            proof_uri: self.content_uri(key, &manifest.content_hash),
            generation: Some(manifest_generation),
            bytes,
        }))
    }

    async fn mark_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<()> {
        let name = self.invalidation_name(key, generation, content_hash);
        let upload = self
            .storage
            .write_object(&self.bucket_resource, &name, bytes::Bytes::new())
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(upload).await {
            Ok(_) => Ok(()),
            Err(error) if error.http_status_code() == Some(412) => Ok(()),
            Err(error) => Err(error).context("failed to publish GCS invalidation marker"),
        }
    }

    async fn is_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<bool> {
        let name = self.invalidation_name(key, generation, content_hash);
        match self
            .storage
            .read_object(&self.bucket_resource, &name)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if error.http_status_code() == Some(404) => Ok(false),
            Err(error) => Err(error).context("failed to read GCS invalidation marker"),
        }
    }

    async fn delete(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<()> {
        let Some((manifest, current_generation)) = self.read_manifest(key).await? else {
            return Ok(());
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
        let mut upload = self.storage.write_object(
            &self.bucket_resource,
            self.runtime_state_name(),
            bytes::Bytes::copy_from_slice(bytes),
        );
        upload = upload.set_if_generation_match(expected_generation.unwrap_or(0));
        match Box::pin(upload.send_buffered()).await {
            Ok(object) => Ok(RuntimeStateWriteResult::Stored {
                generation: Some(object.generation),
            }),
            Err(error) if error.http_status_code() == Some(412) => Ok(
                RuntimeStateWriteResult::Conflict(self.load_runtime_state().await?),
            ),
            Err(error) => {
                Err(error).context("failed to persist GCS runtime state; outcome is unknown")
            }
        }
    }

    async fn claim_namespace_owner(
        &self,
        owner_id: &str,
        now_secs: u64,
        lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        const MAX_ATTEMPTS: usize = 3;
        let name = self.owner_name();
        for attempt in 1..=MAX_ATTEMPTS {
            let current = self
                .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
                .await?;
            let (epoch, expected_generation) = if let Some(current) = current {
                let lease: NamespaceOwnerLease = serde_json::from_slice(&current.bytes)
                    .context("invalid GCS namespace owner record")?;
                anyhow::ensure!(
                    lease.owner_id == owner_id || lease.expires_at_secs <= now_secs,
                    "runtime namespace is owned by {} until {}",
                    lease.owner_id,
                    lease.expires_at_secs
                );
                let epoch = if lease.owner_id == owner_id {
                    lease.epoch
                } else {
                    lease.epoch.saturating_add(1)
                };
                (epoch, current.generation)
            } else {
                (1, None)
            };
            let lease = NamespaceOwnerLease {
                owner_id: owner_id.to_string(),
                epoch,
                expires_at_secs: now_secs.saturating_add(lease_secs),
                generation: None,
            };
            let bytes = serde_json::to_vec(&lease).context("serialize namespace owner")?;
            match Box::pin(
                self.storage
                    .write_object(&self.bucket_resource, &name, bytes::Bytes::from(bytes))
                    .set_if_generation_match(expected_generation.unwrap_or(0))
                    .send_buffered(),
            )
            .await
            {
                Ok(object) => {
                    return Ok(Some(NamespaceOwnerLease {
                        generation: Some(object.generation),
                        ..lease
                    }));
                }
                Err(error) if error.http_status_code() == Some(412) => {
                    let observed = self
                        .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
                        .await?;
                    if let Some(observed) = observed {
                        let mut observed_lease: NamespaceOwnerLease =
                            serde_json::from_slice(&observed.bytes)
                                .context("invalid GCS namespace owner record")?;
                        observed_lease.generation = observed.generation;
                        if observed_lease.owner_id == owner_id {
                            return Ok(Some(observed_lease));
                        }
                        anyhow::ensure!(
                            observed_lease.expires_at_secs <= now_secs,
                            "runtime namespace is owned by {} until {}",
                            observed_lease.owner_id,
                            observed_lease.expires_at_secs
                        );
                    }
                    if attempt < MAX_ATTEMPTS {
                        tokio::task::yield_now().await;
                        continue;
                    }
                    anyhow::bail!("runtime namespace owner changed during claim");
                }
                Err(error) => {
                    if let Ok(Some(observed)) = self
                        .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
                        .await
                    {
                        let mut observed_lease: NamespaceOwnerLease =
                            serde_json::from_slice(&observed.bytes)
                                .context("invalid GCS namespace owner record")?;
                        observed_lease.generation = observed.generation;
                        if observed_lease.owner_id == owner_id {
                            return Ok(Some(observed_lease));
                        }
                    }
                    return Err(error).context("failed to claim GCS namespace owner");
                }
            }
        }
        unreachable!("namespace owner claim loop returns on every terminal branch")
    }

    async fn renew_namespace_owner(
        &self,
        lease: &NamespaceOwnerLease,
        now_secs: u64,
        lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        let name = self.owner_name();
        let Some(current) = self
            .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
            .await?
        else {
            return Ok(None);
        };
        let current_lease: NamespaceOwnerLease =
            serde_json::from_slice(&current.bytes).context("invalid GCS namespace owner record")?;
        if current_lease.owner_id != lease.owner_id || current_lease.epoch != lease.epoch {
            return Ok(None);
        }
        let mut renewed = NamespaceOwnerLease {
            owner_id: lease.owner_id.clone(),
            epoch: lease.epoch,
            expires_at_secs: now_secs.saturating_add(lease_secs),
            generation: None,
        };
        let bytes = serde_json::to_vec(&renewed).context("serialize namespace owner renewal")?;
        let result = Box::pin(
            self.storage
                .write_object(&self.bucket_resource, &name, bytes::Bytes::from(bytes))
                .set_if_generation_match(current.generation.unwrap_or(0))
                .send_buffered(),
        )
        .await;
        match result {
            Ok(object) => {
                renewed.generation = Some(object.generation);
                Ok(Some(renewed))
            }
            Err(error) => {
                let observed = self
                    .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
                    .await;
                if let Ok(Some(observed)) = observed {
                    let mut observed_lease: NamespaceOwnerLease =
                        serde_json::from_slice(&observed.bytes)
                            .context("invalid GCS namespace owner record")?;
                    observed_lease.generation = observed.generation;
                    if observed_lease.owner_id == renewed.owner_id
                        && observed_lease.epoch == renewed.epoch
                        && observed_lease.expires_at_secs >= renewed.expires_at_secs
                    {
                        return Ok(Some(observed_lease));
                    }
                    if error.http_status_code() == Some(412) {
                        return Ok(None);
                    }
                }
                Err(error).context("failed to renew GCS namespace owner; outcome is unknown")
            }
        }
    }

    async fn verify_namespace_owner(
        &self,
        lease: &NamespaceOwnerLease,
        now_secs: u64,
    ) -> Result<bool> {
        let name = self.owner_name();
        let Some(current) = self
            .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
            .await?
        else {
            return Ok(false);
        };
        let current_lease: NamespaceOwnerLease =
            serde_json::from_slice(&current.bytes).context("invalid GCS namespace owner record")?;
        Ok(current_lease.owner_id == lease.owner_id
            && current_lease.epoch == lease.epoch
            && current.generation == lease.generation
            && current_lease.expires_at_secs > now_secs)
    }

    async fn release_namespace_owner(&self, lease: &NamespaceOwnerLease) -> Result<bool> {
        let name = self.owner_name();
        let Some(current) = self
            .read_named(&name, format!("gs://{}/{}", self.bucket_id, name))
            .await?
        else {
            return Ok(false);
        };
        let current_lease: NamespaceOwnerLease =
            serde_json::from_slice(&current.bytes).context("invalid GCS namespace owner record")?;
        if current_lease.owner_id != lease.owner_id
            || current_lease.epoch != lease.epoch
            || current.generation != lease.generation
        {
            return Ok(false);
        }
        let request = self
            .control
            .delete_object()
            .set_bucket(&self.bucket_resource)
            .set_object(&name)
            .set_if_generation_match(current.generation.unwrap_or(0));
        match request.send().await {
            Ok(()) => Ok(true),
            Err(error) if matches!(error.http_status_code(), Some(404 | 412)) => Ok(false),
            Err(error) => Err(error).context("failed to release GCS namespace owner"),
        }
    }
}
