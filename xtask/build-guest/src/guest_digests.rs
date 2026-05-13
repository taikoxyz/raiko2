use std::fs;
use std::path::{Path, PathBuf};
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
use sp1_sdk::{
    HashableKey, ProvingKey as _,
    blocking::{Prover as _, ProverClient},
};

use crate::util;

#[derive(Args)]
pub struct GuestDigestsArgs {
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
    let summary = collect_guest_digests(root)?;
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
    let elf_dir = root.join(DEFAULT_GUEST_ELF_DIR);
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
        guest_elf_dir: DEFAULT_GUEST_ELF_DIR.to_string(),
        digests,
    })
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
        risc0_digest_entry(
            "risc0_shasta_boundless_aggregation",
            Stage::Aggregation,
            elves.boundless_aggregation.as_ref(),
        )?,
    ])
}

fn sp1_digest_entries(elves: &Sp1ShastaGuestElves) -> Result<Vec<GuestDigestEntry>> {
    let client = ProverClient::builder().cpu().build();
    let proposal_pk = client
        .setup(elves.proposal.as_ref().into())
        .context("failed to setup SP1 proposal ELF")?;
    let aggregation_pk = client
        .setup(elves.aggregation.as_ref().into())
        .context("failed to setup SP1 aggregation ELF")?;
    let proposal_vk = proposal_pk.verifying_key();
    let aggregation_vk = aggregation_pk.verifying_key();

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
    use std::fs;

    use raiko2_guests::DEFAULT_GUEST_ELF_DIR;

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
            counts.get(&(
                "risc0_shasta_boundless_aggregation".to_string(),
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
}
