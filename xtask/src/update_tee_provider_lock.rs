use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::tee_provider_lock;

#[derive(Args, Debug)]
pub(crate) struct UpdateTeeProviderLockArgs {
    pub(crate) provider: String,

    #[arg(long)]
    pub(crate) commit: String,
}

pub(crate) fn run(root: &Path, args: UpdateTeeProviderLockArgs) -> Result<()> {
    let path = provider_lock_path(root);
    update_provider_lock_file(&path, &args.provider, &args.commit)
}

fn provider_lock_path(root: &Path) -> PathBuf {
    root.join("release").join("providers.toml")
}

fn update_provider_lock_file(path: &Path, provider: &str, commit: &str) -> Result<()> {
    if commit.trim().is_empty() {
        bail!("commit must not be empty");
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read provider lock {}", path.display()))?;
    let _lock = tee_provider_lock::parse(&raw)?;
    let updated = update_provider_commit(&raw, provider, commit)?;
    fs::write(path, updated)
        .with_context(|| format!("failed to write provider lock {}", path.display()))?;
    Ok(())
}

fn update_provider_commit(raw: &str, provider: &str, commit: &str) -> Result<String> {
    let target_header = format!("[providers.{provider}]");
    let mut in_target = false;
    let mut found_header = false;
    let mut replaced = false;
    let mut lines = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_target = trimmed == target_header;
            found_header |= in_target;
        }

        if in_target && trimmed.starts_with("commit") {
            let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
            lines.push(format!("{indent}commit = \"{commit}\""));
            replaced = true;
            continue;
        }

        lines.push(line.to_string());
    }

    if !found_header {
        bail!("provider {provider} not found in provider lock");
    }
    if !replaced {
        bail!("provider {provider} is missing a commit field in provider lock");
    }

    Ok(format!("{}\n", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{update_provider_commit, update_provider_lock_file};

    const SAMPLE_LOCK: &str = r#"
[providers.gaiko2]
repo = "https://github.com/taikoxyz/gaiko2.git"
commit = "oldsha"
provider = "gaiko2-sgxgeth"
lane = "sgxgeth"
image_name = "gaiko2-sgxgeth"
repository = "us-docker.pkg.dev/evmchain/images/gaiko2-sgxgeth"
dockerfile = "docker/Dockerfile.tee"
context = "."
attestation_path = "/opt/gaiko2/etc/attestation.json"

[providers.other]
repo = "https://example.com/other.git"
commit = "stayput"
provider = "other-provider"
lane = "tdx"
image_name = "other-image"
repository = "us-docker.pkg.dev/evmchain/images/other"
dockerfile = "docker/Dockerfile"
context = "."
attestation_path = "/opt/other/etc/attestation.json"
"#;

    #[test]
    fn update_tee_provider_lock_updates_only_targeted_commit() {
        let updated =
            update_provider_commit(SAMPLE_LOCK, "gaiko2", "newsha").expect("update should pass");

        assert!(updated.contains("commit = \"newsha\""));
        assert!(updated.contains("[providers.other]"));
        assert!(updated.contains("commit = \"stayput\""));
        assert_eq!(updated.matches("commit = ").count(), 2);
    }

    #[test]
    fn update_tee_provider_lock_file_rewrites_temp_lock() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = env::temp_dir().join(format!("raiko2-tee-provider-lock-{unique}.toml"));
        fs::write(&path, SAMPLE_LOCK).expect("write sample lock");

        update_provider_lock_file(&path, "gaiko2", "abcdef123").expect("rewrite should pass");
        let contents = fs::read_to_string(&path).expect("read updated lock");

        assert!(contents.contains("commit = \"abcdef123\""));
        assert!(contents.contains("commit = \"stayput\""));

        fs::remove_file(path).expect("cleanup temp lock");
    }
}
