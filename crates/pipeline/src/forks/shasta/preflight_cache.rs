use alloy_primitives::B256;
use anyhow::Result;
use async_trait::async_trait;
pub use raiko2_primitives_shasta::{CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightKeyV1};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPreflightObject {
    pub key_digest: B256,
    pub content_hash: String,
    pub generation: Option<i64>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPreflightDescriptor {
    pub key_digest: B256,
    pub content_hash: String,
    pub generation: Option<i64>,
}

impl CanonicalPreflightObject {
    #[must_use]
    pub fn descriptor(&self) -> CanonicalPreflightDescriptor {
        CanonicalPreflightDescriptor {
            key_digest: self.key_digest,
            content_hash: self.content_hash.clone(),
            generation: self.generation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalPreflightPutResult {
    Created(CanonicalPreflightObject),
    AlreadyExists(CanonicalPreflightObject),
    Conflict(CanonicalPreflightDescriptor),
}

impl CanonicalPreflightPutResult {
    #[must_use]
    pub const fn try_object(&self) -> Option<&CanonicalPreflightObject> {
        match self {
            Self::Created(object) | Self::AlreadyExists(object) => Some(object),
            Self::Conflict(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalPreflightInvalidateResult {
    Invalidated,
    AlreadyInvalidated,
    Stale,
    Missing,
}

#[async_trait]
pub trait CanonicalPreflightStore: std::fmt::Debug + Send + Sync {
    async fn get_canonical_preflight(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<CanonicalPreflightObject>>;

    async fn put_canonical_preflight_if_absent(
        &self,
        key: &CanonicalPreflightKeyV1,
        bytes: &[u8],
    ) -> Result<CanonicalPreflightPutResult>;

    async fn invalidate_canonical_preflight_exact(
        &self,
        key: &CanonicalPreflightKeyV1,
        descriptor: &CanonicalPreflightDescriptor,
    ) -> Result<CanonicalPreflightInvalidateResult>;
}
