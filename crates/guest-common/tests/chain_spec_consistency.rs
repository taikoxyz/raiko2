//! Consistency guard: every chain ID special-cased in the guest's anchor logic must have a
//! trusted, compiled-in chain spec. Historically this drifted and the guest failed *open* (F-1).
//! See docs/plans/2026-06-29-f1-chain-spec-fail-closed-design.md.

use raiko2_primitives::SupportedChainSpecs;
use raiko2_primitives_shasta::MAINNET_WINDOW_CHAIN_IDS;

#[test]
fn every_special_cased_chain_id_has_a_trusted_spec() {
    let specs = SupportedChainSpecs::default();
    for &chain_id in MAINNET_WINDOW_CHAIN_IDS {
        assert!(
            specs.get_chain_spec_with_chain_id(chain_id).is_some(),
            "chain id {chain_id} is special-cased in anchor.rs but has no trusted chain-spec entry"
        );
    }
}
