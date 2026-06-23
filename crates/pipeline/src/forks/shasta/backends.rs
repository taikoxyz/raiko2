use crate::{Risc0ShastaBackend, ShastaElfBackend, Sp1ShastaBackend};
use raiko2_guests::{
    Risc0ShastaGuestElves, ShastaGuestElves, Sp1ShastaGuestElves, load_risc0_shasta_guest_elves,
    load_shasta_guest_elves, load_sp1_shasta_guest_elves,
};
use raiko2_primitives::{RaikoError, RaikoResult};

#[derive(Debug, Clone)]
pub struct ShastaBackends {
    pub risc0: Risc0ShastaBackend,
    pub risc0_boundless: Risc0ShastaBackend,
    pub sp1: Sp1ShastaBackend,
}

/// Load Shasta backend instances from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if any required Shasta guest ELF cannot be read.
pub fn load_shasta_backends() -> RaikoResult<ShastaBackends> {
    let elves = load_shasta_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load Shasta guest ELFs: {err}"))
    })?;
    Ok(shasta_backends_from_elves(elves))
}

/// Load the RISC0 local Shasta backend from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if a required RISC0 local guest ELF cannot be read.
pub fn load_risc0_shasta_backend() -> RaikoResult<Risc0ShastaBackend> {
    let elves = load_risc0_shasta_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load RISC0 Shasta guest ELFs: {err}"))
    })?;
    Ok(risc0_shasta_backend_from_elves(elves))
}

/// Load the RISC0 Boundless Shasta backend from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if a required RISC0 Boundless guest ELF cannot be read.
pub fn load_risc0_boundless_shasta_backend() -> RaikoResult<Risc0ShastaBackend> {
    let elves = load_risc0_shasta_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load RISC0 Shasta guest ELFs: {err}"))
    })?;
    Ok(risc0_boundless_shasta_backend_from_elves(elves))
}

/// Load the SP1 Shasta backend from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if a required SP1 guest ELF cannot be read.
pub fn load_sp1_shasta_backend() -> RaikoResult<Sp1ShastaBackend> {
    let elves = load_sp1_shasta_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load SP1 Shasta guest ELFs: {err}"))
    })?;
    Ok(sp1_shasta_backend_from_elves(elves))
}

#[must_use]
pub fn shasta_backends_from_elves(elves: ShastaGuestElves) -> ShastaBackends {
    ShastaBackends {
        risc0: risc0_shasta_backend_from_elves(elves.risc0.clone()),
        risc0_boundless: risc0_boundless_shasta_backend_from_elves(elves.risc0),
        sp1: sp1_shasta_backend_from_elves(elves.sp1),
    }
}

#[must_use]
pub fn risc0_shasta_backend_from_elves(elves: Risc0ShastaGuestElves) -> Risc0ShastaBackend {
    Risc0ShastaBackend::from_elf_backend(ShastaElfBackend::new(elves.proposal, elves.aggregation))
}

#[must_use]
pub fn risc0_boundless_shasta_backend_from_elves(
    elves: Risc0ShastaGuestElves,
) -> Risc0ShastaBackend {
    Risc0ShastaBackend::from_elf_backend(ShastaElfBackend::new(elves.proposal, elves.aggregation))
}

#[must_use]
pub fn sp1_shasta_backend_from_elves(elves: Sp1ShastaGuestElves) -> Sp1ShastaBackend {
    Sp1ShastaBackend::new(
        elves.proposal,
        elves.aggregation,
        elves.proposal_vk,
        elves.aggregation_vk,
    )
}
