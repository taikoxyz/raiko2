//! Host-only completion of the sealed Alethia zkGas pricing model.

#![allow(missing_docs)]

mod mpt;
mod proposal_inputs;
mod reconstruct;
mod runtime_quote;

pub use proposal_inputs::{ProposalBlobSelectors, ProposalPayloadSelectors};
pub use raiko2_stateless::workload_lab::ExecutionProfile;
pub use runtime_quote::{
    BlockWorkloadQuote, CodePreparationSelectorsSnapshot, EcmulSemanticCountersSnapshot,
    MptSelectorsSnapshot, NodeBlockWorkloadQuote, NodePricingBreakdownSnapshot,
    NodeProposalWorkloadQuote, NodeSemanticCountersSnapshot, PricingBreakdownSnapshot,
    ProposalPricingSelectorsSnapshot, ProposalWorkloadQuote, ProveBlockWorkloadQuote,
    ProveProposalWorkloadQuote, ResourceSelectorsSnapshot, RuntimeQuoteError,
    SemanticCounterDiagnostic, TransactionPayloadSelectorsSnapshot, quote_shasta_proposal,
    quote_shasta_proposal_with_execution_profile, validate_shasta_proposal_with_execution_profile,
};
