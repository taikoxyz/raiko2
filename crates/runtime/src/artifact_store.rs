use anyhow::Result;
use async_trait::async_trait;
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Mutex;

mod gcs;

pub use gcs::GcsProofArtifactStore;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ProofArtifactDescriptor {
    pub proof_uri: String,
    pub content_hash: String,
    pub generation: Option<i64>,
}

impl ProofArtifactObject {
    #[must_use]
    pub fn descriptor(&self) -> ProofArtifactDescriptor {
        ProofArtifactDescriptor {
            proof_uri: self.proof_uri.clone(),
            content_hash: self.content_hash.clone(),
            generation: self.generation,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofArtifactDeleteResult {
    Removed,
    Missing,
}

impl ProofArtifactPutResult {
    #[must_use]
    pub const fn object(&self) -> &ProofArtifactObject {
        match self {
            Self::Created(object) | Self::AlreadyExists(object) | Self::Conflict(object) => object,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeStateObject {
    pub bytes: Vec<u8>,
    pub generation: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeStateWriteResult {
    Stored { generation: Option<i64> },
    Conflict(Option<RuntimeStateObject>),
}

#[async_trait]
pub trait ProofArtifactStore: std::fmt::Debug + Send + Sync {
    fn environment(&self) -> &str;
    fn namespace(&self) -> &str;
    fn backend_name(&self) -> &'static str;

    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult>;
    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>>;
    async fn get_descriptor(
        &self,
        key: &ProofArtifactKey,
    ) -> Result<Option<ProofArtifactDescriptor>> {
        Ok(self.get(key).await?.map(|object| object.descriptor()))
    }
    async fn get_prefix(
        &self,
        key: &ProofArtifactKey,
        max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>>;
    async fn mark_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<()>;
    async fn is_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<bool>;
    async fn delete(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<ProofArtifactDeleteResult>;

    async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>>;

    async fn store_runtime_state(
        &self,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<RuntimeStateWriteResult>;
}

#[derive(Debug)]
pub struct MemoryProofArtifactStore {
    environment: String,
    namespace: String,
    inner: Mutex<MemoryStoreInner>,
}

#[derive(Debug, Default)]
struct MemoryStoreInner {
    next_generation: i64,
    manifests: HashMap<ProofArtifactKey, MemoryManifest>,
    contents: HashMap<(ProofArtifactKey, String), Vec<u8>>,
    invalidations: HashSet<(ProofArtifactKey, Option<i64>, String)>,
    runtime_state: Option<RuntimeStateObject>,
}

#[derive(Clone, Debug)]
struct MemoryManifest {
    content_hash: String,
    generation: i64,
}

impl MemoryProofArtifactStore {
    pub fn new(environment: String, namespace: String) -> Result<Self> {
        validate_scope_component("runtime.environment", &environment)?;
        validate_scope_component("runtime.namespace", &namespace)?;
        Ok(Self {
            environment,
            namespace,
            inner: Mutex::new(MemoryStoreInner::default()),
        })
    }

    const fn next_generation(inner: &mut MemoryStoreInner) -> i64 {
        inner.next_generation = inner.next_generation.saturating_add(1);
        inner.next_generation
    }

    fn content_uri(&self, key: &ProofArtifactKey, hash: &str) -> String {
        format!(
            "memory://{}/{}/proofs/{}/{}/{}/{}/{}.json",
            encode_component(&self.environment),
            encode_component(&self.namespace),
            encode_component(key.pipeline_key.as_str()),
            encode_component(&key.route.to_string()),
            encode_component(&key.network_pair),
            encode_component(&key.proof_ref),
            encode_component(hash),
        )
    }
}

#[async_trait]
impl ProofArtifactStore for MemoryProofArtifactStore {
    fn environment(&self) -> &str {
        &self.environment
    }

    fn namespace(&self) -> &str {
        &self.namespace
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }

    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let hash = content_hash(bytes);
        if let Some(existing) = inner.manifests.get(key).cloned() {
            return Ok(if existing.content_hash == hash {
                inner
                    .contents
                    .entry((key.clone(), hash.clone()))
                    .or_insert_with(|| bytes.to_vec());
                ProofArtifactPutResult::AlreadyExists(ProofArtifactObject {
                    proof_uri: self.content_uri(key, &hash),
                    content_hash: hash,
                    generation: Some(existing.generation),
                    bytes: bytes.to_vec(),
                })
            } else {
                let existing_bytes = inner
                    .contents
                    .get(&(key.clone(), existing.content_hash.clone()))
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("proof manifest references missing content"))?;
                ProofArtifactPutResult::Conflict(ProofArtifactObject {
                    proof_uri: self.content_uri(key, &existing.content_hash),
                    content_hash: existing.content_hash,
                    generation: Some(existing.generation),
                    bytes: existing_bytes,
                })
            });
        }
        inner
            .contents
            .entry((key.clone(), hash.clone()))
            .or_insert_with(|| bytes.to_vec());
        let object = ProofArtifactObject {
            proof_uri: self.content_uri(key, &hash),
            content_hash: hash,
            generation: Some(Self::next_generation(&mut inner)),
            bytes: bytes.to_vec(),
        };
        inner.manifests.insert(
            key.clone(),
            MemoryManifest {
                content_hash: object.content_hash.clone(),
                generation: object.generation.expect("memory generation"),
            },
        );
        Ok(ProofArtifactPutResult::Created(object))
    }

    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let Some(manifest) = inner.manifests.get(key) else {
            return Ok(None);
        };
        let bytes = inner
            .contents
            .get(&(key.clone(), manifest.content_hash.clone()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("proof manifest references missing content"))?;
        Ok(Some(ProofArtifactObject {
            proof_uri: self.content_uri(key, &manifest.content_hash),
            content_hash: manifest.content_hash.clone(),
            generation: Some(manifest.generation),
            bytes,
        }))
    }

    async fn get_descriptor(
        &self,
        key: &ProofArtifactKey,
    ) -> Result<Option<ProofArtifactDescriptor>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let Some(manifest) = inner.manifests.get(key) else {
            return Ok(None);
        };
        anyhow::ensure!(
            inner
                .contents
                .contains_key(&(key.clone(), manifest.content_hash.clone())),
            "proof manifest references missing content"
        );
        Ok(Some(ProofArtifactDescriptor {
            proof_uri: self.content_uri(key, &manifest.content_hash),
            content_hash: manifest.content_hash.clone(),
            generation: Some(manifest.generation),
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
        Ok(self.get(key).await?.map(|object| ProofArtifactPrefix {
            proof_uri: object.proof_uri,
            generation: object.generation,
            bytes: object.bytes.into_iter().take(max_bytes).collect(),
        }))
    }

    async fn mark_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        inner
            .invalidations
            .insert((key.clone(), generation, content_hash.to_string()));
        Ok(())
    }

    async fn is_invalidated(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<bool> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        Ok(inner
            .invalidations
            .contains(&(key.clone(), generation, content_hash.to_string())))
    }

    async fn delete(
        &self,
        key: &ProofArtifactKey,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<ProofArtifactDeleteResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let Some(current) = inner.manifests.get(key) else {
            return Ok(ProofArtifactDeleteResult::Missing);
        };
        anyhow::ensure!(
            Some(current.generation) == generation && current.content_hash == expected_content_hash,
            "proof artifact changed before conditional delete"
        );
        inner.manifests.remove(key);
        Ok(ProofArtifactDeleteResult::Removed)
    }

    async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        Ok(inner.runtime_state.clone())
    }

    async fn store_runtime_state(
        &self,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<RuntimeStateWriteResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        if inner
            .runtime_state
            .as_ref()
            .and_then(|state| state.generation)
            != expected_generation
        {
            return Ok(RuntimeStateWriteResult::Conflict(
                inner.runtime_state.clone(),
            ));
        }
        let generation = Self::next_generation(&mut inner);
        inner.runtime_state = Some(RuntimeStateObject {
            bytes: bytes.to_vec(),
            generation: Some(generation),
        });
        Ok(RuntimeStateWriteResult::Stored {
            generation: Some(generation),
        })
    }
}

pub fn validate_scope_component(name: &str, value: &str) -> Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{name} must not be empty");
    anyhow::ensure!(
        value.trim() == value,
        "{name} must not contain surrounding whitespace"
    );
    anyhow::ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "{name} may only contain ASCII letters, digits, '.', '_' and '-'"
    );
    Ok(())
}

pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

pub fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(char::from(byte));
        } else {
            // GCS client URLs encode '/' but intentionally leave '%' untouched. Using percent
            // escapes here would therefore make a literal `%2F` object name read as a slash.
            write!(&mut encoded, "~{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_pipeline::PipelineKey;

    fn key() -> ProofArtifactKey {
        let pipeline_key = PipelineKey::ShastaSp1;
        ProofArtifactKey {
            network_pair: "taiko_dev/ethereum".to_string(),
            pipeline_key,
            route: pipeline_key.route(),
            proof_ref: "proposal-1".to_string(),
        }
    }

    #[test]
    fn component_encoding_is_gcs_url_safe_and_unambiguous() {
        assert_eq!(encode_component("risc0/network"), "risc0~2Fnetwork");
        assert_eq!(
            encode_component("taiko_dev/taiko_dev_l1"),
            "taiko_dev~2Ftaiko_dev_l1"
        );
        assert_eq!(encode_component("already~escaped"), "already~7Eescaped");
        assert!(!encode_component("risc0/network").contains('%'));
    }

    #[tokio::test]
    async fn stale_delete_cannot_remove_new_manifest() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-a".into())?;
        let key = key();
        let first = store
            .put_if_absent(&key, b"proof-a")
            .await?
            .object()
            .clone();
        store
            .delete(&key, first.generation, &first.content_hash)
            .await?;

        let second = store
            .put_if_absent(&key, b"proof-b")
            .await?
            .object()
            .clone();
        assert_ne!(first.proof_uri, second.proof_uri);
        assert!(
            store
                .delete(&key, first.generation, &first.content_hash)
                .await
                .is_err()
        );
        assert_eq!(store.get(&key).await?, Some(second));
        Ok(())
    }

    #[tokio::test]
    async fn delete_reports_removed_then_missing() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-delete-result".into())?;
        let key = key();
        let object = store
            .put_if_absent(&key, b"proof-a")
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
    async fn different_content_does_not_replace_active_manifest() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-a".into())?;
        let key = key();
        let first = store
            .put_if_absent(&key, b"proof-a")
            .await?
            .object()
            .clone();
        let conflict = store.put_if_absent(&key, b"proof-b").await?;
        assert!(matches!(conflict, ProofArtifactPutResult::Conflict(_)));
        assert_eq!(store.get(&key).await?, Some(first));
        Ok(())
    }

    #[tokio::test]
    async fn invalidation_is_scoped_to_manifest_generation() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-a".into())?;
        let key = key();
        let first = store
            .put_if_absent(&key, b"deterministic-proof")
            .await?
            .object()
            .clone();
        store
            .mark_invalidated(&key, first.generation, &first.content_hash)
            .await?;
        store
            .delete(&key, first.generation, &first.content_hash)
            .await?;

        let second = store
            .put_if_absent(&key, b"deterministic-proof")
            .await?
            .object()
            .clone();
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
        Ok(())
    }

    #[tokio::test]
    async fn identical_put_repairs_missing_manifest_content() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-a".into())?;
        let key = key();
        let first = store
            .put_if_absent(&key, b"proof-a")
            .await?
            .object()
            .clone();
        store
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?
            .contents
            .remove(&(key.clone(), first.content_hash.clone()));
        assert!(store.get(&key).await.is_err());

        let repaired = store.put_if_absent(&key, b"proof-a").await?;
        assert!(matches!(repaired, ProofArtifactPutResult::AlreadyExists(_)));
        assert_eq!(store.get(&key).await?, Some(first));
        Ok(())
    }
}
