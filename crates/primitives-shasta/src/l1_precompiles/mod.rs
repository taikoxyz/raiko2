//! L1 precompile verification primitives for the Shasta path.
//!
//! Provides the data structures and verification helpers that raiko2's pipeline
//! and guest use to verify L1SLOAD storage proofs and L1STATICCALL execution
//! witnesses before populating the `alethia-reth-evm` precompile caches.
//!
//! **Trust chain**:
//! 1. The L1 origin block hash is bound on-chain via the proposal commitment
//!    (`originBlockHash` in Shasta). Written by the L1 EVM via `blockhash(...)`
//!    during `propose()`, so the value is cryptographically verifiable.
//! 2. `TaikoManifest.l1_header` is the L1 header at that origin block. The
//!    protocol-instance layer asserts
//!    `taiko_manifest.l1_header.hash_slow() == proposal.originBlockHash`.
//! 3. [`build_verified_state_root_map`] walks `parent_hash` backward from
//!    `taiko_manifest.l1_header` through `TaikoManifest.l1_ancestor_headers`,
//!    producing a `block_number → state_root` map bound to the trusted root at
//!    every step.
//! 4. L1SLOAD MPT proofs and L1STATICCALL witnesses are verified against those
//!    roots before per-call results are populated into the precompile cache.

pub mod l1sload;
pub mod l1staticcall;
pub mod witness_db;

use std::sync::{LazyLock, Mutex, MutexGuard};

use alloy_primitives::{Address, B256, Bytes};
use serde::{Deserialize, Serialize};

pub use l1sload::{
    build_verified_state_root_map, clear_l1sload_cache, populate_l1sload_cache,
    verify_and_populate_l1sload_proofs,
};

pub use l1staticcall::{
    verify_and_populate_l1_staticcall_witnesses,
    verify_and_populate_l1_staticcall_witnesses_with_headers,
};

/// Re-export shared constants so downstream raiko2 consumers can import them from this module
/// without adding `alethia-reth-evm` as a direct dependency.
pub use alethia_reth_evm::precompiles::l1staticcall::{L1_PRECOMPILE_CALLER, L1STATICCALL_GAS_CAP};

/// Re-export L1 RPC fallback functions for L1SLOAD support
pub use alethia_reth_evm::precompiles::l1sload::{
    clear_l1_rpc_fetcher, clear_l1_rpc_served_calls, set_l1_origin_block_id, set_l1_rpc_fetcher,
    take_l1_rpc_served_calls,
};

/// Re-export L1STATICCALL RPC fallback functions
pub use alethia_reth_evm::precompiles::l1staticcall::{
    L1StaticCallRecord, clear_l1_staticcall_cache, clear_l1_staticcall_rpc_fetcher,
    clear_l1_staticcall_rpc_served_calls, set_l1_staticcall_rpc_fetcher,
    take_l1_staticcall_rpc_served_calls,
};

/// L1SLOAD storage proof — verified against a trusted L1 state root before the
/// resulting `value` is dropped into the precompile cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L1StorageProof {
    /// L1 contract address.
    pub contract_address: Address,
    /// Storage slot key (32 bytes).
    pub storage_key: B256,
    /// L1 block number (B256-encoded — matches the precompile cache key).
    pub block_number: B256,
    /// Storage value at `(contract_address, storage_key, block_number)`.
    pub value: B256,
    /// Merkle-Patricia proof for the account at `state_root`.
    pub account_proof: Vec<Bytes>,
    /// Merkle-Patricia proof for the storage slot at `account.storage_root`.
    pub storage_proof: Vec<Bytes>,
}

/// L1STATICCALL execution witness — a self-contained re-execution package.
///
/// The ZK guest verifies the witness against a trusted state root, rebuilds an
/// in-memory MPT, re-executes the L1 view function with `revm`, and asserts the
/// output + gas consumed match the recorded values. Only then is the result
/// populated into the precompile cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L1StaticCallWitness {
    /// L1 contract address targeted by the call.
    pub target_address: Address,
    /// L1 block number at which the call was executed.
    pub block_number: u64,
    /// Calldata sent to the L1 contract.
    pub calldata: Bytes,
    /// Return data captured from the L1 call.
    pub return_data: Bytes,
    /// Actual gas consumed on L1, as reported by `debug_traceCall`.
    pub gas_used: u64,
    /// Whether the L1 call reverted.
    #[serde(default)]
    pub is_reverted: bool,
    /// Execution witness package (trie nodes, codes, keys, headers).
    pub execution_witness: L1ExecutionWitness,
}

/// Self-contained witness package returned by NMC's `proof_call`.
///
/// **Note**: distinct from `raiko2_primitives::ExecutionWitness`. This is the
/// L1-side, single-call witness format (flat lists of MPT-node RLP, contract
/// bytecodes, key preimages, and ancestor headers). raiko2's `ExecutionWitness`
/// is a structured L2-side, block-level format (`WitnessStateNode`,
/// `WitnessHeader`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct L1ExecutionWitness {
    /// MPT trie node preimages.
    pub state: Vec<Bytes>,
    /// Contract bytecodes touched during the call.
    pub codes: Vec<Bytes>,
    /// Unhashed addresses and storage slots — cleartext keys for hashed-trie paths.
    pub keys: Vec<Bytes>,
    /// RLP-encoded block headers needed to satisfy `BLOCKHASH` opcode lookups.
    pub headers: Vec<Bytes>,
}

/// Serializes the `clear → populate → execute → finalize` cycle for both precompiles. They share
/// process-global state (origin context + per-precompile caches/fetchers/served-calls), so
/// concurrent proving tasks would cross-contaminate. Held through proof construction; lifting to
/// per-call context would require an alethia-reth precompile redesign.
pub(crate) static L1_PRECOMPILE_EXECUTION_LOCK: LazyLock<Mutex<()>> =
    LazyLock::new(|| Mutex::new(()));

/// Acquire the execution lock for the whole `clear → populate → execute → finalize` cycle.
/// Recovers from a poisoned lock via `into_inner()` (the next acquirer resets state via
/// [`reset_l1_precompile_state`]), so one panicked proving task can't wedge every subsequent one.
pub fn acquire_l1_precompile_lock() -> MutexGuard<'static, ()> {
    L1_PRECOMPILE_EXECUTION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reset all L1 precompile global state to a clean baseline. Sweeps **both halves** of every
/// precompile (cache + RPC fetcher slot + served-calls list for L1SLOAD and L1STATICCALL) plus
/// the shared origin context — so a panic mid-cycle can't leak a stale fetcher into the next
/// task recovered via [`acquire_l1_precompile_lock`].
pub fn reset_l1_precompile_state() {
    // L1SLOAD: cache + origin context (shared) + fetcher slot + served-calls list.
    // `clear_l1sload_cache` already calls `clear_l1_origin_context` + `clear_l1_rpc_fetcher` +
    // `clear_l1_rpc_served_calls` via `alethia_reth_evm::precompiles::l1sload::clear_l1_storage`,
    // but we spell the latter two out explicitly so a future refactor of `clear_l1_storage`
    // can't silently drop them.
    clear_l1sload_cache();
    clear_l1_rpc_fetcher();
    clear_l1_rpc_served_calls();
    // L1STATICCALL: cache + fetcher slot + served-calls list.
    clear_l1_staticcall_cache();
    clear_l1_staticcall_rpc_fetcher();
    clear_l1_staticcall_rpc_served_calls();
}
