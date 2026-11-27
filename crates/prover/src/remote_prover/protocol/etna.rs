//! Etna transport reusing existing execution witnesses and proof response framing.

use alloy_primitives::{Address, B256};
use raiko2_primitives::{ExecutionWitness, WitnessHeader, WitnessStateNode};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::input::ConsensusRangeInput;
use serde::{Deserialize, Serialize};

/// Explicit schema: Etna statements do not use the Shasta carry-data relation.
pub const RAIKO2_ETNA_REQUEST_SCHEMA: &str = "raiko2-etna-request-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Raiko2EtnaRequest {
    pub schema: String,
    pub payload: Raiko2EtnaPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Raiko2EtnaPayload {
    /// Supplied native blocks, certificates, SSZ history and L1 state openings.
    pub input: ConsensusRangeInput<ExecutionWitness>,
    /// Existing blob/KZG input type, with the statement's original ordered blobs.
    pub data_source: InputDataSource,
    /// Existing execution resources in exact covered-height order.
    pub executions: Vec<Raiko2EtnaExecutionWitness>,
    pub shared_state_nodes: Vec<WitnessStateNode>,
    /// Configuration preimage, checked against the statement and proven Inbox state.
    pub staking: Address,
    pub unsettled_limit: u64,
    /// Concrete SGX subverifier, used in its on-chain signature domain.
    pub verifier_address: Address,
    /// Immutable client job identity; compared with the verified landing digest.
    pub expected_statement_hash: B256,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Raiko2EtnaExecutionWitness {
    pub witness: ExecutionWitness,
    pub ancestor_headers: Vec<WitnessHeader>,
}
