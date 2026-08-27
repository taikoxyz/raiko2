#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Guest program ELF assets for Raiko2.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const DEFAULT_GUEST_ELF_DIR: &str = "crates/guests/elf";
pub const GUEST_ELF_DIR_ENV: &str = "RAIKO2_GUEST_ELF_DIR";

pub const RISC0_SHASTA_PROPOSAL_ELF: &str = "risc0_shasta_proposal.elf";
pub const RISC0_SHASTA_AGGREGATION_ELF: &str = "risc0_shasta_aggregation.elf";
pub const SP1_SHASTA_PROPOSAL_ELF: &str = "sp1_shasta_proposal.elf";
pub const SP1_SHASTA_AGGREGATION_ELF: &str = "sp1_shasta_aggregation.elf";
pub const SP1_SHASTA_PROPOSAL_VK_BIN: &str = "sp1_shasta_proposal.vk.bin";
pub const SP1_SHASTA_AGGREGATION_VK_BIN: &str = "sp1_shasta_aggregation.vk.bin";

#[derive(Clone, Debug)]
pub struct ShastaGuestElves {
    pub risc0: Risc0GuestElves,
    pub sp1: Sp1GuestElves,
}

#[derive(Clone, Debug)]
pub struct Risc0GuestElves {
    pub proposal: Arc<[u8]>,
    pub aggregation: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub struct Sp1GuestElves {
    pub proposal: Arc<[u8]>,
    pub aggregation: Arc<[u8]>,
    pub proposal_vk: Arc<[u8]>,
    pub aggregation_vk: Arc<[u8]>,
}

/// Load all Shasta guest ELFs from the default fixed repository/runtime path.
///
/// # Errors
///
/// Returns an error if the guest ELF directory cannot be resolved or any required ELF file cannot
/// be read.
pub fn load_guest_elves() -> io::Result<ShastaGuestElves> {
    let dir = default_guest_elf_dir()?;
    load_guest_elves_from_dir(dir)
}

/// Load all RISC0 Shasta guest ELFs from the default fixed repository/runtime path.
///
/// # Errors
///
/// Returns an error if the guest ELF directory cannot be resolved or any required RISC0 ELF file
/// cannot be read.
pub fn load_risc0_guest_elves() -> io::Result<Risc0GuestElves> {
    let dir = default_guest_elf_dir()?;
    load_risc0_guest_elves_from_dir(dir)
}

/// Load all SP1 Shasta guest ELFs from the default fixed repository/runtime path.
///
/// # Errors
///
/// Returns an error if the guest ELF directory cannot be resolved or any required SP1 ELF file
/// cannot be read.
pub fn load_sp1_guest_elves() -> io::Result<Sp1GuestElves> {
    let dir = default_guest_elf_dir()?;
    load_sp1_guest_elves_from_dir(dir)
}

/// Load all Shasta guest ELFs from an explicit directory.
///
/// # Errors
///
/// Returns an error if any required ELF file cannot be read from the provided directory.
pub fn load_guest_elves_from_dir(dir: impl AsRef<Path>) -> io::Result<ShastaGuestElves> {
    let dir = dir.as_ref();
    Ok(ShastaGuestElves {
        risc0: load_risc0_guest_elves_from_dir(dir)?,
        sp1: load_sp1_guest_elves_from_dir(dir)?,
    })
}

/// Load all RISC0 Shasta guest ELFs from an explicit directory.
///
/// # Errors
///
/// Returns an error if any required RISC0 ELF file cannot be read from the provided directory.
pub fn load_risc0_guest_elves_from_dir(dir: impl AsRef<Path>) -> io::Result<Risc0GuestElves> {
    let dir = dir.as_ref();
    Ok(Risc0GuestElves {
        proposal: read_elf(dir, RISC0_SHASTA_PROPOSAL_ELF)?,
        aggregation: read_elf(dir, RISC0_SHASTA_AGGREGATION_ELF)?,
    })
}

/// Load all SP1 Shasta guest ELFs from an explicit directory.
///
/// # Errors
///
/// Returns an error if any required SP1 ELF file cannot be read from the provided directory.
pub fn load_sp1_guest_elves_from_dir(dir: impl AsRef<Path>) -> io::Result<Sp1GuestElves> {
    let dir = dir.as_ref();
    Ok(Sp1GuestElves {
        proposal: read_guest_file(dir, SP1_SHASTA_PROPOSAL_ELF)?,
        aggregation: read_guest_file(dir, SP1_SHASTA_AGGREGATION_ELF)?,
        proposal_vk: read_guest_file(dir, SP1_SHASTA_PROPOSAL_VK_BIN)?,
        aggregation_vk: read_guest_file(dir, SP1_SHASTA_AGGREGATION_VK_BIN)?,
    })
}

/// Resolve the fixed guest ELF directory independent of the process working directory.
///
/// # Errors
///
/// Returns an error if `RAIKO2_GUEST_ELF_DIR` is explicitly set to an empty path.
pub fn default_guest_elf_dir() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os(GUEST_ELF_DIR_ENV) {
        if path.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{GUEST_ELF_DIR_ENV} must not be empty"),
            ));
        }
        return Ok(PathBuf::from(path));
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("elf"))
}

fn read_elf(dir: &Path, filename: &str) -> io::Result<Arc<[u8]>> {
    read_guest_file(dir, filename)
}

fn read_guest_file(dir: &Path, filename: &str) -> io::Result<Arc<[u8]>> {
    let path = dir.join(filename);
    fs::read(&path).map(Vec::into).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("failed to read guest file {}: {err}", path.display()),
        )
    })
}

#[cfg(test)]
mod frozen_identity_tests {
    use super::{
        RISC0_SHASTA_AGGREGATION_ELF, RISC0_SHASTA_PROPOSAL_ELF, SP1_SHASTA_AGGREGATION_ELF,
        SP1_SHASTA_AGGREGATION_VK_BIN, SP1_SHASTA_PROPOSAL_ELF, SP1_SHASTA_PROPOSAL_VK_BIN,
    };

    /// Pins the guest artifact filenames recorded in `crates/guests/elf/*.provenance.json`.
    ///
    /// Renaming any of these forces a guest rebuild, which yields new RISC Zero image ids and SP1
    /// verifying keys and therefore requires re-registering verifiers on every network. The
    /// retired Shasta spelling is kept here for exactly that reason.
    #[test]
    fn guest_artifact_filenames_are_frozen_provenance_identity() {
        assert_eq!(RISC0_SHASTA_PROPOSAL_ELF, "risc0_shasta_proposal.elf");
        assert_eq!(RISC0_SHASTA_AGGREGATION_ELF, "risc0_shasta_aggregation.elf");
        assert_eq!(SP1_SHASTA_PROPOSAL_ELF, "sp1_shasta_proposal.elf");
        assert_eq!(SP1_SHASTA_AGGREGATION_ELF, "sp1_shasta_aggregation.elf");
        assert_eq!(SP1_SHASTA_PROPOSAL_VK_BIN, "sp1_shasta_proposal.vk.bin");
        assert_eq!(
            SP1_SHASTA_AGGREGATION_VK_BIN,
            "sp1_shasta_aggregation.vk.bin"
        );
    }
}
