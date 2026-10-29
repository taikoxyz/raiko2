//! Receipt only newly created SDK resources backed by this process's open descriptors.
use std::{collections::BTreeSet, fs::File, path::Path};

use anyhow::{Result, ensure};

pub(crate) fn baseline() -> Result<BTreeSet<String>> {
    Ok(std::fs::read_dir("/dev/shm")?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?)
}

fn sdk_id(name: &str) -> Option<&str> {
    let id = name
        .strip_suffix("_m")
        .or_else(|| name.strip_suffix("_t"))?;
    let token = id.strip_prefix("sp1_")?;
    (token.len() == 24
        && token.ends_with("==")
        && token.as_bytes()[..22]
            .iter()
            .copied()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
    .then_some(id)
}

#[cfg(target_os = "linux")]
pub(crate) fn publish(output: File, before: &BTreeSet<String>) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let mut ids = BTreeSet::new();
    let mut resources = Vec::new();
    for entry in std::fs::read_dir("/proc/self/fd")? {
        let entry = entry?;
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if target.parent() != Some(Path::new("/dev/shm")) {
            continue;
        }
        let Some(name) = target.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if before.contains(name) {
            continue;
        }
        let Some(id) = sdk_id(name) else { continue };
        // Match the pathname to the actual open descriptor, not merely its printed name.
        let descriptor = std::fs::metadata(entry.path())?;
        let named = std::fs::symlink_metadata(&target)?;
        ensure!(
            named.is_file() && named.dev() == descriptor.dev() && named.ino() == descriptor.ino(),
            "SDK shared-memory pathname/descriptor mismatch"
        );
        ids.insert((id.to_owned(), descriptor.uid()));
    }
    for (id, owner) in ids {
        for name in [
            format!("{id}_m"),
            format!("{id}_t"),
            format!("sem.{id}_t_filled"),
            format!("sem.{id}_t_empty"),
        ] {
            ensure!(
                !before.contains(&name),
                "SDK resource existed before this runner"
            );
            let path = Path::new("/dev/shm").join(&name);
            let metadata = std::fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_file() && metadata.uid() == owner,
                "unexpected SDK resource ownership"
            );
            resources.push(serde_json::json!({"name": name, "device": metadata.dev(),
                "inode": metadata.ino(), "uid": metadata.uid(), "size": metadata.len(),
                "allocated_bytes": metadata.blocks() * 512}));
        }
    }
    ensure!(
        resources.len() == 4,
        "expected one newly constructed native SP1 runner's four resources"
    );
    crate::io::write_json(
        output,
        &serde_json::json!({"schema_version": 1,
        "stage": "sp1_owned_shared_memory", "producer_pid": std::process::id(),
        "resources": resources}),
    )
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn publish(_output: File, _before: &BTreeSet<String>) -> Result<()> {
    anyhow::bail!("shared-memory receipts require Linux /proc and /dev/shm")
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_exact_sdk_uuid_names_are_candidates() {
        assert_eq!(
            super::sdk_id("sp1_AAAAAAAAAAAAAAAAAAAAAA==_m"),
            Some("sp1_AAAAAAAAAAAAAAAAAAAAAA==")
        );
        for name in [
            "sp1_unknown_m",
            "other_AAAAAAAAAAAAAAAAAAAAAA==_m",
            "../sp1_AAAAAAAAAAAAAAAAAAAAAA==_m",
        ] {
            assert!(super::sdk_id(name).is_none());
        }
    }
}
