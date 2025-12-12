//! Application state for the HTTP server.

use crate::config::{Config, ProverType, QueueBackend};
use anyhow::Result;
use raiko2_engine::input_builder::{GuestInputBuilder, NetworkGuestInputBuilder};
use raiko2_engine::queue::EngineQueue;
use raiko2_prover::Prover;
use std::sync::Arc;

#[cfg(feature = "redis-queue")]
use raiko2_engine::tasks::{EngineOutput, EngineTask};
#[cfg(feature = "redis-queue")]
use std::time::Duration;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub engine: EngineQueue,
}

impl AppState {
    /// Create new application state.
    pub async fn new(config: Config) -> Result<Self> {
        let prover: Arc<dyn Prover> = match config.prover.prover_type {
            ProverType::Risc0 => {
                let risc0_config = raiko2_prover::risc0::Risc0Config {
                    bonsai: config.prover.risc0.bonsai,
                    snark: config.prover.risc0.snark,
                    profile: false,
                    execution_po2: 20,
                };
                Arc::new(raiko2_prover::risc0::Risc0Prover::new(risc0_config))
            }
            ProverType::Sp1 => {
                let sp1_config = raiko2_prover::sp1::Sp1Config {
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
                Arc::new(raiko2_prover::sp1::Sp1Prover::new(sp1_config))
            }
        };

        let proof_type = match config.prover.prover_type {
            ProverType::Risc0 => "risc0".to_string(),
            ProverType::Sp1 => "sp1".to_string(),
        };
        let guest_input_builder: Arc<dyn GuestInputBuilder> = Arc::new(
            NetworkGuestInputBuilder::new(
                &config.rpc.l2_rpc,
                config.rpc.l1_chain_id,
                config.rpc.l2_chain_id,
                proof_type,
                raiko2_primitives::ProverConfig::default(),
            )
            .map_err(|e| anyhow::anyhow!(e))?,
        );

        let engine = match config.queue.backend {
            QueueBackend::Memory => EngineQueue::with_store_and_builder(
                prover,
                raiko2_queue::MemoryStore::new(),
                guest_input_builder.clone(),
            ),
            QueueBackend::Redis => {
                #[cfg(feature = "redis-queue")]
                {
                    let url = config.queue.redis_url.clone().unwrap_or_default();
                    let store = raiko2_queue::RedisStore::<EngineTask, EngineOutput>::connect(
                        &url,
                        &config.queue.namespace,
                        Duration::from_secs(60),
                    )
                    .await?;
                    EngineQueue::with_store_and_builder(prover, store, guest_input_builder.clone())
                }

                #[cfg(not(feature = "redis-queue"))]
                {
                    anyhow::bail!(
                        "queue backend redis requires building raiko2 with `--features redis-queue`"
                    );
                }
            }
        };

        engine.start_workers(config.queue.workers);

        Ok(Self {
            config: Arc::new(config),
            engine,
        })
    }
}
