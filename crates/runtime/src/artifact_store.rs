use anyhow::Result;
use async_trait::async_trait;
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeStateObject {
    pub bytes: Vec<u8>,
    pub generation: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NamespaceOwnerLease {
    pub owner_id: String,
    pub epoch: u64,
    pub expires_at_secs: u64,
    pub generation: Option<i64>,
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
    ) -> Result<()>;

    async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
        Ok(None)
    }

    async fn store_runtime_state(
        &self,
        _bytes: &[u8],
        _expected_generation: Option<i64>,
    ) -> Result<Option<i64>> {
        Ok(None)
    }

    async fn claim_namespace_owner(
        &self,
        _owner_id: &str,
        _now_secs: u64,
        _lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        Ok(None)
    }

    async fn renew_namespace_owner(
        &self,
        lease: &NamespaceOwnerLease,
        _now_secs: u64,
        _lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        Ok(Some(lease.clone()))
    }

    async fn verify_namespace_owner(
        &self,
        _lease: &NamespaceOwnerLease,
        _now_secs: u64,
    ) -> Result<bool> {
        Ok(true)
    }
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
    namespace_owner: Option<NamespaceOwnerLease>,
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
    ) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        if let Some(current) = inner.manifests.get(key) {
            anyhow::ensure!(
                Some(current.generation) == generation
                    && current.content_hash == expected_content_hash,
                "proof artifact changed before conditional delete"
            );
            inner.manifests.remove(key);
        }
        Ok(())
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
    ) -> Result<Option<i64>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        anyhow::ensure!(
            inner
                .runtime_state
                .as_ref()
                .and_then(|state| state.generation)
                == expected_generation,
            "runtime state generation changed"
        );
        let generation = Self::next_generation(&mut inner);
        inner.runtime_state = Some(RuntimeStateObject {
            bytes: bytes.to_vec(),
            generation: Some(generation),
        });
        Ok(Some(generation))
    }

    async fn claim_namespace_owner(
        &self,
        owner_id: &str,
        now_secs: u64,
        lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let epoch = if let Some(current) = inner.namespace_owner.as_ref() {
            anyhow::ensure!(
                current.owner_id == owner_id || current.expires_at_secs <= now_secs,
                "runtime namespace is owned by {} until {}",
                current.owner_id,
                current.expires_at_secs
            );
            if current.owner_id == owner_id {
                current.epoch
            } else {
                current.epoch.saturating_add(1)
            }
        } else {
            1
        };
        let lease = NamespaceOwnerLease {
            owner_id: owner_id.to_string(),
            epoch,
            expires_at_secs: now_secs.saturating_add(lease_secs),
            generation: Some(Self::next_generation(&mut inner)),
        };
        inner.namespace_owner = Some(lease.clone());
        Ok(Some(lease))
    }

    async fn renew_namespace_owner(
        &self,
        lease: &NamespaceOwnerLease,
        now_secs: u64,
        lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let Some(current) = inner.namespace_owner.as_ref() else {
            return Ok(None);
        };
        if current.owner_id != lease.owner_id || current.epoch != lease.epoch {
            return Ok(None);
        }
        let renewed = NamespaceOwnerLease {
            owner_id: lease.owner_id.clone(),
            epoch: lease.epoch,
            expires_at_secs: now_secs.saturating_add(lease_secs),
            generation: Some(Self::next_generation(&mut inner)),
        };
        inner.namespace_owner = Some(renewed.clone());
        Ok(Some(renewed))
    }

    async fn verify_namespace_owner(
        &self,
        lease: &NamespaceOwnerLease,
        now_secs: u64,
    ) -> Result<bool> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        Ok(inner.namespace_owner.as_ref().is_some_and(|current| {
            current.owner_id == lease.owner_id
                && current.epoch == lease.epoch
                && current.generation == lease.generation
                && current.expires_at_secs > now_secs
        }))
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
    use anyhow::Context as _;
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

    #[tokio::test]
    async fn memory_namespace_owner_lease_supports_expiry_and_takeover() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-owner".into())?;
        let first = store
            .claim_namespace_owner("owner-a", 100, 10)
            .await?
            .context("first owner must receive a lease")?;

        assert!(store.verify_namespace_owner(&first, 109).await?);
        assert!(
            store
                .claim_namespace_owner("owner-b", 109, 10)
                .await
                .is_err()
        );

        let renewed = store
            .renew_namespace_owner(&first, 105, 20)
            .await?
            .context("current owner must renew")?;
        assert_eq!(renewed.epoch, first.epoch);
        assert_eq!(renewed.expires_at_secs, 125);
        assert_ne!(renewed.generation, first.generation);
        assert!(!store.verify_namespace_owner(&first, 105).await?);
        assert!(store.verify_namespace_owner(&renewed, 124).await?);
        let mut superseded = first.clone();
        superseded.owner_id = "owner-b".to_string();
        assert!(
            store
                .renew_namespace_owner(&superseded, 106, 20)
                .await?
                .is_none(),
            "a different owner must not renew the active lease"
        );
        assert!(store.verify_namespace_owner(&renewed, 124).await?);

        let second = store
            .claim_namespace_owner("owner-b", 125, 10)
            .await?
            .context("expired owner must be replaceable")?;
        assert_eq!(second.epoch, renewed.epoch + 1);
        assert!(!store.verify_namespace_owner(&renewed, 125).await?);
        assert!(store.verify_namespace_owner(&second, 125).await?);

        let empty = MemoryProofArtifactStore::new("devnet".into(), "raiko2-no-owner".into())?;
        assert!(
            empty
                .renew_namespace_owner(&first, 100, 10)
                .await?
                .is_none()
        );
        Ok(())
    }
}
