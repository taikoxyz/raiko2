use anyhow::{Context, Result};
use async_trait::async_trait;
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod gcs;

pub use gcs::GcsProofArtifactStore;

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
    async fn mark_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<()>;
    async fn is_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<bool>;
    async fn delete(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<()>;
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
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .context("failed to resolve current directory")?
                .join(root)
        };
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create proof artifact root {}", root.display()))?;
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

    fn invalidation_path(&self, key: &ProofArtifactKey, content_hash: &str) -> PathBuf {
        self.path(key)
            .parent()
            .expect("proof artifact path always has a parent")
            .join(".invalidated")
            .join(safe_component(&key.proof_ref))
            .join(safe_component(content_hash))
    }

    fn lock_path(&self, key: &ProofArtifactKey) -> PathBuf {
        self.path(key)
            .parent()
            .expect("proof artifact path always has a parent")
            .join(".locks")
            .join(format!("{}.lock", safe_component(&key.proof_ref)))
    }

    async fn lock_key(&self, key: &ProofArtifactKey) -> Result<std::fs::File> {
        let path = self.lock_path(key);
        let parent = path
            .parent()
            .with_context(|| format!("path {} has no parent", path.display()))?;
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
        tokio::task::spawn_blocking(move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .with_context(|| {
                    format!("failed to open proof artifact lock {}", path.display())
                })?;
            file.lock()
                .with_context(|| format!("failed to lock proof artifact {}", path.display()))?;
            Ok(file)
        })
        .await
        .context("proof artifact lock task failed")?
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
        let _lock = self.lock_key(key).await?;

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

    async fn mark_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<()> {
        let path = self.invalidation_path(key, content_hash);
        let parent = path
            .parent()
            .with_context(|| format!("path {} has no parent", path.display()))?;
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => {
                file.sync_all().await.with_context(|| {
                    format!(
                        "failed to sync proof invalidation marker {}",
                        path.display()
                    )
                })?;
                sync_directory(parent).await
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to create proof invalidation marker {}",
                    path.display()
                )
            }),
        }
    }

    async fn is_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<bool> {
        match fs::metadata(self.invalidation_path(key, content_hash)).await {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("failed to read proof invalidation marker"),
        }
    }

    async fn delete(
        &self,
        key: &ProofArtifactKey,
        _generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<()> {
        let _lock = self.lock_key(key).await?;
        let Some(current) = self.load_path(&self.path(key)).await? else {
            return Ok(());
        };
        anyhow::ensure!(
            current.content_hash == expected_content_hash,
            "proof artifact content changed before delete"
        );
        match fs::remove_file(self.path(key)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("failed to delete proof artifact"),
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

pub(crate) fn content_hash(bytes: &[u8]) -> String {
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
                write!(&mut component, "~{byte:02x}").expect("writing to String should not fail");
            }
        }
    }
    component
}

fn legacy_safe_component(raw: &str) -> String {
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
        legacy_safe_component, safe_component,
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

    #[test]
    fn gcs_object_components_do_not_contain_percent_escapes() {
        let component = safe_component("risc0/network");

        assert_eq!(component, "risc0~2fnetwork");
        assert!(!component.contains('%'));
    }

    #[test]
    fn legacy_gcs_component_encoding_remains_available_for_read_migration() {
        assert_eq!(legacy_safe_component("risc0/network"), "risc0%2fnetwork");
        assert_eq!(safe_component("risc0/network"), "risc0~2fnetwork");
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
    async fn filesystem_invalidation_marker_is_content_bound() -> anyhow::Result<()> {
        let root = unique_root();
        let store = FilesystemProofArtifactStore::new("devnet-a".to_string(), root.clone())?;
        let publication = store.put_if_absent(&key(), b"proof").await?;
        let content_hash = publication.object().content_hash.clone();

        store.mark_invalidated(&key(), &content_hash).await?;

        assert!(store.is_invalidated(&key(), &content_hash).await?);
        assert!(
            !store
                .is_invalidated(&key(), "different-content-hash")
                .await?
        );
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

    #[tokio::test]
    async fn filesystem_delete_rejects_a_republished_generation() -> anyhow::Result<()> {
        let root = unique_root();
        let store = FilesystemProofArtifactStore::new("devnet-a".to_string(), root.clone())?;
        let old = store.put_if_absent(&key(), b"old-proof").await?;
        let old_hash = old.object().content_hash.clone();
        store.delete(&key(), None, &old_hash).await?;

        store.put_if_absent(&key(), b"new-proof").await?;
        let error = store
            .delete(&key(), None, &old_hash)
            .await
            .expect_err("stale deletion must not remove republished content");
        assert!(error.to_string().contains("content changed before delete"));
        assert_eq!(
            store.get(&key()).await?.expect("new artifact").bytes,
            b"new-proof"
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn filesystem_root_is_resolved_before_creation() -> anyhow::Result<()> {
        let relative = std::path::PathBuf::from(format!(
            ".raiko2-artifact-store-relative-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let store = FilesystemProofArtifactStore::new("devnet-a".to_string(), relative)?;
        assert!(store.root.is_absolute());
        assert!(store.root.is_dir());
        std::fs::remove_dir_all(&store.root)?;
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
