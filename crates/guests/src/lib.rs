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

#[derive(Clone, Debug)]
pub struct ShastaGuestElves {
    pub risc0: Risc0ShastaGuestElves,
    pub sp1: Sp1ShastaGuestElves,
}

#[derive(Clone, Debug)]
pub struct Risc0ShastaGuestElves {
    pub proposal: Arc<[u8]>,
    pub aggregation: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub struct Sp1ShastaGuestElves {
    pub proposal: Arc<[u8]>,
    pub aggregation: Arc<[u8]>,
}

/// Load all Shasta guest ELFs from the default fixed repository/runtime path.
///
/// # Errors
///
/// Returns an error if the guest ELF directory cannot be resolved or any required ELF file cannot
/// be read.
pub fn load_shasta_guest_elves() -> io::Result<ShastaGuestElves> {
    let dir = default_guest_elf_dir()?;
    load_shasta_guest_elves_from_dir(dir)
}

/// Load all RISC0 Shasta guest ELFs from the default fixed repository/runtime path.
///
/// # Errors
///
/// Returns an error if the guest ELF directory cannot be resolved or any required RISC0 ELF file
/// cannot be read.
pub fn load_risc0_shasta_guest_elves() -> io::Result<Risc0ShastaGuestElves> {
    let dir = default_guest_elf_dir()?;
    load_risc0_shasta_guest_elves_from_dir(dir)
}

/// Load all SP1 Shasta guest ELFs from the default fixed repository/runtime path.
///
/// # Errors
///
/// Returns an error if the guest ELF directory cannot be resolved or any required SP1 ELF file
/// cannot be read.
pub fn load_sp1_shasta_guest_elves() -> io::Result<Sp1ShastaGuestElves> {
    let dir = default_guest_elf_dir()?;
    load_sp1_shasta_guest_elves_from_dir(dir)
}

/// Load all Shasta guest ELFs from an explicit directory.
///
/// # Errors
///
/// Returns an error if any required ELF file cannot be read from the provided directory.
pub fn load_shasta_guest_elves_from_dir(dir: impl AsRef<Path>) -> io::Result<ShastaGuestElves> {
    let dir = dir.as_ref();
    Ok(ShastaGuestElves {
        risc0: load_risc0_shasta_guest_elves_from_dir(dir)?,
        sp1: load_sp1_shasta_guest_elves_from_dir(dir)?,
    })
}

/// Load all RISC0 Shasta guest ELFs from an explicit directory.
///
/// # Errors
///
/// Returns an error if any required RISC0 ELF file cannot be read from the provided directory.
pub fn load_risc0_shasta_guest_elves_from_dir(
    dir: impl AsRef<Path>,
) -> io::Result<Risc0ShastaGuestElves> {
    let dir = dir.as_ref();
    Ok(Risc0ShastaGuestElves {
        proposal: read_elf(dir, RISC0_SHASTA_PROPOSAL_ELF)?,
        aggregation: read_elf(dir, RISC0_SHASTA_AGGREGATION_ELF)?,
    })
}

/// Load all SP1 Shasta guest ELFs from an explicit directory.
///
/// # Errors
///
/// Returns an error if any required SP1 ELF file cannot be read from the provided directory.
pub fn load_sp1_shasta_guest_elves_from_dir(
    dir: impl AsRef<Path>,
) -> io::Result<Sp1ShastaGuestElves> {
    let dir = dir.as_ref();
    Ok(Sp1ShastaGuestElves {
        proposal: read_elf(dir, SP1_SHASTA_PROPOSAL_ELF)?,
        aggregation: read_elf(dir, SP1_SHASTA_AGGREGATION_ELF)?,
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
    let path = dir.join(filename);
    fs::read(&path).map(Vec::into).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("failed to read guest ELF {}: {err}", path.display()),
        )
    })
}
