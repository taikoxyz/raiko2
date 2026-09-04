mod backends;
mod checkpoint_verify;
mod manifest;
pub mod preflight_cache;
mod spec;

use raiko2_primitives::{ChainSpec, ProofContext, RaikoError, RaikoResult, SupportedChainSpecs};

pub use backends::{
    ShastaBackends, load_risc0_boundless_shasta_backend, load_risc0_shasta_backend,
    load_shasta_backends, load_sp1_shasta_backend, risc0_boundless_shasta_backend_from_elves,
    risc0_shasta_backend_from_elves, shasta_backends_from_elves, sp1_shasta_backend_from_elves,
};
pub use checkpoint_verify::{
    compare_guest_input_checkpoint_against_l2_blocks, verify_guest_input_checkpoint_against_l2_rpc,
};
pub use manifest::ShastaManifestBuilder;
pub use spec::{ShastaSpec, validate_shasta_guest_input};

// ELF selection is handled by the backend instance.

const UNKNOWN_L2_CHAIN_SPEC_NAME: &str = "unknown";

/// Host-side L2 chain spec resolution: config/CLI override first, compiled defaults second.
///
/// Guest programs do not trust this helper; they validate witness chain specs against their own
/// compiled default list.
fn host_l2_chain_spec_from_context(ctx: &ProofContext) -> RaikoResult<ChainSpec> {
    if let Some(chain_spec) = &ctx.preflight.resolved_l2_chain_spec {
        if chain_spec.chain_id != ctx.request.l2_chain_id {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "preflight l2 chain spec chain_id {} does not match request l2_chain_id {}",
                chain_spec.chain_id, ctx.request.l2_chain_id
            )));
        }
        return Ok(chain_spec.clone());
    }

    Ok(SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(ctx.request.l2_chain_id)
        .unwrap_or_else(|| ChainSpec {
            name: UNKNOWN_L2_CHAIN_SPEC_NAME.to_string(),
            chain_id: ctx.request.l2_chain_id,
            // Host/preflight fallback for ad-hoc Taiko L2 testing. Guests still reject unknown
            // chain IDs unless they are compiled into the trusted chain-spec list.
            is_taiko: true,
            ..Default::default()
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProofStage, ProverBackend};
    use raiko2_guests::load_shasta_guest_elves;
    use raiko2_primitives::{ProofRequest, ProverConfig};

    #[test]
    fn shasta_backends_return_expected_elves() -> Result<(), Box<dyn std::error::Error>> {
        let elves = load_shasta_guest_elves()?;
        let expected_risc0_proposal = elves.risc0.proposal.clone();
        let expected_risc0_agg = elves.risc0.aggregation.clone();
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
        assert_eq!(boundless_agg, expected_risc0_agg.as_ref());

        let sp1_proposal = backends.sp1.elf(ProofStage::Proposal)?;
        let sp1_agg = backends.sp1.elf(ProofStage::Aggregation)?;
        assert_eq!(sp1_proposal, expected_sp1_proposal.as_ref());
        assert_eq!(sp1_agg, expected_sp1_agg.as_ref());
        Ok(())
    }

    #[test]
    fn host_l2_chain_spec_unknown_fallback_is_taiko() {
        let ctx = ProofContext::new(
            ProofRequest {
                l2_chain_id: 999_999,
                ..Default::default()
            },
            ProverConfig::default(),
        );

        let spec = host_l2_chain_spec_from_context(&ctx).expect("fallback chain spec");

        assert_eq!(spec.name, UNKNOWN_L2_CHAIN_SPEC_NAME);
        assert_eq!(spec.chain_id, 999_999);
        assert!(spec.is_taiko);
    }
}
