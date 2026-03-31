mod backends;
mod manifest;
mod spec;

pub use backends::{RISC0_UZEN_BACKEND, SP1_UZEN_BACKEND};
pub use manifest::UzenManifestBuilder;
pub use spec::UzenSpec;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProofStage, ProverBackend};
    use raiko2_guests::{risc0, sp1};

    #[test]
    fn uzen_backends_return_expected_elves() -> Result<(), Box<dyn std::error::Error>> {
        let risc0_proposal = RISC0_UZEN_BACKEND.elf(ProofStage::Proposal)?;
        let risc0_agg = RISC0_UZEN_BACKEND.elf(ProofStage::Aggregation)?;
        assert_eq!(risc0_proposal, risc0::uzen::PROPOSAL_ELF);
        assert_eq!(risc0_agg, risc0::uzen::AGGREGATION_ELF);

        let sp1_proposal = SP1_UZEN_BACKEND.elf(ProofStage::Proposal)?;
        let sp1_agg = SP1_UZEN_BACKEND.elf(ProofStage::Aggregation)?;
        assert_eq!(sp1_proposal, sp1::uzen::PROPOSAL_ELF);
        assert_eq!(sp1_agg, sp1::uzen::AGGREGATION_ELF);
        Ok(())
    }
}
