use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::{B256, hex};
use anyhow::{Context, Result};
use clap::Args;
use raiko2_guests::{
    DEFAULT_GUEST_ELF_DIR, Risc0ShastaGuestElves, Sp1ShastaGuestElves,
    load_risc0_shasta_guest_elves_from_dir, load_sp1_shasta_guest_elves_from_dir,
};
use risc0_zkvm::compute_image_id;
use serde::Serialize;
use sp1_sdk::HashableKey;

use crate::{util, verified_sp1_vk};

#[derive(Args)]
pub struct GuestDigestsArgs {
    /// Guest ELF directory to inspect. Defaults to crates/guests/elf under the repository root.
    #[arg(long)]
    pub guest_elf_dir: Option<PathBuf>,

    /// Output path for the JSON summary.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProofSystem {
    Risc0,
    Sp1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Proposal,
    Aggregation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum DigestSource {
    ImageId,
    VkBn254,
    VkHashBytes,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct GuestDigestEntry {
    proof_system: ProofSystem,
    object_name: String,
    stage: Stage,
    digest_source: DigestSource,
    digest: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GuestDigestSummary {
    created_at_unix: u64,
    guest_elf_dir: String,
    digests: Vec<GuestDigestEntry>,
}

pub fn run(root: &Path, args: GuestDigestsArgs) -> Result<()> {
    let summary = collect_guest_digests_with_dir(root, args.guest_elf_dir.as_deref())?;
    let output_path = resolve_output_path(root, args.output.as_deref())?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    write_json(&output_path, &summary)?;
    println!("Wrote guest digest summary to {}", output_path.display());
    Ok(())
}

pub fn collect_guest_digests(root: &Path) -> Result<GuestDigestSummary> {
    collect_guest_digests_with_dir(root, None)
}

pub fn collect_guest_digests_with_dir(
    root: &Path,
    guest_elf_dir: Option<&Path>,
) -> Result<GuestDigestSummary> {
    let elf_dir = guest_elf_dir
        .map(|path| resolve_input_path(root, path))
        .unwrap_or_else(|| root.join(DEFAULT_GUEST_ELF_DIR));
    let risc0_elves = load_risc0_shasta_guest_elves_from_dir(&elf_dir)
        .with_context(|| format!("failed to load RISC0 guest ELFs from {}", elf_dir.display()))?;
    let sp1_elves = load_sp1_shasta_guest_elves_from_dir(&elf_dir)
        .with_context(|| format!("failed to load SP1 guest ELFs from {}", elf_dir.display()))?;

    let mut digests = Vec::new();
    digests.extend(risc0_digest_entries(&risc0_elves)?);
    digests.extend(sp1_digest_entries(&sp1_elves)?);
    digests.sort();

    Ok(GuestDigestSummary {
        created_at_unix: unix_timestamp(),
        guest_elf_dir: guest_elf_dir
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| DEFAULT_GUEST_ELF_DIR.to_string()),
        digests,
    })
}

fn resolve_input_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn resolve_output_path(root: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        });
    }

    Ok(util::target_root(root)
        .join("guest-digests")
        .join("summary.json"))
}

fn risc0_digest_entries(elves: &Risc0ShastaGuestElves) -> Result<Vec<GuestDigestEntry>> {
    Ok(vec![
        risc0_digest_entry(
            "risc0_shasta_proposal",
            Stage::Proposal,
            elves.proposal.as_ref(),
        )?,
        risc0_digest_entry(
            "risc0_shasta_aggregation",
            Stage::Aggregation,
            elves.aggregation.as_ref(),
        )?,
    ])
}

fn sp1_digest_entries(elves: &Sp1ShastaGuestElves) -> Result<Vec<GuestDigestEntry>> {
    let proposal_vk = verified_sp1_vk(
        Arc::clone(&elves.proposal),
        Some(elves.proposal_vk.as_ref()),
        "sp1_shasta_proposal",
    )?;
    let aggregation_vk = verified_sp1_vk(
        Arc::clone(&elves.aggregation),
        Some(elves.aggregation_vk.as_ref()),
        "sp1_shasta_aggregation",
    )?;

    Ok(vec![
        sp1_digest_entry(
            "sp1_shasta_proposal",
            Stage::Proposal,
            DigestSource::VkBn254,
            proposal_vk.bytes32(),
        ),
        sp1_digest_entry(
            "sp1_shasta_proposal",
            Stage::Proposal,
            DigestSource::VkHashBytes,
            hex::encode_prefixed(proposal_vk.hash_bytes()),
        ),
        sp1_digest_entry(
            "sp1_shasta_aggregation",
            Stage::Aggregation,
            DigestSource::VkBn254,
            aggregation_vk.bytes32(),
        ),
        sp1_digest_entry(
            "sp1_shasta_aggregation",
            Stage::Aggregation,
            DigestSource::VkHashBytes,
            hex::encode_prefixed(aggregation_vk.hash_bytes()),
        ),
    ])
}

fn risc0_digest_entry(object_name: &str, stage: Stage, elf: &[u8]) -> Result<GuestDigestEntry> {
    let image_id = compute_image_id(elf)
        .with_context(|| format!("failed to compute RISC0 image id for {object_name}"))?;
    Ok(GuestDigestEntry {
        proof_system: ProofSystem::Risc0,
        object_name: object_name.to_string(),
        stage,
        digest_source: DigestSource::ImageId,
        digest: b256_hex(B256::from_slice(image_id.as_bytes())),
    })
}

fn sp1_digest_entry(
    object_name: &str,
    stage: Stage,
    digest_source: DigestSource,
    digest: String,
) -> GuestDigestEntry {
    GuestDigestEntry {
        proof_system: ProofSystem::Sp1,
        object_name: object_name.to_string(),
        stage,
        digest_source,
        digest,
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).context("failed to serialize json")?;
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn b256_hex(value: B256) -> String {
    format!("{value:#x}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use raiko2_guests::{
        DEFAULT_GUEST_ELF_DIR, RISC0_SHASTA_AGGREGATION_ELF, RISC0_SHASTA_PROPOSAL_ELF,
        SP1_SHASTA_AGGREGATION_ELF, SP1_SHASTA_AGGREGATION_VK_BIN, SP1_SHASTA_PROPOSAL_ELF,
        SP1_SHASTA_PROPOSAL_VK_BIN,
    };

    #[test]
    fn guest_digests_cover_expected_objects_and_sources() {
        let root = crate::repo_root();
        let summary = super::collect_guest_digests(&root).expect("collect guest digests");

        let mut counts = BTreeMap::<(String, String), usize>::new();
        for entry in &summary.digests {
            *counts
                .entry((
                    entry.object_name.clone(),
                    format!("{:?}", entry.digest_source),
                ))
                .or_default() += 1;
        }

        assert_eq!(
            counts.get(&("risc0_shasta_proposal".to_string(), "ImageId".to_string())),
            Some(&1)
        );
        assert_eq!(
            counts.get(&(
                "risc0_shasta_aggregation".to_string(),
                "ImageId".to_string()
            )),
            Some(&1)
        );
        assert_eq!(
            counts.get(&("sp1_shasta_proposal".to_string(), "VkBn254".to_string())),
            Some(&1)
        );
        assert_eq!(
            counts.get(&("sp1_shasta_proposal".to_string(), "VkHashBytes".to_string())),
            Some(&1)
        );
        assert_eq!(
            counts.get(&("sp1_shasta_aggregation".to_string(), "VkBn254".to_string())),
            Some(&1)
        );
        assert_eq!(
            counts.get(&(
                "sp1_shasta_aggregation".to_string(),
                "VkHashBytes".to_string()
            )),
            Some(&1)
        );
    }

    #[test]
    fn relative_guest_elf_dir_resolves_from_repo_root() {
        let root = PathBuf::from("/repo/root");
        assert_eq!(
            super::resolve_input_path(&root, Path::new("crates/guests/elf")),
            root.join("crates/guests/elf")
        );
        assert_eq!(
            super::resolve_input_path(&root, Path::new("/tmp/guest-elf")),
            PathBuf::from("/tmp/guest-elf")
        );
    }

    #[test]
    fn guest_digests_run_writes_json_summary() {
        let root = crate::repo_root();
        let output = crate::util::target_root(&root)
            .join("guest-digests-test")
            .join("summary.json");
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        if output.exists() {
            fs::remove_file(&output).expect("remove old output");
        }

        super::run(
            &root,
            super::GuestDigestsArgs {
                guest_elf_dir: None,
                output: Some(output.clone()),
            },
        )
        .expect("run guest digests");

        let contents = fs::read_to_string(&output).expect("read output");
        assert!(contents.contains("\"guest_elf_dir\""));
        assert!(contents.contains(DEFAULT_GUEST_ELF_DIR));
        assert!(contents.contains("risc0_shasta_proposal"));
        assert!(contents.contains("sp1_shasta_aggregation"));
    }

    #[test]
    fn collect_guest_digests_rejects_swapped_sp1_vk_artifact() {
        let root = repo_root();
        let source = root.join(DEFAULT_GUEST_ELF_DIR);
        let temp = temp_test_dir();

        for file_name in [
            RISC0_SHASTA_PROPOSAL_ELF,
            RISC0_SHASTA_AGGREGATION_ELF,
            SP1_SHASTA_PROPOSAL_ELF,
            SP1_SHASTA_AGGREGATION_ELF,
            SP1_SHASTA_AGGREGATION_VK_BIN,
        ] {
            fs::copy(source.join(file_name), temp.join(file_name))
                .unwrap_or_else(|err| panic!("copy {file_name}: {err}"));
        }
        fs::copy(
            source.join(SP1_SHASTA_AGGREGATION_VK_BIN),
            temp.join(SP1_SHASTA_PROPOSAL_VK_BIN),
        )
        .expect("write swapped proposal VK artifact");

        let err = match super::collect_guest_digests_with_dir(&root, Some(&temp)) {
            Ok(_) => panic!("guest-digests should reject swapped SP1 VK artifacts"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("SP1 VK artifact mismatch for sp1_shasta_proposal")
        );

        let _ = fs::remove_dir_all(temp);
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask-build-guest lives under xtask/")
            .to_path_buf()
    }

    fn temp_test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("raiko2-guest-digests-test-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
