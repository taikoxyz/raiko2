use serde::{Deserialize, Serialize};

/// Proof status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProofStatus {
    Pending,
    Proving,
    Completed,
    Failed,
    Cancelled,
}

/// Engine task status summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatusView {
    pub status: ProofStatus,
    pub proof: Option<String>,
    pub error: Option<String>,
}
