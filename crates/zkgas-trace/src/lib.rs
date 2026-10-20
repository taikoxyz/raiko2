//! Host-only completion of the sealed Alethia zkGas pricing model.

#![allow(missing_docs)]

mod mpt;
mod proposal_inputs;
mod reconstruct;
mod runtime_quote;

pub use proposal_inputs::{ProposalBlobSelectors, ProposalPayloadSelectors};
pub use runtime_quote::{
    BlockWorkloadQuote, PricingBreakdownSnapshot, ProposalWorkloadQuote, RuntimeQuoteError,
    quote_shasta_proposal,
};
