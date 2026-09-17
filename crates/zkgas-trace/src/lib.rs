//! Host-only zk-gas trace collection for Shasta proposal experiments.

#![allow(missing_docs)]

pub mod inspector;
pub mod reconstruct;
pub mod transactions;

pub use inspector::{
    DispatchStatus, OperationComponent, OperationPhase, OperationTrace, PricingBasis,
    TraceCollector, TraceInspector, TraceSink,
};
pub use reconstruct::{
    BlockTrace, ExecutionParityRecord, ParityStatus, PartialBlockTrace, ProposalTrace,
    ProposalTraceStatus, ProposalTraceSummary, RecoveryFailure, TraceFailure,
    TracingDerivedBlockExecutor, guest_input_identity, trace_shasta_proposal,
};
pub use transactions::{
    RecoveredTransactionOccurrence, TransactionDisposition, TransactionTrace,
    classify_started_occurrences, classify_started_occurrences_with_statuses,
    is_native_value_transfer,
};
