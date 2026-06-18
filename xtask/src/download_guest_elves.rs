use std::path::PathBuf;
use std::process::Command;

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};

use crate::util;

const DEFAULT_GUEST_RELEASE_REPO: &str = "taikoxyz/raiko2";
const DEFAULT_GUEST_ELF_DIR: &str = "crates/guests/elf";

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestElfBackend {
    Risc0,
    Sp1,
    All,
}

#[derive(Args, Debug)]
pub(crate) struct DownloadGuestElvesArgs {
    /// GitHub Release tag that owns the guest ELF assets.
    #[arg(long)]
    pub(crate) tag: String,

    /// GitHub repository containing the guest ELF release assets.
    #[arg(long, default_value = DEFAULT_GUEST_RELEASE_REPO)]
    pub(crate) repo: String,

    /// Guest ELF output directory.
    #[arg(long, default_value = DEFAULT_GUEST_ELF_DIR)]
    pub(crate) dir: PathBuf,

    /// Guest backend asset set to download.
    #[arg(long, value_enum, default_value_t = GuestElfBackend::All)]
    pub(crate) backend: GuestElfBackend,
}

pub(crate) fn run(root: &std::path::Path, args: DownloadGuestElvesArgs) -> Result<()> {
    ensure_non_empty("tag", &args.tag)?;
    ensure_non_empty("repo", &args.repo)?;
    ensure_gh_available()?;
    let dir = if args.dir.is_absolute() {
        args.dir
    } else {
        root.join(args.dir)
    };
    std::fs::create_dir_all(&dir)?;

    for pattern in asset_patterns(args.backend) {
        let mut cmd = Command::new("gh");
        cmd.arg("release")
            .arg("download")
            .arg(&args.tag)
            .arg("--repo")
            .arg(&args.repo)
            .arg("--dir")
            .arg(&dir)
            .arg("--clobber")
            .arg("--pattern")
            .arg(pattern);
        util::run(cmd)?;
    }

    Ok(())
}

fn ensure_gh_available() -> Result<()> {
    match Command::new("gh").arg("--version").status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!(
            "GitHub CLI `gh` is required to download guest ELFs; install it and run `gh auth login` (gh --version exited with {status})"
        ),
        Err(err) => bail!(
            "GitHub CLI `gh` is required to download guest ELFs; install it and run `gh auth login`: {err}"
        ),
    }
}

fn asset_patterns(backend: GuestElfBackend) -> &'static [&'static str] {
    match backend {
        GuestElfBackend::Risc0 => &["risc0_shasta_*.elf"],
        GuestElfBackend::Sp1 => &["sp1_shasta_*.elf"],
        GuestElfBackend::All => &["risc0_shasta_*.elf", "sp1_shasta_*.elf"],
    }
}

fn ensure_non_empty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{GuestElfBackend, asset_patterns};

    #[test]
    fn asset_patterns_are_backend_scoped() {
        assert_eq!(
            asset_patterns(GuestElfBackend::Risc0),
            ["risc0_shasta_*.elf"]
        );
        assert_eq!(asset_patterns(GuestElfBackend::Sp1), ["sp1_shasta_*.elf"]);
        assert_eq!(
            asset_patterns(GuestElfBackend::All),
            ["risc0_shasta_*.elf", "sp1_shasta_*.elf"]
        );
    }
}
