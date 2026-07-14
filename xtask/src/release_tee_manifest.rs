use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct TeeAttestationManifest {
    pub(crate) release: String,
    pub(crate) generated_at: String,
    pub(crate) providers: Vec<TeeProviderManifestEntry>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct TeeProviderManifestEntry {
    pub(crate) lane: String,
    pub(crate) provider: String,
    pub(crate) source: TeeProviderSource,
    pub(crate) image: TeeProviderImage,
    pub(crate) attestation: TeeProviderAttestation,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct TeeProviderSource {
    pub(crate) repo: String,
    pub(crate) commit: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct TeeProviderImage {
    pub(crate) repository: String,
    pub(crate) tag: String,
    pub(crate) digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sgx_edmm: Option<bool>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct TeeProviderAttestation {
    pub(crate) mr_enclave: String,
    pub(crate) mr_signer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) isv_prod_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) isv_svn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) debug_enclave: Option<bool>,
}

pub(crate) fn release_manifest_path(root: &Path, tag: &str) -> PathBuf {
    root.join("target")
        .join("releases")
        .join(tag)
        .join(format!("tee-attestation-manifest-{tag}.json"))
}

pub(crate) fn write_manifest(path: &Path, manifest: &TeeAttestationManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create manifest dir {}", parent.display()))?;
    }

    let mut bytes = serde_json::to_vec_pretty(manifest).context("serialize tee manifest json")?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{
        TeeAttestationManifest, TeeProviderAttestation, TeeProviderImage, TeeProviderManifestEntry,
        TeeProviderSource, release_manifest_path, write_manifest,
    };

    #[test]
    fn release_tee_manifest_path_uses_tagged_release_dir() {
        let root = std::path::Path::new("/tmp/raiko2");
        let path = release_manifest_path(root, "v1.2.3");
        assert_eq!(
            path,
            std::path::Path::new(
                "/tmp/raiko2/target/releases/v1.2.3/tee-attestation-manifest-v1.2.3.json"
            )
        );
    }

    #[test]
    fn release_tee_manifest_writer_emits_stable_json() {
        let manifest = TeeAttestationManifest {
            release: "v1.2.3".to_string(),
            generated_at: "2026-05-14T12:34:56Z".to_string(),
            providers: vec![TeeProviderManifestEntry {
                lane: "sgx".to_string(),
                provider: "raiko2-sgx".to_string(),
                source: TeeProviderSource {
                    repo: "local".to_string(),
                    commit: "abcdef123".to_string(),
                },
                image: TeeProviderImage {
                    repository: "us-docker.pkg.dev/evmchain/images/raiko2-sgx".to_string(),
                    tag: "v1.2.3".to_string(),
                    digest: "us-docker.pkg.dev/evmchain/images/raiko2-sgx@sha256:deadbeef"
                        .to_string(),
                    sgx_edmm: Some(false),
                },
                attestation: TeeProviderAttestation {
                    mr_enclave: "enclave".to_string(),
                    mr_signer: "signer".to_string(),
                    isv_prod_id: Some(0),
                    isv_svn: Some(0),
                    debug_enclave: Some(false),
                },
            }],
        };

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("raiko2-tee-manifest-{unique}.json"));

        write_manifest(&path, &manifest).expect("manifest write");
        let contents = std::fs::read_to_string(&path).expect("manifest contents");

        assert!(contents.contains("\"release\": \"v1.2.3\""));
        assert!(contents.contains("\"providers\""));
        assert!(contents.contains("\"lane\": \"sgx\""));
        assert!(contents.contains("\"sgx_edmm\": false"));
        assert!(contents.contains("\"attestation\""));
        assert!(contents.ends_with('\n'));

        std::fs::remove_file(path).expect("cleanup manifest");
    }

    #[test]
    fn release_tee_manifest_omits_unspecified_sgx_edmm_capability() {
        let image = TeeProviderImage {
            repository: "example.invalid/provider/image".to_string(),
            tag: "v1.2.3".to_string(),
            digest: "example.invalid/provider/image@sha256:deadbeef".to_string(),
            sgx_edmm: None,
        };

        let contents = serde_json::to_string(&image).expect("image serialization");

        assert!(!contents.contains("sgx_edmm"));
    }
}
