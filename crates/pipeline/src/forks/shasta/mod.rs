mod backends;
mod manifest;
mod spec;

pub use backends::{
    ShastaBackends, load_risc0_boundless_shasta_backend, load_risc0_shasta_backend,
    load_shasta_backends, load_sp1_shasta_backend, risc0_boundless_shasta_backend_from_elves,
    risc0_shasta_backend_from_elves, shasta_backends_from_elves, sp1_shasta_backend_from_elves,
};
pub use manifest::ShastaManifestBuilder;
pub use spec::{ShastaSpec, validate_shasta_guest_input};

// ELF selection is handled by the backend instance.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProofStage, ProverBackend};
    use raiko2_guests::load_shasta_guest_elves;

    #[test]
    fn shasta_backends_return_expected_elves() -> Result<(), Box<dyn std::error::Error>> {
        let elves = load_shasta_guest_elves()?;
        let expected_risc0_proposal = elves.risc0.proposal.clone();
        let expected_risc0_agg = elves.risc0.aggregation.clone();
        let expected_boundless_agg = elves.risc0.boundless_aggregation.clone();
        let expected_sp1_proposal = elves.sp1.proposal.clone();
        let expected_sp1_agg = elves.sp1.aggregation.clone();
        let backends = shasta_backends_from_elves(elves);

        let risc0_proposal = backends.risc0.elf(ProofStage::Proposal)?;
        let risc0_agg = backends.risc0.elf(ProofStage::Aggregation)?;
        assert_eq!(risc0_proposal, expected_risc0_proposal.as_ref());
        assert_eq!(risc0_agg, expected_risc0_agg.as_ref());

        let boundless_proposal = backends.risc0_boundless.elf(ProofStage::Proposal)?;
        let boundless_agg = backends.risc0_boundless.elf(ProofStage::Aggregation)?;
        assert_eq!(boundless_proposal, expected_risc0_proposal.as_ref());
        assert_eq!(boundless_agg, expected_boundless_agg.as_ref());

        let sp1_proposal = backends.sp1.elf(ProofStage::Proposal)?;
        let sp1_agg = backends.sp1.elf(ProofStage::Aggregation)?;
        assert_eq!(sp1_proposal, expected_sp1_proposal.as_ref());
        assert_eq!(sp1_agg, expected_sp1_agg.as_ref());
        Ok(())
    }
}
