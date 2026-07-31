use anyhow::Result;
use async_trait::async_trait;
use raiko2_pipeline::{
    PipelineKey, PipelineRoute,
    forks::shasta::preflight_cache::{
        CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightDescriptor,
        CanonicalPreflightInvalidateResult, CanonicalPreflightKeyV1, CanonicalPreflightObject,
        CanonicalPreflightPutResult, CanonicalPreflightStore,
    },
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::BitOr;
use std::sync::Mutex;
use std::time::Duration;

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
    Conflict(ProofArtifactConflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofArtifactConflict {
    pub descriptor: ProofArtifactDescriptor,
    pub object: Option<ProofArtifactObject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofArtifactDeleteResult {
    Removed,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactInvalidationResult {
    Invalidated(ProofArtifactDeleteResult),
    AlreadyInvalidated,
    Stale,
    Missing,
}

impl ProofArtifactPutResult {
    #[must_use]
    pub const fn try_object(&self) -> Option<&ProofArtifactObject> {
        match self {
            Self::Created(object) | Self::AlreadyExists(object) => Some(object),
            Self::Conflict(conflict) => conflict.object.as_ref(),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupCleanupScope {
    Proof,
    Preflight,
}

impl StartupCleanupScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proof => "proof",
            Self::Preflight => "preflight",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartupCleanupMask(u8);

impl StartupCleanupMask {
    pub const NONE: Self = Self(0);
    pub const PROOF: Self = Self(1 << 0);
    pub const PREFLIGHT: Self = Self(1 << 1);
    pub const ALL: Self = Self(Self::PROOF.0 | Self::PREFLIGHT.0);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn ordered_scopes(self) -> impl Iterator<Item = StartupCleanupScope> {
        [
            (Self::PROOF, StartupCleanupScope::Proof),
            (Self::PREFLIGHT, StartupCleanupScope::Preflight),
        ]
        .into_iter()
        .filter_map(move |(mask, scope)| self.contains(mask).then_some(scope))
    }
}

impl BitOr for StartupCleanupMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl From<StartupCleanupScope> for StartupCleanupMask {
    fn from(scope: StartupCleanupScope) -> Self {
        match scope {
            StartupCleanupScope::Proof => Self::PROOF,
            StartupCleanupScope::Preflight => Self::PREFLIGHT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupCleanupScopeReport {
    pub scope: StartupCleanupScope,
    pub matched: usize,
    pub removed: usize,
    pub failed: usize,
    pub duration: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupCleanupReport {
    pub scopes: Vec<StartupCleanupScopeReport>,
}

impl StartupCleanupReport {
    #[must_use]
    pub fn scope(&self, scope: StartupCleanupScope) -> Option<&StartupCleanupScopeReport> {
        self.scopes.iter().find(|report| report.scope == scope)
    }
}

pub trait RuntimeStoreScope: std::fmt::Debug + Send + Sync {
    fn environment(&self) -> &str;
    fn namespace(&self) -> &str;
    fn backend_name(&self) -> &'static str;
}

#[async_trait]
pub trait ProofObjectStore: RuntimeStoreScope {
    async fn put_if_absent(
        &self,
        key: &ProofArtifactKey,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult>;
    async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>>;
    async fn get_descriptor(
        &self,
        key: &ProofArtifactKey,
    ) -> Result<Option<ProofArtifactDescriptor>>;
    async fn get_prefix(
        &self,
        key: &ProofArtifactKey,
        max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>>;
    async fn invalidate_exact(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ExactInvalidationResult>;
    async fn is_invalidated(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<bool>;
    async fn delete_exact(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ProofArtifactDeleteResult>;
}

#[async_trait]
pub trait RuntimeStateStore: RuntimeStoreScope {
    async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>>;

    async fn store_runtime_state(
        &self,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<RuntimeStateWriteResult>;

    async fn cleanup_before_start(
        &self,
        scopes: StartupCleanupMask,
    ) -> Result<StartupCleanupReport> {
        if scopes.is_empty() {
            return Ok(StartupCleanupReport::default());
        }
        anyhow::bail!(
            "scoped startup cleanup is not supported by {} store",
            self.backend_name()
        )
    }

    /// Removes every persistent object in this store's configured namespace.
    ///
    /// Stores that do not implement a complete namespace reset fail closed when
    /// an operator requests one at startup.
    async fn reset_namespace(&self) -> Result<usize> {
        anyhow::bail!(
            "runtime namespace reset is not supported by {} store",
            self.backend_name()
        )
    }
}

pub trait RuntimeStore: ProofObjectStore + RuntimeStateStore {}

impl<T> RuntimeStore for T where T: ProofObjectStore + RuntimeStateStore {}

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
    preflight_manifests: HashMap<alloy_primitives::B256, MemoryPreflightManifest>,
    preflight_contents: HashMap<(alloy_primitives::B256, String), Vec<u8>>,
    preflight_invalidations: HashSet<(alloy_primitives::B256, Option<i64>, String)>,
    runtime_state: Option<RuntimeStateObject>,
}

#[derive(Clone, Debug)]
struct MemoryManifest {
    content_hash: String,
    generation: i64,
}

#[derive(Clone, Debug)]
struct MemoryPreflightManifest {
    key: CanonicalPreflightKeyV1,
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

impl RuntimeStoreScope for MemoryProofArtifactStore {
    fn environment(&self) -> &str {
        &self.environment
    }

    fn namespace(&self) -> &str {
        &self.namespace
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }
}

#[async_trait]
impl CanonicalPreflightStore for MemoryProofArtifactStore {
    async fn get_canonical_preflight(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<CanonicalPreflightObject>> {
        let key_digest = key.digest()?;
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let Some(manifest) = inner.preflight_manifests.get(&key_digest) else {
            return Ok(None);
        };
        anyhow::ensure!(
            manifest.key == *key,
            "canonical preflight manifest key does not match requested full key"
        );
        let bytes = inner
            .preflight_contents
            .get(&(key_digest, manifest.content_hash.clone()))
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("canonical preflight manifest references missing content")
            })?;
        anyhow::ensure!(
            content_hash(&bytes) == manifest.content_hash,
            "canonical preflight content hash mismatch"
        );
        Ok(Some(CanonicalPreflightObject {
            key_digest,
            content_hash: manifest.content_hash.clone(),
            generation: Some(manifest.generation),
            bytes,
        }))
    }

    async fn put_canonical_preflight_if_absent(
        &self,
        key: &CanonicalPreflightKeyV1,
        bytes: &[u8],
    ) -> Result<CanonicalPreflightPutResult> {
        let key_digest = key.digest()?;
        let hash = content_hash(bytes);
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        if let Some(existing) = inner.preflight_manifests.get(&key_digest).cloned() {
            anyhow::ensure!(
                existing.key == *key,
                "canonical preflight key digest collision"
            );
            if existing.content_hash == hash {
                inner
                    .preflight_contents
                    .entry((key_digest, hash.clone()))
                    .or_insert_with(|| bytes.to_vec());
                return Ok(CanonicalPreflightPutResult::AlreadyExists(
                    CanonicalPreflightObject {
                        key_digest,
                        content_hash: hash,
                        generation: Some(existing.generation),
                        bytes: bytes.to_vec(),
                    },
                ));
            }
            return Ok(CanonicalPreflightPutResult::Conflict(
                CanonicalPreflightDescriptor {
                    key_digest,
                    content_hash: existing.content_hash,
                    generation: Some(existing.generation),
                },
            ));
        }

        inner
            .preflight_contents
            .entry((key_digest, hash.clone()))
            .or_insert_with(|| bytes.to_vec());
        let generation = Self::next_generation(&mut inner);
        inner.preflight_manifests.insert(
            key_digest,
            MemoryPreflightManifest {
                key: key.clone(),
                content_hash: hash.clone(),
                generation,
            },
        );
        Ok(CanonicalPreflightPutResult::Created(
            CanonicalPreflightObject {
                key_digest,
                content_hash: hash,
                generation: Some(generation),
                bytes: bytes.to_vec(),
            },
        ))
    }

    async fn invalidate_canonical_preflight_exact(
        &self,
        key: &CanonicalPreflightKeyV1,
        descriptor: &CanonicalPreflightDescriptor,
    ) -> Result<CanonicalPreflightInvalidateResult> {
        let key_digest = key.digest()?;
        if descriptor.key_digest != key_digest {
            return Ok(CanonicalPreflightInvalidateResult::Stale);
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let invalidation = (
            key_digest,
            descriptor.generation,
            descriptor.content_hash.clone(),
        );
        let Some(current) = inner.preflight_manifests.get(&key_digest) else {
            return Ok(if inner.preflight_invalidations.contains(&invalidation) {
                CanonicalPreflightInvalidateResult::AlreadyInvalidated
            } else {
                CanonicalPreflightInvalidateResult::Missing
            });
        };
        if current.key != *key
            || Some(current.generation) != descriptor.generation
            || current.content_hash != descriptor.content_hash
        {
            return Ok(CanonicalPreflightInvalidateResult::Stale);
        }
        inner.preflight_invalidations.insert(invalidation);
        inner.preflight_manifests.remove(&key_digest);
        Ok(CanonicalPreflightInvalidateResult::Invalidated)
    }
}

#[async_trait]
impl ProofObjectStore for MemoryProofArtifactStore {
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
                    .cloned();
                let descriptor = ProofArtifactDescriptor {
                    proof_uri: self.content_uri(key, &existing.content_hash),
                    content_hash: existing.content_hash.clone(),
                    generation: Some(existing.generation),
                };
                ProofArtifactPutResult::Conflict(ProofArtifactConflict {
                    object: existing_bytes.map(|bytes| ProofArtifactObject {
                        proof_uri: descriptor.proof_uri.clone(),
                        content_hash: descriptor.content_hash.clone(),
                        generation: descriptor.generation,
                        bytes,
                    }),
                    descriptor,
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

    async fn invalidate_exact(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ExactInvalidationResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let invalidation = (
            key.clone(),
            descriptor.generation,
            descriptor.content_hash.clone(),
        );
        let Some(current) = inner.manifests.get(key) else {
            return Ok(if inner.invalidations.contains(&invalidation) {
                ExactInvalidationResult::AlreadyInvalidated
            } else {
                ExactInvalidationResult::Missing
            });
        };
        if Some(current.generation) != descriptor.generation
            || current.content_hash != descriptor.content_hash
            || self.content_uri(key, &current.content_hash) != descriptor.proof_uri
        {
            return Ok(ExactInvalidationResult::Stale);
        }
        inner.invalidations.insert(invalidation);
        inner.manifests.remove(key);
        Ok(ExactInvalidationResult::Invalidated(
            ProofArtifactDeleteResult::Removed,
        ))
    }

    async fn is_invalidated(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<bool> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        Ok(inner.invalidations.contains(&(
            key.clone(),
            descriptor.generation,
            descriptor.content_hash.clone(),
        )))
    }

    async fn delete_exact(
        &self,
        key: &ProofArtifactKey,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ProofArtifactDeleteResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let Some(current) = inner.manifests.get(key) else {
            return Ok(ProofArtifactDeleteResult::Missing);
        };
        anyhow::ensure!(
            Some(current.generation) == descriptor.generation
                && current.content_hash == descriptor.content_hash
                && self.content_uri(key, &current.content_hash) == descriptor.proof_uri,
            "proof artifact changed before conditional delete"
        );
        inner.manifests.remove(key);
        Ok(ProofArtifactDeleteResult::Removed)
    }
}

#[async_trait]
impl RuntimeStateStore for MemoryProofArtifactStore {
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

    async fn cleanup_before_start(
        &self,
        scopes: StartupCleanupMask,
    ) -> Result<StartupCleanupReport> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let mut report = StartupCleanupReport::default();
        if scopes.contains(StartupCleanupMask::PROOF) {
            let started_at = std::time::Instant::now();
            let matched = inner.manifests.len() + usize::from(inner.runtime_state.is_some());
            inner.runtime_state = None;
            inner.manifests.clear();
            report.scopes.push(StartupCleanupScopeReport {
                scope: StartupCleanupScope::Proof,
                matched,
                removed: matched,
                failed: 0,
                duration: started_at.elapsed(),
            });
        }
        if scopes.contains(StartupCleanupMask::PREFLIGHT) {
            let started_at = std::time::Instant::now();
            let matched = inner.preflight_manifests.len();
            inner.preflight_manifests.clear();
            report.scopes.push(StartupCleanupScopeReport {
                scope: StartupCleanupScope::Preflight,
                matched,
                removed: matched,
                failed: 0,
                duration: started_at.elapsed(),
            });
        }
        Ok(report)
    }

    async fn reset_namespace(&self) -> Result<usize> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        let cleared = inner.manifests.len()
            + inner.contents.len()
            + inner.invalidations.len()
            + inner.preflight_manifests.len()
            + inner.preflight_contents.len()
            + inner.preflight_invalidations.len()
            + usize::from(inner.runtime_state.is_some());
        let next_generation = inner.next_generation;
        *inner = MemoryStoreInner {
            next_generation,
            ..MemoryStoreInner::default()
        };
        Ok(cleared)
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
    use raiko2_pipeline::{
        PipelineKey,
        forks::shasta::preflight_cache::{
            CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightInvalidateResult,
            CanonicalPreflightKeyV1, CanonicalPreflightPutResult, CanonicalPreflightStore,
        },
    };
    use raiko2_primitives::{L2BlockRange, ShastaCheckpoint};

    fn key() -> ProofArtifactKey {
        let pipeline_key = PipelineKey::ShastaSp1;
        ProofArtifactKey {
            network_pair: "taiko_dev/ethereum".to_string(),
            pipeline_key,
            route: pipeline_key.route(),
            proof_ref: "proposal-1".to_string(),
        }
    }

    fn preflight_key() -> CanonicalPreflightKeyV1 {
        CanonicalPreflightKeyV1 {
            schema: CANONICAL_PREFLIGHT_SCHEMA_V1,
            l1_chain_id: 32_382,
            l2_chain_id: 167_001,
            proposal_id: 42,
            l2_block_range: L2BlockRange {
                start: 100,
                end: 101,
            },
            l1_inclusion_block_number: 77,
            last_anchor_block_number: 99,
            checkpoint: Some(ShastaCheckpoint {
                block_number: 101,
                block_hash: [0x11; 32].into(),
                state_root: [0x22; 32].into(),
            }),
            l1_inclusion_hash: [0x33; 32].into(),
            proposal_event_digest: [0x44; 32].into(),
            chain_rules_fingerprint: [0x55; 32].into(),
        }
    }

    #[tokio::test]
    async fn canonical_preflight_put_get_and_identical_reuse() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "preflight-a".into())?;
        let key = preflight_key();

        let CanonicalPreflightPutResult::Created(first) =
            CanonicalPreflightStore::put_canonical_preflight_if_absent(
                &store,
                &key,
                b"canonical-a",
            )
            .await?
        else {
            anyhow::bail!("first preflight put should create");
        };
        let CanonicalPreflightPutResult::AlreadyExists(second) =
            CanonicalPreflightStore::put_canonical_preflight_if_absent(
                &store,
                &key,
                b"canonical-a",
            )
            .await?
        else {
            anyhow::bail!("identical preflight put should reuse");
        };

        assert_eq!(first.descriptor(), second.descriptor());
        assert_eq!(
            CanonicalPreflightStore::get_canonical_preflight(&store, &key)
                .await?
                .expect("cached preflight")
                .bytes,
            b"canonical-a"
        );
        Ok(())
    }

    #[tokio::test]
    async fn canonical_preflight_conflict_is_first_write_wins() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "preflight-b".into())?;
        let key = preflight_key();
        let first = CanonicalPreflightStore::put_canonical_preflight_if_absent(
            &store,
            &key,
            b"canonical-a",
        )
        .await?
        .try_object()
        .expect("created object")
        .clone();

        let CanonicalPreflightPutResult::Conflict(conflict) =
            CanonicalPreflightStore::put_canonical_preflight_if_absent(
                &store,
                &key,
                b"canonical-b",
            )
            .await?
        else {
            anyhow::bail!("different preflight content should conflict");
        };

        assert_eq!(conflict, first.descriptor());
        assert_eq!(
            CanonicalPreflightStore::get_canonical_preflight(&store, &key)
                .await?
                .expect("cached preflight")
                .bytes,
            b"canonical-a"
        );
        Ok(())
    }

    #[tokio::test]
    async fn canonical_preflight_invalidation_is_generation_scoped() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "preflight-c".into())?;
        let key = preflight_key();
        let first = CanonicalPreflightStore::put_canonical_preflight_if_absent(
            &store,
            &key,
            b"canonical-a",
        )
        .await?
        .try_object()
        .expect("created object")
        .clone();

        assert_eq!(
            CanonicalPreflightStore::invalidate_canonical_preflight_exact(
                &store,
                &key,
                &first.descriptor(),
            )
            .await?,
            CanonicalPreflightInvalidateResult::Invalidated
        );
        assert!(
            CanonicalPreflightStore::get_canonical_preflight(&store, &key)
                .await?
                .is_none()
        );

        let second = CanonicalPreflightStore::put_canonical_preflight_if_absent(
            &store,
            &key,
            b"canonical-b",
        )
        .await?
        .try_object()
        .expect("replacement object")
        .clone();
        assert_ne!(first.generation, second.generation);
        assert_eq!(
            CanonicalPreflightStore::invalidate_canonical_preflight_exact(
                &store,
                &key,
                &first.descriptor(),
            )
            .await?,
            CanonicalPreflightInvalidateResult::Stale
        );
        assert_eq!(
            CanonicalPreflightStore::get_canonical_preflight(&store, &key)
                .await?
                .expect("replacement remains")
                .bytes,
            b"canonical-b"
        );
        Ok(())
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
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        store.delete_exact(&key, &first.descriptor()).await?;

        let second = store
            .put_if_absent(&key, b"proof-b")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        assert_ne!(first.proof_uri, second.proof_uri);
        assert!(store.delete_exact(&key, &first.descriptor()).await.is_err());
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
            .try_object()
            .expect("proof publication should materialize content")
            .clone();

        assert_eq!(
            store.delete_exact(&key, &object.descriptor()).await?,
            ProofArtifactDeleteResult::Removed
        );
        assert_eq!(
            store.delete_exact(&key, &object.descriptor()).await?,
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
            .try_object()
            .expect("proof publication should materialize content")
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
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        assert!(matches!(
            store.invalidate_exact(&key, &first.descriptor()).await?,
            ExactInvalidationResult::Invalidated(ProofArtifactDeleteResult::Removed)
        ));

        let second = store
            .put_if_absent(&key, b"deterministic-proof")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        assert_ne!(first.generation, second.generation);
        assert!(store.is_invalidated(&key, &first.descriptor()).await?);
        assert!(!store.is_invalidated(&key, &second.descriptor()).await?);
        Ok(())
    }

    #[tokio::test]
    async fn identical_put_repairs_missing_manifest_content() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-a".into())?;
        let key = key();
        let first = store
            .put_if_absent(&key, b"proof-a")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
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
    async fn different_put_conflicts_when_manifest_content_is_missing() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-a".into())?;
        let key = key();
        let first = store
            .put_if_absent(&key, b"proof-a")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        store
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?
            .contents
            .remove(&(key.clone(), first.content_hash.clone()));

        let conflict = store.put_if_absent(&key, b"proof-b").await?;

        let ProofArtifactPutResult::Conflict(conflict) = conflict else {
            anyhow::bail!("different proof did not conflict with dangling manifest");
        };
        assert_eq!(conflict.descriptor, first.descriptor());
        assert_eq!(conflict.object, None);
        Ok(())
    }

    #[tokio::test]
    async fn descriptor_remains_readable_when_manifest_content_is_missing() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-a".into())?;
        let key = key();
        let first = store
            .put_if_absent(&key, b"proof-a")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        store
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?
            .contents
            .remove(&(key.clone(), first.content_hash.clone()));

        assert_eq!(store.get_descriptor(&key).await?, Some(first.descriptor()));
        Ok(())
    }

    #[tokio::test]
    async fn proof_startup_cleanup_preserves_preflight_and_immutable_content() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "cleanup-proof".into())?;
        let proof_key = key();
        let proof = store
            .put_if_absent(&proof_key, b"proof")
            .await?
            .try_object()
            .expect("proof publication")
            .clone();
        let preflight_key = preflight_key();
        CanonicalPreflightStore::put_canonical_preflight_if_absent(
            &store,
            &preflight_key,
            b"preflight",
        )
        .await?;
        store.store_runtime_state(b"runtime", None).await?;

        let report = store
            .cleanup_before_start(StartupCleanupMask::PROOF)
            .await?;

        let proof_report = report
            .scope(StartupCleanupScope::Proof)
            .expect("proof cleanup report");
        assert_eq!(
            (
                proof_report.matched,
                proof_report.removed,
                proof_report.failed
            ),
            (2, 2, 0)
        );
        assert_eq!(store.load_runtime_state().await?, None);
        assert_eq!(store.get_descriptor(&proof_key).await?, None);
        assert!(
            CanonicalPreflightStore::get_canonical_preflight(&store, &preflight_key)
                .await?
                .is_some()
        );
        let inner = store
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        assert!(
            inner
                .contents
                .contains_key(&(proof_key, proof.content_hash))
        );
        assert_eq!(inner.preflight_contents.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn preflight_startup_cleanup_preserves_runtime_and_proof() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "cleanup-preflight".into())?;
        let proof_key = key();
        let proof = store
            .put_if_absent(&proof_key, b"proof")
            .await?
            .try_object()
            .expect("proof publication")
            .clone();
        let preflight_key = preflight_key();
        CanonicalPreflightStore::put_canonical_preflight_if_absent(
            &store,
            &preflight_key,
            b"preflight",
        )
        .await?;
        store.store_runtime_state(b"runtime", None).await?;

        let report = store
            .cleanup_before_start(StartupCleanupMask::PREFLIGHT)
            .await?;

        let preflight_report = report
            .scope(StartupCleanupScope::Preflight)
            .expect("preflight cleanup report");
        assert_eq!(
            (
                preflight_report.matched,
                preflight_report.removed,
                preflight_report.failed
            ),
            (1, 1, 0)
        );
        assert!(store.load_runtime_state().await?.is_some());
        assert_eq!(
            store.get_descriptor(&proof_key).await?,
            Some(proof.descriptor())
        );
        assert!(
            CanonicalPreflightStore::get_canonical_preflight(&store, &preflight_key)
                .await?
                .is_none()
        );
        let inner = store
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("memory store poisoned"))?;
        assert_eq!(inner.preflight_contents.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn all_startup_cleanup_runs_proof_before_preflight() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "cleanup-all".into())?;
        store.put_if_absent(&key(), b"proof").await?;
        CanonicalPreflightStore::put_canonical_preflight_if_absent(
            &store,
            &preflight_key(),
            b"preflight",
        )
        .await?;
        store.store_runtime_state(b"runtime", None).await?;

        let report = store.cleanup_before_start(StartupCleanupMask::ALL).await?;

        assert_eq!(
            report
                .scopes
                .iter()
                .map(|entry| entry.scope)
                .collect::<Vec<_>>(),
            vec![StartupCleanupScope::Proof, StartupCleanupScope::Preflight]
        );
        assert_eq!(report.scopes[0].matched, 2);
        assert_eq!(report.scopes[1].matched, 1);
        Ok(())
    }

    #[tokio::test]
    async fn namespace_reset_removes_runtime_state_artifacts_and_invalidations() -> Result<()> {
        let store = MemoryProofArtifactStore::new("devnet".into(), "raiko2-reset".into())?;
        let key = key();
        let proof = store
            .put_if_absent(&key, b"proof")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        assert!(matches!(
            store.invalidate_exact(&key, &proof.descriptor()).await?,
            ExactInvalidationResult::Invalidated(ProofArtifactDeleteResult::Removed)
        ));
        let RuntimeStateWriteResult::Stored {
            generation: Some(runtime_generation),
        } = store.store_runtime_state(b"runtime", None).await?
        else {
            anyhow::bail!("runtime state should receive a memory generation");
        };

        assert_eq!(store.reset_namespace().await?, 3);
        assert_eq!(store.load_runtime_state().await?, None);
        assert_eq!(store.get_descriptor(&key).await?, None);
        assert!(!store.is_invalidated(&key, &proof.descriptor()).await?);
        assert!(
            store
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("memory store poisoned"))?
                .contents
                .is_empty()
        );
        let RuntimeStateWriteResult::Stored {
            generation: Some(next_generation),
        } = store
            .store_runtime_state(b"runtime-after-reset", None)
            .await?
        else {
            anyhow::bail!("runtime state should receive a memory generation after reset");
        };
        assert!(next_generation > runtime_generation);
        Ok(())
    }
}
