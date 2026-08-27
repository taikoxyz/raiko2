use crate::{Risc0ProposalBackend, ShastaElfBackend, Sp1ProposalBackend};
use raiko2_guests::{
    Risc0GuestElves, ShastaGuestElves, Sp1GuestElves, load_guest_elves, load_risc0_guest_elves,
    load_sp1_guest_elves,
};
use raiko2_primitives::{RaikoError, RaikoResult};

#[derive(Debug, Clone)]
pub struct ProposalBackends {
    pub risc0: Risc0ProposalBackend,
    pub risc0_boundless: Risc0ProposalBackend,
    pub sp1: Sp1ProposalBackend,
}

/// Load Shasta backend instances from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if any required Shasta guest ELF cannot be read.
pub fn load_proposal_backends() -> RaikoResult<ProposalBackends> {
    let elves = load_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load Shasta guest ELFs: {err}"))
    })?;
    Ok(proposal_backends_from_elves(elves))
}

/// Load the RISC0 local Shasta backend from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if a required RISC0 local guest ELF cannot be read.
pub fn load_risc0_proposal_backend() -> RaikoResult<Risc0ProposalBackend> {
    let elves = load_risc0_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load RISC0 Shasta guest ELFs: {err}"))
    })?;
    Ok(risc0_proposal_backend_from_elves(elves))
}

/// Load the RISC0 Boundless Shasta backend from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if a required RISC0 Boundless guest ELF cannot be read.
pub fn load_risc0_boundless_proposal_backend() -> RaikoResult<Risc0ProposalBackend> {
    let elves = load_risc0_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load RISC0 Shasta guest ELFs: {err}"))
    })?;
    Ok(risc0_boundless_proposal_backend_from_elves(elves))
}

/// Load the SP1 Shasta backend from the fixed guest ELF path.
///
/// # Errors
///
/// Returns an error if a required SP1 guest ELF cannot be read.
pub fn load_sp1_proposal_backend() -> RaikoResult<Sp1ProposalBackend> {
    let elves = load_sp1_guest_elves().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("failed to load SP1 Shasta guest ELFs: {err}"))
    })?;
    Ok(sp1_proposal_backend_from_elves(elves))
}

#[must_use]
pub fn proposal_backends_from_elves(elves: ShastaGuestElves) -> ProposalBackends {
    ProposalBackends {
        risc0: risc0_proposal_backend_from_elves(elves.risc0.clone()),
        risc0_boundless: risc0_boundless_proposal_backend_from_elves(elves.risc0),
        sp1: sp1_proposal_backend_from_elves(elves.sp1),
    }
}

#[must_use]
pub fn risc0_proposal_backend_from_elves(elves: Risc0GuestElves) -> Risc0ProposalBackend {
    Risc0ProposalBackend::from_elf_backend(ShastaElfBackend::new(elves.proposal, elves.aggregation))
}

#[must_use]
pub fn risc0_boundless_proposal_backend_from_elves(elves: Risc0GuestElves) -> Risc0ProposalBackend {
    Risc0ProposalBackend::from_elf_backend(ShastaElfBackend::new(elves.proposal, elves.aggregation))
}

#[must_use]
pub fn sp1_proposal_backend_from_elves(elves: Sp1GuestElves) -> Sp1ProposalBackend {
    Sp1ProposalBackend::new(
        elves.proposal,
        elves.aggregation,
        elves.proposal_vk,
        elves.aggregation_vk,
    )
}
