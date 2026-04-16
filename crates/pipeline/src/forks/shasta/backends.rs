use crate::{Risc0ShastaBackend, ShastaElfBackend, Sp1ShastaBackend};
use raiko2_guests::{risc0, sp1};

/// RISC0 backend preloaded with Shasta ELFs.
pub const RISC0_SHASTA_BACKEND: Risc0ShastaBackend = Risc0ShastaBackend::from_elf_backend(
    ShastaElfBackend::new(risc0::shasta::PROPOSAL_ELF, risc0::shasta::AGGREGATION_ELF),
);

/// RISC0 Boundless backend with a receipt-backed aggregation guest.
pub const RISC0_BOUNDLESS_SHASTA_BACKEND: Risc0ShastaBackend =
    Risc0ShastaBackend::from_elf_backend(ShastaElfBackend::new(
        risc0::shasta::PROPOSAL_ELF,
        risc0::shasta::BOUNDLESS_AGGREGATION_ELF,
    ));

/// SP1 backend preloaded with Shasta ELFs.
pub const SP1_SHASTA_BACKEND: Sp1ShastaBackend = Sp1ShastaBackend::from_elf_backend(
    ShastaElfBackend::new(sp1::shasta::PROPOSAL_ELF, sp1::shasta::AGGREGATION_ELF),
);
