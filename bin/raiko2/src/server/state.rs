//! Application state for the HTTP server.

use crate::config::{Config, ProverType};
use anyhow::Result;
use raiko2_prover::Prover;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Proof job tracking.
#[derive(Debug, Clone)]
#[allow(dead_code)] // batch_id will be used for job queries
pub struct ProofJob {
    pub id: String,
    pub batch_id: u64,
    pub status: ProofJobStatus,
    pub proof: Option<String>,
    pub error: Option<String>,
}

/// Proof job status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofJobStatus {
    Pending,
    Proving,
    Completed,
    Failed,
    Cancelled,
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub prover: Arc<dyn Prover>,
    pub jobs: Arc<RwLock<HashMap<String, ProofJob>>>,
}

impl AppState {
    /// Create new application state.
    pub fn new(config: Config) -> Result<Self> {
        let prover: Arc<dyn Prover> = match config.prover.prover_type {
            #[cfg(feature = "risc0")]
            ProverType::Risc0 => {
                let risc0_config = raiko2_prover::Risc0Config {
                    bonsai: config.prover.risc0.bonsai,
                    snark: config.prover.risc0.snark,
                    profile: false,
                    execution_po2: 20,
                };
                Arc::new(raiko2_prover::Risc0Prover::new(risc0_config))
            }
            #[cfg(feature = "sp1")]
            ProverType::Sp1 => {
                let sp1_config = raiko2_prover::Sp1Config {
                    recursion: if config.prover.sp1.plonk {
                        raiko2_prover::sp1::RecursionMode::Plonk
                    } else {
                        raiko2_prover::sp1::RecursionMode::Compressed
                    },
                    prover: if config.prover.sp1.network {
                        Some(raiko2_prover::sp1::ProverMode::Network)
                    } else {
                        Some(raiko2_prover::sp1::ProverMode::Local)
                    },
                    verify: true,
                };
                Arc::new(raiko2_prover::Sp1Prover::new(sp1_config))
            }
            #[cfg(not(feature = "risc0"))]
            ProverType::Risc0 => {
                anyhow::bail!(
                    "Risc0 prover requested but 'risc0' feature is not enabled. Build with --features risc0"
                );
            }
            #[cfg(not(feature = "sp1"))]
            ProverType::Sp1 => {
                anyhow::bail!(
                    "SP1 prover requested but 'sp1' feature is not enabled. Build with --features sp1"
                );
            }
        };

        Ok(Self {
            config: Arc::new(config),
            prover,
            jobs: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Add a new proof job.
    pub async fn add_job(&self, job: ProofJob) {
        let mut jobs = self.jobs.write().await;
        jobs.insert(job.id.clone(), job);
    }

    /// Get a proof job by ID.
    pub async fn get_job(&self, id: &str) -> Option<ProofJob> {
        let jobs = self.jobs.read().await;
        jobs.get(id).cloned()
    }

    /// Update a proof job.
    pub async fn update_job(
        &self,
        id: &str,
        status: ProofJobStatus,
        proof: Option<String>,
        error: Option<String>,
    ) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(id) {
            job.status = status;
            job.proof = proof;
            job.error = error;
        }
    }
}
