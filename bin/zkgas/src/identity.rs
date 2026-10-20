//! Deterministic identities for generated descriptors and executable byte identity.
use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub use crate::contracts::ExecutableIdentity;

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Compact JSON, with recursive lexicographic object-key ordering and preserved array order.
pub fn canonical_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(values) => {
                let ordered: std::collections::BTreeMap<_, _> =
                    values.into_iter().map(|(k, v)| (k, sorted(v))).collect();
                serde_json::Value::Object(ordered.into_iter().collect())
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            value => value,
        }
    }
    Ok(serde_json::to_vec(&sorted(serde_json::to_value(value)?))?)
}

pub fn content_id(domain: &str, value: &impl Serialize) -> Result<String> {
    // Length-prefix the domain so even embedded NULs cannot alias another descriptor domain.
    let mut hash = Sha256::new();
    hash.update((domain.len() as u64).to_be_bytes());
    hash.update(domain.as_bytes());
    hash.update(canonical_json(value)?);
    Ok(hex::encode(hash.finalize()))
}

pub fn executable_identity() -> Result<ExecutableIdentity> {
    let path = std::env::current_exe()?;
    Ok(ExecutableIdentity {
        sha256: sha256_hex(&std::fs::read(path)?),
        package_version: env!("CARGO_PKG_VERSION").into(),
        source_revision: option_env!("ZKGAS_SOURCE_REVISION").map(str::to_owned),
        dirty: option_env!("ZKGAS_SOURCE_DIRTY").and_then(|text| text.parse().ok()),
    })
}
