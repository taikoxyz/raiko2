use anyhow::{Context, Result};
use async_trait::async_trait;
use google_cloud_storage::client::{Storage, StorageControl};
use google_cloud_storage::model_ext::ReadRange;
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofArtifactKey {
    pub network_pair: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub proof_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofArtifactObject {
    pub proof_uri: String,
    pub content_hash: String,
    pub generation: Option<i64>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofArtifactPrefix {
    pub proof_uri: String,
    pub generation: Option<i64>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofArtifactPutResult {
    Created(ProofArtifactObject),
    AlreadyExists(ProofArtifactObject),
    Conflict(ProofArtifactObject),
}

impl ProofArtifactPutResult {
    #[must_use]
    pub const fn object(&self) -> &ProofArtifactObject {
        match self {
            Self::Created(object) | Self::AlreadyExists(object) | Self::Conflict(object) => object,
        }
    }
}

#[async_trait]
pub trait ProofArtifactStore: std::fmt::Debug + Send + Sync {
    fn environment_id(&self) -> &str;
    fn proof_uri(&self, key: &ProofArtifactKey) -> String;
    /// Publishes an artifact only when its canonical key is absent.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot complete or verify the operation.
    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult>;
    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>>;
    /// Reads at most `max_bytes` from the beginning of an artifact.
    async fn get_prefix(
        &self,
        key: &ProofArtifactKey,
        max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>>;
    async fn delete(&self, key: &ProofArtifactKey, generation: Option<i64>) -> Result<()>;
}

#[derive(Debug)]
pub struct FilesystemProofArtifactStore {
    environment_id: String,
    root: PathBuf,
}

impl FilesystemProofArtifactStore {
    /// Creates a filesystem-backed artifact store rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns an error when the environment identifier is invalid or the root cannot be created.
    pub fn new(environment_id: String, root: PathBuf) -> Result<Self> {
        validate_environment_id(&environment_id)?;
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create proof artifact root {}", root.display()))?;
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .context("failed to resolve current directory")?
                .join(root)
        };
        Ok(Self {
            environment_id,
            root,
        })
    }

    fn path(&self, key: &ProofArtifactKey) -> PathBuf {
        self.root
            .join(safe_component(&self.environment_id))
            .join(safe_component(key.pipeline_key.as_str()))
            .join(safe_component(&key.route.to_string()))
            .join(safe_component(&key.network_pair))
            .join(format!("{}.json", safe_component(&key.proof_ref)))
    }

    async fn load_path(&self, path: &Path) -> Result<Option<ProofArtifactObject>> {
        let bytes = match fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read proof artifact {}", path.display()));
            }
        };
        Ok(Some(ProofArtifactObject {
            proof_uri: file_uri(path),
            content_hash: content_hash(&bytes),
            generation: None,
            bytes,
        }))
    }
}

#[async_trait]
impl ProofArtifactStore for FilesystemProofArtifactStore {
    fn environment_id(&self) -> &str {
        &self.environment_id
    }

    fn proof_uri(&self, key: &ProofArtifactKey) -> String {
        file_uri(&self.path(key))
    }

    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let path = self.path(key);
        let parent = path
            .parent()
            .with_context(|| format!("path {} has no parent", path.display()))?;
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create directory {}", parent.display()))?;

        let temp_path = atomic_temp_path(&path);
        let write_result = async {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .await
                .with_context(|| format!("failed to create temp file {}", temp_path.display()))?;
            file.write_all(bytes)
                .await
                .with_context(|| format!("failed to write temp file {}", temp_path.display()))?;
            file.sync_all()
                .await
                .with_context(|| format!("failed to sync temp file {}", temp_path.display()))?;
            drop(file);

            match fs::hard_link(&temp_path, &path).await {
                Ok(()) => {
                    fs::remove_file(&temp_path).await.with_context(|| {
                        format!("failed to remove temp file {}", temp_path.display())
                    })?;
                    sync_directory(parent).await?;
                    let object = self
                        .load_path(&path)
                        .await?
                        .context("created proof artifact is missing")?;
                    Ok(ProofArtifactPutResult::Created(object))
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&temp_path).await;
                    let existing = self
                        .load_path(&path)
                        .await?
                        .context("existing proof artifact disappeared")?;
                    if existing.content_hash == content_hash(bytes) {
                        Ok(ProofArtifactPutResult::AlreadyExists(existing))
                    } else {
                        Ok(ProofArtifactPutResult::Conflict(existing))
                    }
                }
                Err(error) => Err(error).with_context(|| {
                    format!(
                        "failed to publish temp file {} to {}",
                        temp_path.display(),
                        path.display()
                    )
                }),
            }
        }
        .await;
        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path).await;
        }
        write_result
    }

    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        self.load_path(&self.path(key)).await
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
        let path = self.path(key);
        let file = match fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to open proof artifact {}", path.display()));
            }
        };
        let mut bytes = Vec::with_capacity(max_bytes);
        let length =
            u64::try_from(max_bytes).context("proof artifact prefix limit is too large")?;
        file.take(length)
            .read_to_end(&mut bytes)
            .await
            .with_context(|| format!("failed to read proof artifact prefix {}", path.display()))?;
        Ok(Some(ProofArtifactPrefix {
            proof_uri: file_uri(&path),
            generation: None,
            bytes,
        }))
    }

    async fn delete(&self, key: &ProofArtifactKey, _generation: Option<i64>) -> Result<()> {
        match fs::remove_file(self.path(key)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("failed to delete proof artifact"),
        }
    }
}

#[derive(Clone, Debug)]
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
        let suffix = format!(
            "{}/{}/{}/{}/{}.json",
            safe_component(&self.environment_id),
            safe_component(key.pipeline_key.as_str()),
            safe_component(&key.route.to_string()),
            safe_component(&key.network_pair),
            safe_component(&key.proof_ref)
        );
        if self.prefix.is_empty() {
            suffix
        } else {
            format!("{}/{suffix}", self.prefix)
        }
    }

    async fn load_object(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        let object_name = self.object_name(key);
        let mut response = match self
            .storage
            .read_object(&self.bucket_resource, &object_name)
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
            proof_uri: self.proof_uri(key),
            content_hash: content_hash(&bytes),
            generation: Some(generation),
            bytes,
        }))
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
        let upload = self
            .storage
            .write_object(
                &self.bucket_resource,
                &object_name,
                bytes::Bytes::copy_from_slice(bytes),
            )
            .set_if_generation_match(0)
            .send_buffered();
        match Box::pin(upload).await {
            Ok(object) => Ok(ProofArtifactPutResult::Created(ProofArtifactObject {
                proof_uri: self.proof_uri(key),
                content_hash: content_hash(bytes),
                generation: Some(object.generation),
                bytes: bytes.to_vec(),
            })),
            Err(error) if error.http_status_code() == Some(412) => {
                let existing = self
                    .load_object(key)
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
        let length =
            u64::try_from(max_bytes).context("proof artifact prefix limit is too large")?;
        let mut response = match self
            .storage
            .read_object(&self.bucket_resource, &object_name)
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
            proof_uri: self.proof_uri(key),
            generation: Some(generation),
            bytes,
        }))
    }

    async fn delete(&self, key: &ProofArtifactKey, generation: Option<i64>) -> Result<()> {
        let mut request = self
            .control
            .delete_object()
            .set_bucket(&self.bucket_resource)
            .set_object(self.object_name(key));
        if let Some(generation) = generation {
            request = request.set_if_generation_match(generation);
        }
        match request.send().await {
            Ok(()) => Ok(()),
            Err(error) if error.http_status_code() == Some(404) => Ok(()),
            Err(error) => Err(error).context("failed to delete GCS proof artifact"),
        }
    }
}

/// Validates the immutable environment namespace used in artifact identities.
///
/// # Errors
///
/// Returns an error when the identifier is empty or has surrounding whitespace.
pub fn validate_environment_id(environment_id: &str) -> Result<()> {
    if environment_id.trim().is_empty() {
        anyhow::bail!("runtime.environment_id must not be empty");
    }
    if environment_id.trim() != environment_id {
        anyhow::bail!("runtime.environment_id must not contain surrounding whitespace");
    }
    Ok(())
}

fn content_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn atomic_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    path.with_file_name(format!(".{file_name}.tmp.{}.{}", process::id(), unique))
}

async fn sync_directory(path: &Path) -> Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::File::open(&path)
            .and_then(|directory| directory.sync_all())
            .with_context(|| format!("failed to sync directory {}", path.display()))
    })
    .await
    .context("directory sync task failed")?
}

fn safe_component(raw: &str) -> String {
    if raw == "." {
        return "%2e".to_string();
    }
    if raw == ".." {
        return "%2e%2e".to_string();
    }
    let mut component = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                component.push(char::from(byte));
            }
            _ => {
                write!(&mut component, "%{byte:02x}").expect("writing to String should not fail");
            }
        }
    }
    component
}

#[cfg(test)]
mod tests {
    use super::{
        FilesystemProofArtifactStore, ProofArtifactKey, ProofArtifactPutResult, ProofArtifactStore,
    };
    use raiko2_pipeline::PipelineKey;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "raiko2-artifact-store-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    fn key() -> ProofArtifactKey {
        ProofArtifactKey {
            network_pair: "taiko_dev/ethereum".to_string(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            proof_ref: "proposal_0xabc".to_string(),
        }
    }

    #[tokio::test]
    async fn filesystem_publication_is_create_only_and_idempotent() -> anyhow::Result<()> {
        let root = unique_root();
        let store = FilesystemProofArtifactStore::new("devnet-a".to_string(), root.clone())?;

        let created = store.put_if_absent(&key(), b"first").await?;
        assert!(matches!(created, ProofArtifactPutResult::Created(_)));
        let repeated = store.put_if_absent(&key(), b"first").await?;
        assert!(matches!(repeated, ProofArtifactPutResult::AlreadyExists(_)));
        let conflict = store.put_if_absent(&key(), b"second").await?;
        assert!(matches!(conflict, ProofArtifactPutResult::Conflict(_)));
        assert_eq!(store.get(&key()).await?.expect("artifact").bytes, b"first");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn filesystem_prefix_read_is_bounded() -> anyhow::Result<()> {
        let root = unique_root();
        let store = FilesystemProofArtifactStore::new("devnet-a".to_string(), root.clone())?;
        store.put_if_absent(&key(), b"prefix-and-more").await?;

        let prefix = store.get_prefix(&key(), 6).await?.expect("artifact prefix");
        assert_eq!(prefix.bytes, b"prefix");
        assert_eq!(prefix.proof_uri, store.proof_uri(&key()));
        assert_eq!(prefix.generation, None);
        assert!(store.get_prefix(&key(), 0).await.is_err());

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn environment_is_part_of_filesystem_uri() -> anyhow::Result<()> {
        let root = unique_root();
        let left = FilesystemProofArtifactStore::new("devnet-a".to_string(), root.clone())?;
        let right = FilesystemProofArtifactStore::new("devnet-b".to_string(), root.clone())?;
        assert_ne!(left.proof_uri(&key()), right.proof_uri(&key()));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn dot_components_cannot_escape_filesystem_root() -> anyhow::Result<()> {
        let root = unique_root();
        let store = FilesystemProofArtifactStore::new("..".to_string(), root.clone())?;
        let mut artifact_key = key();
        artifact_key.network_pair = "..".to_string();

        let publication = store.put_if_absent(&artifact_key, b"proof").await?;
        let uri = &publication.object().proof_uri;
        assert!(uri.contains("/%2e%2e/"), "{uri}");
        assert!(
            uri.starts_with(&format!("file://{}/", root.display())),
            "{uri}"
        );
        assert_eq!(
            store.get(&artifact_key).await?.expect("artifact").bytes,
            b"proof"
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
