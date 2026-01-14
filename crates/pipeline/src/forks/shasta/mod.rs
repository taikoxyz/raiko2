mod backends;
mod manifest;
mod spec;

pub use backends::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND};
pub use manifest::ShastaManifestBuilder;
pub use spec::ShastaSpec;

// ELF selection is handled by the backend instance.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProofStage, ProverBackend};
    use raiko2_guests::{risc0, sp1};

    #[test]
    fn shasta_backends_return_expected_elves() {
        let risc0_proposal = RISC0_SHASTA_BACKEND
            .elf(ProofStage::Proposal)
            .expect("risc0 proposal elf");
        let risc0_agg = RISC0_SHASTA_BACKEND
            .elf(ProofStage::Aggregation)
            .expect("risc0 aggregation elf");
        assert_eq!(risc0_proposal, risc0::shasta::PROPOSAL_ELF);
        assert_eq!(risc0_agg, risc0::shasta::AGGREGATION_ELF);

        let sp1_proposal = SP1_SHASTA_BACKEND
            .elf(ProofStage::Proposal)
            .expect("sp1 proposal elf");
        let sp1_agg = SP1_SHASTA_BACKEND
            .elf(ProofStage::Aggregation)
            .expect("sp1 aggregation elf");
        assert_eq!(sp1_proposal, sp1::shasta::PROPOSAL_ELF);
        assert_eq!(sp1_agg, sp1::shasta::AGGREGATION_ELF);
    }
}
