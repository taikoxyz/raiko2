//! Raiko2 Engine - queue-driven proving orchestration.
//!
//! ## Module Structure
//!
//! - `queue` - Task queue and scheduling for proof jobs (main entrypoint)
//! - `worker` - Supervised worker management with auto-restart
//! - `tasks` - Task types and outputs
//!
//! Prover integration is provided via `raiko2-prover` and wired through
//! `queue::Engine` / `tasks::EngineTask`.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

pub mod queue;
pub mod tasks;
pub mod worker;

pub use queue::Engine;
pub use tasks::{BatchStage, EngineTaskId, EngineTaskKey};
