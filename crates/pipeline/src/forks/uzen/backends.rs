use crate::{Risc0ShastaBackend, ShastaElfBackend, Sp1ShastaBackend};
use raiko2_guests::{risc0, sp1};

/// RISC0 backend preloaded with Uzen guest ELFs.
pub const RISC0_UZEN_BACKEND: Risc0ShastaBackend = Risc0ShastaBackend::from_elf_backend(
    ShastaElfBackend::new(risc0::uzen::PROPOSAL_ELF, risc0::uzen::AGGREGATION_ELF),
);

/// SP1 backend preloaded with Uzen guest ELFs.
pub const SP1_UZEN_BACKEND: Sp1ShastaBackend = Sp1ShastaBackend::from_elf_backend(
    ShastaElfBackend::new(sp1::uzen::PROPOSAL_ELF, sp1::uzen::AGGREGATION_ELF),
);
