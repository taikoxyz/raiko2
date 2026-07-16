use super::{
    ProofArtifactKey, ProofArtifactObject, ProofArtifactPrefix, ProofArtifactPutResult,
    ProofArtifactStore, content_hash, legacy_safe_component, safe_component,
    validate_environment_id,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use google_cloud_storage::client::{Storage, StorageControl};
use google_cloud_storage::model_ext::ReadRange;

#[derive(Debug)]
pub struct GcsProofArtifactStore {
    environment_id: String,
    bucket_id: String,
    bucket_resource: String,
    prefix: String,
    storage: Storage,
    control: StorageControl,
}

impl GcsProofArtifactStore {
    /// Creates a GCS-backed proof artifact store using application default credentials.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid configuration or when GCS clients cannot be initialized.
    pub async fn new(environment_id: String, bucket_id: String, prefix: String) -> Result<Self> {
        validate_environment_id(&environment_id)?;
        if bucket_id.trim().is_empty() {
            anyhow::bail!("runtime.artifact_store.bucket must not be empty");
        }
        // GCS enables aws-lc-rs while other HTTP clients can enable another rustls provider.
        // Select the GCS provider explicitly before constructing its clients.
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
            environment_id,
            bucket_resource: format!("projects/_/buckets/{bucket_id}"),
            bucket_id,
            prefix: prefix.trim_matches('/').to_string(),
            storage,
            control,
        })
    }

    fn object_name(&self, key: &ProofArtifactKey) -> String {
        self.object_name_with(key, safe_component)
    }

    fn legacy_object_name(&self, key: &ProofArtifactKey) -> String {
        self.object_name_with(key, legacy_safe_component)
    }

    fn object_name_with(&self, key: &ProofArtifactKey, encode: fn(&str) -> String) -> String {
        let suffix = format!(
            "{}/{}/{}/{}/{}.json",
            encode(&self.environment_id),
            encode(key.pipeline_key.as_str()),
            encode(&key.route.to_string()),
            encode(&key.network_pair),
            encode(&key.proof_ref)
        );
        if self.prefix.is_empty() {
            suffix
        } else {
            format!("{}/{suffix}", self.prefix)
        }
    }

    fn invalidation_object_name(&self, key: &ProofArtifactKey, content_hash: &str) -> String {
        format!(
            "{}.invalidated/{}",
            self.object_name(key),
            safe_component(content_hash)
        )
    }

    fn legacy_invalidation_object_name(
        &self,
        key: &ProofArtifactKey,
        content_hash: &str,
    ) -> String {
        format!(
            "{}.invalidated/{}",
            self.legacy_object_name(key),
            legacy_safe_component(content_hash)
        )
    }

    async fn load_object(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        let object_name = self.object_name(key);
        if let Some(object) = self
            .load_named_object(&object_name, self.proof_uri(key))
            .await?
        {
            return Ok(Some(object));
        }
        self.load_and_migrate_legacy_object(key, &object_name).await
    }

    async fn load_and_migrate_legacy_object(
        &self,
        key: &ProofArtifactKey,
        object_name: &str,
    ) -> Result<Option<ProofArtifactObject>> {
        let legacy_name = self.legacy_object_name(key);
        if legacy_name == object_name {
            return Ok(None);
        }
        let Some(legacy) = self
            .load_named_object(
                &legacy_name,
                format!("gs://{}/{legacy_name}", self.bucket_id),
            )
            .await?
        else {
            return Ok(None);
        };

        let upload = self
            .storage
            .write_object(
                &self.bucket_resource,
                object_name,
                bytes::Bytes::copy_from_slice(&legacy.bytes),
            )
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(upload).await {
            Ok(object) => Ok(Some(ProofArtifactObject {
                proof_uri: self.proof_uri(key),
                content_hash: legacy.content_hash,
                generation: Some(object.generation),
                bytes: legacy.bytes,
            })),
            Err(error) if error.http_status_code() == Some(412) => self
                .load_named_object(object_name, self.proof_uri(key))
                .await
                .context("canonical GCS artifact appeared during legacy migration"),
            Err(error) => Err(error).context("failed to migrate legacy GCS proof artifact"),
        }
    }

    async fn load_named_object(
        &self,
        object_name: &str,
        proof_uri: String,
    ) -> Result<Option<ProofArtifactObject>> {
        let mut response = match self
            .storage
            .read_object(&self.bucket_resource, object_name)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if error.http_status_code() == Some(404) => return Ok(None),
            Err(error) => return Err(error).context("failed to read GCS proof artifact"),
        };
        let generation = response.object().generation;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.next().await {
            bytes.extend_from_slice(&chunk.context("failed to stream GCS proof artifact")?);
        }
        Ok(Some(ProofArtifactObject {
            proof_uri,
            content_hash: content_hash(&bytes),
            generation: Some(generation),
            bytes,
        }))
    }

    async fn put_named_if_absent(
        &self,
        object_name: &str,
        proof_uri: String,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let upload = self
            .storage
            .write_object(
                &self.bucket_resource,
                object_name,
                bytes::Bytes::copy_from_slice(bytes),
            )
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(upload).await {
            Ok(object) => Ok(ProofArtifactPutResult::Created(ProofArtifactObject {
                proof_uri,
                content_hash: content_hash(bytes),
                generation: Some(object.generation),
                bytes: bytes.to_vec(),
            })),
            Err(error) if error.http_status_code() == Some(412) => {
                let existing = self
                    .load_named_object(object_name, proof_uri)
                    .await?
                    .context("GCS precondition failed but existing object is missing")?;
                if existing.content_hash == content_hash(bytes) {
                    Ok(ProofArtifactPutResult::AlreadyExists(existing))
                } else {
                    Ok(ProofArtifactPutResult::Conflict(existing))
                }
            }
            Err(error) => Err(error).context("failed to publish GCS proof artifact"),
        }
    }

    async fn publish_invalidation_marker(&self, object_name: &str) -> Result<()> {
        let upload = self
            .storage
            .write_object(&self.bucket_resource, object_name, bytes::Bytes::new())
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(upload).await {
            Ok(_) => Ok(()),
            Err(error) if error.http_status_code() == Some(412) => Ok(()),
            Err(error) => Err(error).context("failed to publish GCS proof invalidation marker"),
        }
    }

    async fn get_named_prefix(
        &self,
        object_name: &str,
        max_bytes: usize,
        proof_uri: String,
    ) -> Result<Option<ProofArtifactPrefix>> {
        let length =
            u64::try_from(max_bytes).context("proof artifact prefix limit is too large")?;
        let mut response = match self
            .storage
            .read_object(&self.bucket_resource, object_name)
            .set_read_range(ReadRange::segment(0, length))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if error.http_status_code() == Some(404) => return Ok(None),
            Err(error) => return Err(error).context("failed to read GCS proof artifact prefix"),
        };
        let generation = response.object().generation;
        let mut bytes = Vec::with_capacity(max_bytes);
        while let Some(chunk) = response.next().await {
            bytes.extend_from_slice(&chunk.context("failed to stream GCS proof artifact prefix")?);
        }
        anyhow::ensure!(
            bytes.len() <= max_bytes,
            "GCS proof artifact prefix exceeded requested limit"
        );
        Ok(Some(ProofArtifactPrefix {
            proof_uri,
            generation: Some(generation),
            bytes,
        }))
    }

    async fn invalidation_marker_exists(&self, object_name: &str) -> Result<bool> {
        match self
            .storage
            .read_object(&self.bucket_resource, object_name)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if error.http_status_code() == Some(404) => Ok(false),
            Err(error) => Err(error).context("failed to read GCS proof invalidation marker"),
        }
    }

    async fn delete_named_object(
        &self,
        object_name: &str,
        generation: Option<i64>,
    ) -> Result<bool> {
        let mut request = self
            .control
            .delete_object()
            .set_bucket(&self.bucket_resource)
            .set_object(object_name);
        if let Some(generation) = generation {
            request = request.set_if_generation_match(generation);
        }
        match request.send().await {
            Ok(()) => Ok(true),
            Err(error) if error.http_status_code() == Some(404) => Ok(false),
            Err(error) => Err(error).context("failed to delete GCS proof artifact"),
        }
    }
}

#[async_trait]
impl ProofArtifactStore for GcsProofArtifactStore {
    fn environment_id(&self) -> &str {
        &self.environment_id
    }

    fn proof_uri(&self, key: &ProofArtifactKey) -> String {
        format!("gs://{}/{}", self.bucket_id, self.object_name(key))
    }

    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let object_name = self.object_name(key);
        if let Some(existing) = self
            .load_named_object(&object_name, self.proof_uri(key))
            .await?
        {
            return if existing.content_hash == content_hash(bytes) {
                Ok(ProofArtifactPutResult::AlreadyExists(existing))
            } else {
                Ok(ProofArtifactPutResult::Conflict(existing))
            };
        }
        let legacy_name = self.legacy_object_name(key);
        if legacy_name != object_name {
            let legacy_uri = format!("gs://{}/{legacy_name}", self.bucket_id);
            let legacy = self
                .put_named_if_absent(&legacy_name, legacy_uri, bytes)
                .await?;
            if let ProofArtifactPutResult::Conflict(existing) = legacy {
                return Ok(ProofArtifactPutResult::Conflict(existing));
            }
        }
        self.put_named_if_absent(&object_name, self.proof_uri(key), bytes)
            .await
    }

    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        self.load_object(key).await
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
        let object_name = self.object_name(key);
        if let Some(prefix) = self
            .get_named_prefix(&object_name, max_bytes, self.proof_uri(key))
            .await?
        {
            return Ok(Some(prefix));
        }
        let legacy_name = self.legacy_object_name(key);
        if legacy_name == object_name {
            return Ok(None);
        }
        self.get_named_prefix(
            &legacy_name,
            max_bytes,
            format!("gs://{}/{legacy_name}", self.bucket_id),
        )
        .await
    }

    async fn mark_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<()> {
        let object_name = self.invalidation_object_name(key, content_hash);
        self.publish_invalidation_marker(&object_name).await?;
        let legacy_name = self.legacy_invalidation_object_name(key, content_hash);
        if legacy_name != object_name {
            self.publish_invalidation_marker(&legacy_name).await?;
        }
        Ok(())
    }

    async fn is_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<bool> {
        if self
            .invalidation_marker_exists(&self.invalidation_object_name(key, content_hash))
            .await?
        {
            return Ok(true);
        }
        let legacy_name = self.legacy_invalidation_object_name(key, content_hash);
        if legacy_name == self.invalidation_object_name(key, content_hash) {
            return Ok(false);
        }
        self.invalidation_marker_exists(&legacy_name).await
    }

    async fn delete(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<()> {
        let canonical = self.object_name(key);
        self.delete_named_object(&canonical, generation).await?;
        let legacy = self.legacy_object_name(key);
        if legacy != canonical {
            let legacy_uri = format!("gs://{}/{legacy}", self.bucket_id);
            if let Some(object) = self.load_named_object(&legacy, legacy_uri).await?
                && object.content_hash == expected_content_hash
            {
                self.delete_named_object(&legacy, object.generation).await?;
            }
        }
        Ok(())
    }
}
