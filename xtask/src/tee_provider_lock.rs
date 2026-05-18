use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct TeeProviderLock {
    pub(crate) providers: BTreeMap<String, TeeProviderEntry>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct TeeProviderEntry {
    pub(crate) repo: String,
    pub(crate) commit: String,
    pub(crate) provider: String,
    pub(crate) lane: String,
    pub(crate) image_name: String,
    pub(crate) repository: String,
    pub(crate) dockerfile: String,
    pub(crate) context: String,
    pub(crate) attestation_path: String,
}

pub(crate) fn load(path: &Path) -> Result<TeeProviderLock> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read provider lock {}", path.display()))?;
    parse(&raw).with_context(|| format!("failed to parse provider lock {}", path.display()))
}

pub(crate) fn parse(raw: &str) -> Result<TeeProviderLock> {
    toml::from_str(raw).context("parse provider lock toml")
}

#[cfg(test)]
mod tests {
    use super::{TeeProviderEntry, parse};

    #[test]
    fn tee_provider_lock_parses_known_provider() {
        let lock = parse(
            r#"
[providers.gaiko2]
repo = "https://github.com/taikoxyz/gaiko2.git"
commit = "abcdef1234567890"
provider = "gaiko2-sgxgeth"
lane = "sgxgeth"
image_name = "gaiko2-sgxgeth"
repository = "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth"
dockerfile = "docker/Dockerfile.tee"
context = "."
attestation_path = "/opt/gaiko2/etc/attestation.json"
"#,
        )
        .expect("lock should parse");

        let entry = lock.providers.get("gaiko2").expect("gaiko2 entry");
        assert_eq!(
            entry,
            &TeeProviderEntry {
                repo: "https://github.com/taikoxyz/gaiko2.git".to_string(),
                commit: "abcdef1234567890".to_string(),
                provider: "gaiko2-sgxgeth".to_string(),
                lane: "sgxgeth".to_string(),
                image_name: "gaiko2-sgxgeth".to_string(),
                repository: "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth".to_string(),
                dockerfile: "docker/Dockerfile.tee".to_string(),
                context: ".".to_string(),
                attestation_path: "/opt/gaiko2/etc/attestation.json".to_string(),
            }
        );
    }

    #[test]
    fn tee_provider_lock_rejects_missing_required_field() {
        parse(
            r#"
[providers.gaiko2]
repo = "https://github.com/taikoxyz/gaiko2.git"
provider = "gaiko2-sgxgeth"
lane = "sgxgeth"
image_name = "gaiko2-sgxgeth"
repository = "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth"
dockerfile = "docker/Dockerfile.tee"
context = "."
attestation_path = "/opt/gaiko2/etc/attestation.json"
"#,
        )
        .expect_err("missing commit must fail");
    }
}
