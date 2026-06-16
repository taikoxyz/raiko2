//! L1STATICCALL witness verification and cache population for the ZK guest.
//!
//! Known M9 limitation: reverted L1 calls are trusted from NMC instead of being
//! re-executed in revm. alethia-reth currently surfaces reverts as
//! `PrecompileError`, which drops the post-call gas accounting NMC keeps. We
//! do however enforce that the host-supplied witness for a reverted call carries
//! `gas_used == 0 && return_data.is_empty()` — matching NMC's
//! `GethLikeTxTracer.MarkAsFailed` contract — so a malicious prover cannot
//! fabricate non-zero gas for reverted invocations.

use alethia_reth_evm::precompiles::l1staticcall::{
    L1_PRECOMPILE_CALLER, L1STATICCALL_GAS_CAP, set_l1_staticcall_value,
};
use alloy_consensus::Header;
use alloy_primitives::{Address, B256, U256};
use anyhow::{Result, anyhow, ensure};
use revm::context::TxEnv;
use revm::context::result::ExecutionResult;
use revm::primitives::TxKind;
use revm::{ExecuteEvm, MainBuilder, MainContext};
use std::collections::{HashMap, HashSet};
use tracing::{debug, trace};

use super::L1StaticCallWitness;
use super::witness_db::WitnessDb;

/// Maximum number of L1 blocks to look back from L1 origin. Matches the L2 precompile.
const L1STATICCALL_MAX_BLOCK_LOOKBACK: u64 = 256;

/// Verify and populate L1STATICCALL results from execution witnesses.
///
/// For each witness:
/// 1. Enforce the `[l1_origin − 256, l1_origin]` window so a prover cannot serve a
///    witness from outside the L2 precompile's accepted range (the L2 precompile
///    already enforces it at runtime; we re-enforce it here so the proof binds it).
/// 2. Verify the L1 state root is trusted (from the verified header chain)
/// 3. Build a `WitnessDb` over the witnessed MPT preimages + bytecodes
/// 4. Re-execute the call with revm against the witnessed state
/// 5. Assert revm's `(output, gas_used)` matches the witness claim; reject halts
/// 6. Populate the L1STATICCALL cache with the verified result
///
/// For reverted witnesses (`is_reverted == true`), we skip revm re-execution but
/// enforce `gas_used == 0 && return_data.is_empty()` — matching NMC's
/// `GethLikeTxTracer.MarkAsFailed` contract — so a malicious prover cannot forge
/// non-zero gas for reverts.
pub fn verify_and_populate_l1_staticcall_witnesses(
    witnesses: &[L1StaticCallWitness],
    state_root_map: &HashMap<u64, B256>,
    l1_origin_block_number: u64,
) -> Result<()> {
    verify_and_populate_l1_staticcall_witnesses_with_headers(
        witnesses,
        state_root_map,
        None,
        l1_origin_block_number,
    )
}

/// Richer entrypoint that can also populate the revm block-env with fields from the
/// verified L1 header (timestamp, base_fee, coinbase, prevrandao, blob_base_fee). When
/// `header_map` is `None` (or empty), those fields fall back to revm defaults — honest L1
/// view functions that don't read block-env opcodes still verify successfully, but
/// contracts that read `TIMESTAMP` / `COINBASE` / `BASEFEE` / `BLOBBASEFEE` / `PREVRANDAO`
/// must be proved with populated headers or they'll diverge from the sequencer's run.
///
/// `header_map` is `Option<&HashMap<...>>` rather than a separate function (D14) so callers
/// who don't have headers don't have to materialize an empty map at the call site.
pub fn verify_and_populate_l1_staticcall_witnesses_with_headers(
    witnesses: &[L1StaticCallWitness],
    state_root_map: &HashMap<u64, B256>,
    header_map: Option<&HashMap<u64, &Header>>,
    l1_origin_block_number: u64,
) -> Result<()> {
    if witnesses.is_empty() {
        debug!("L1STATICCALL: no witnesses to verify, skipping");
        return Ok(());
    }

    debug!(
        "L1STATICCALL: verifying {} execution witnesses (l1_origin={})",
        witnesses.len(),
        l1_origin_block_number
    );

    let window_floor = l1_origin_block_number.saturating_sub(L1STATICCALL_MAX_BLOCK_LOOKBACK);

    // Dedup identical (target, block, calldata) triples within the witness slice. The preflight
    // already deduplicates RPC fetches and replicates fetched witnesses back across the original
    // call sequence so the guest sees one record per L1STATICCALL invocation. Re-running the
    // expensive WitnessDb::build + revm transact for the second-and-later replicas is pure
    // wasted ZK cycles: the cache key is `(target, block, calldata)` so the first verification
    // populates the cache and any duplicate invocation will hit the same entry.
    let mut seen: HashSet<(Address, u64, Vec<u8>)> = HashSet::with_capacity(witnesses.len());

    for (i, w) in witnesses.iter().enumerate() {
        // Block-range check mirrors the L2 precompile `[l1origin − 256, l1origin]` window.
        ensure!(
            w.block_number >= window_floor && w.block_number <= l1_origin_block_number,
            "L1STATICCALL: witness #{i} at block {} outside lookback window [{}, {}]",
            w.block_number,
            window_floor,
            l1_origin_block_number,
        );

        debug!(
            "L1STATICCALL: witness #{}: target={:?}, block={}, calldata_len={}, return_len={}, state_nodes={}, codes={}",
            i,
            w.target_address,
            w.block_number,
            w.calldata.len(),
            w.return_data.len(),
            w.execution_witness.state.len(),
            w.execution_witness.codes.len()
        );

        if w.is_reverted {
            // Match NMC's `GethLikeTxTracer.MarkAsFailed`: revert => gas=0 and empty return data.
            // This binds the fragile coupling between the sequencer tracer and the guest so a
            // malicious prover cannot mark an arbitrary call reverted with forged gas.
            //
            // We deliberately reach this branch *before* the state-root lookup: a reverted call
            // doesn't re-execute against L1 state, so requiring an entry in `state_root_map` for
            // every reverted block would add a useless dependency (and surface as a misleading
            // "no verified state root" error when the real cause is the revert itself).
            ensure!(
                w.gas_used == 0 && w.return_data.is_empty(),
                "L1STATICCALL: witness #{i} reverted but carries non-zero gas ({}) or non-empty data ({} bytes) — expected NMC-tracer semantics",
                w.gas_used,
                w.return_data.len(),
            );
            debug!("L1STATICCALL: witness #{i} reverted on L1 — cached as revert with gas=0");
            set_l1_staticcall_value(
                w.target_address,
                w.block_number,
                &w.calldata,
                w.gas_used,
                w.return_data.to_vec(),
                true,
            )
            .map_err(|e| anyhow!("L1STATICCALL #{i}: cache write rejected for reverted witness: {e}"))?;
            continue;
        }

        // State-root lookup is only required for non-reverted witnesses (revm needs a verified
        // root to bind the WitnessDb to the verified L1 chain). Hoisted under the revert check
        // so reverted calls don't pay the dependency.
        let state_root = state_root_map.get(&w.block_number).ok_or_else(|| {
            anyhow!(
                "L1STATICCALL: no verified state root for block {} (witness #{})",
                w.block_number,
                i
            )
        })?;

        // Dedup — skip the heavy WitnessDb + revm work for any duplicate invocation. We
        // keep the per-record cache write below so duplicate-invocation cache reads continue
        // to land. (The first occurrence already executed and `set_l1_staticcall_value`d the
        // verified output under the same (target, block, calldata) cache key.)
        let key = (w.target_address, w.block_number, w.calldata.to_vec());
        if !seen.insert(key) {
            debug!(
                "L1STATICCALL: witness #{i} duplicates an earlier (target, block, calldata) — skipping re-verification"
            );
            continue;
        }

        // Production invariant: a non-reverted witness must carry state to re-execute
        // against. The previous `cfg(test)` fast-path that bypassed verification when state
        // was empty has been removed (D5) so the production verifier carries no test-only
        // branches. Tests that need to populate the cache without revm re-execution call
        // [`populate_cache_skipping_revm_for_tests`] directly.
        ensure!(
            !w.execution_witness.state.is_empty(),
            "L1STATICCALL: witness #{i} has empty state — not permitted in proving"
        );

        // 1a. Bind witness.headers to the trusted L1 chain before WitnessDb sees them.
        //
        // `WitnessDb::block_hash` answers BLOCKHASH opcode lookups inside revm from
        // `witness.headers`. A malicious prover could otherwise serve arbitrary header
        // bytes for an L1 block they never visited, lie about its hash, and (as long as
        // the lie is internally consistent with the witness's claimed `return_data`)
        // slip past the 3-way assertion. Cross-checking each witness header against the
        // trusted `header_map` closes that gap: every header in the witness must either
        // belong to the verified L1 chain at the same hash, or be rejected.
        //
        // Any block_number absent from `header_map` is also rejected — the witness must
        // not claim headers outside the verified chain. We collect the validated
        // (block_number → witness_hash) pairs here so `WitnessDb` doesn't need to decode
        // the same headers again — the cross-check and the BLOCKHASH lookup table share
        // a single decode pass.
        let mut block_hashes: HashMap<u64, B256> =
            HashMap::with_capacity(w.execution_witness.headers.len());
        for (h_idx, hdr_bytes) in w.execution_witness.headers.iter().enumerate() {
            let hdr: Header =
                alloy_rlp::Decodable::decode(&mut hdr_bytes.as_ref()).map_err(|e| {
                    anyhow!("L1STATICCALL #{i}: witness header #{h_idx} decode failed: {e}")
                })?;
            let trusted = header_map
                .and_then(|m| m.get(&hdr.number))
                .ok_or_else(|| {
                    anyhow!(
                        "L1STATICCALL #{i}: witness header #{h_idx} at block {} not in trusted L1 chain",
                        hdr.number,
                    )
                })?;
            let witness_hash = alloy_primitives::keccak256(hdr_bytes.as_ref());
            let trusted_hash = trusted.hash_slow();
            ensure!(
                witness_hash == trusted_hash,
                "L1STATICCALL #{i}: witness header #{h_idx} hash mismatch at block {} \
                 (witness={witness_hash}, trusted={trusted_hash})",
                hdr.number,
            );
            block_hashes.insert(hdr.number, witness_hash);
        }

        // 1b. Build WitnessDb from the (now-validated) execution witness. Pass the
        // already-decoded block_hashes map to avoid a second RLP-decode pass over
        // the witness headers.
        let db =
            WitnessDb::build(&w.execution_witness, *state_root, Some(block_hashes))
                .map_err(|e| anyhow!("L1STATICCALL #{i}: WitnessDb build: {e}"))?;

        let block_number = w.block_number;
        let header = header_map.and_then(|m| m.get(&w.block_number).copied());

        trace!(
            "L1STATICCALL #{i}: target={:?}, block={}, calldata_len={}, state_root={}, \
             witness_state_nodes={}, witness_codes={}, witness_keys={}, witness_headers={}, \
             witness_return_len={}, witness_gas={}, has_header={}",
            w.target_address,
            w.block_number,
            w.calldata.len(),
            state_root,
            w.execution_witness.state.len(),
            w.execution_witness.codes.len(),
            w.execution_witness.keys.len(),
            w.execution_witness.headers.len(),
            w.return_data.len(),
            w.gas_used,
            header.is_some(),
        );

        // 2. Build the call TxEnv for a read-only call (caller = Address::ZERO).
        //    Gas limit matches NMC's cap so revm can complete calls that NMC sequenced.
        //    gas_price is left at 0 so revm doesn't charge the zero-address caller fees
        //    (which would fail since Address::ZERO has no balance in the witness).
        let tx = TxEnv::builder()
            .caller(L1_PRECOMPILE_CALLER)
            .kind(TxKind::Call(w.target_address))
            .data(w.calldata.clone())
            .gas_limit(L1STATICCALL_GAS_CAP)
            .build()
            .map_err(|e| anyhow!("L1STATICCALL #{i}: TxEnv build: {e:?}"))?;

        // 3. Construct a mainnet EVM over the witness. Populate the **full** block env from
        //    the verified L1 header so opcodes like BASEFEE / GASLIMIT / DIFFICULTY /
        //    BLOBBASEFEE return the same values revm sees as the live L1 EL did (S3). Without
        //    this, any L1 contract reading those opcodes would diverge from the witnessed
        //    output and fail the 3-way assertion as `gas_used mismatch` — silently rejecting
        //    proposals from common patterns (Uniswap quoters, gas-refund logic).
        //
        //    The zero-address caller has no balance, so `gas_price >= basefee` would normally
        //    fail validation. `cfg.disable_base_fee` lifts that check (we're already running
        //    `gas_price = 0`).
        let mut evm = revm::Context::mainnet()
            .with_db(db)
            .modify_block_chained(|blk| {
                blk.number = U256::from(block_number);
                if let Some(h) = header {
                    blk.timestamp = U256::from(h.timestamp);
                    blk.beneficiary = h.beneficiary;
                    blk.prevrandao = Some(h.mix_hash);
                    blk.gas_limit = h.gas_limit;
                    blk.difficulty = h.difficulty;
                    if let Some(bf) = h.base_fee_per_gas {
                        blk.basefee = bf;
                    }
                    // EIP-4844 blob basefee. Compute from `excess_blob_gas` per the spec
                    // formula when the header carries the field; otherwise leave revm's
                    // default. Use the Prague update fraction so the computed price matches
                    // L1's post-Prague semantics.
                    if let Some(excess) = h.excess_blob_gas {
                        blk.set_blob_excess_gas_and_price(
                            excess,
                            revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
                        );
                    }
                }
            })
            .modify_cfg_chained(|cfg| {
                // Override EIP-7825's per-tx gas-limit cap so revm accepts the same 30M budget
                // NMC charges. Without this, mainnet-default cfg (post-Osaka) caps tx.gas_limit
                // at 16,777,216 and re-execution rejects the witness with TxGasLimitGreaterThanCap
                // even when the actual on-L1 call used less gas.
                cfg.tx_gas_limit_cap = Some(L1STATICCALL_GAS_CAP);
                // S3: lift the base-fee check so the zero-address caller (`gas_price = 0`)
                // still passes when we populate the real basefee from the header.
                cfg.disable_base_fee = true;
                // NOTE: revm uses its default (latest) mainnet spec and `chain_id = 1` here. The
                // `gas_used` assertion below requires that spec's gas schedule to match the L1 EL's
                // at `block_number` — i.e. L1's active hardfork — and a callee reading the CHAINID
                // opcode would see `1` rather than the real L1 id. Both align today (devnet/Hoodi/
                // mainnet run the latest fork, and no devnet callee reads CHAINID), but a proven L1
                // block predating revm's default across a gas-changing fork boundary, or a CHAINID
                // read on a non-mainnet L1, would fail an honest witness. Follow-up: thread the L1
                // chain id and fork schedule into the guest and set `cfg.chain_id` + `cfg.spec`
                // from the L1 chainspec at `block_number` (code-review-2026-06-01 R1). Deferred
                // with the other GuestInput wire-format additions (D9) to avoid fixture churn.
            })
            .build_mainnet();

        // Capture the full env revm is actually seeing so the root cause of any divergence
        // is on the wire when it happens.
        {
            let cfg = &evm.ctx.cfg;
            let blk = &evm.ctx.block;
            trace!(
                "L1STATICCALL #{i} evm env: spec={:?}, chain_id={}, \
                 blk.number={}, blk.timestamp={}, blk.beneficiary={:?}, blk.basefee={}, \
                 blk.prevrandao={:?}, blk.gas_limit={}, blk.difficulty={}, \
                 blk.blob_excess_gas_and_price={:?}",
                cfg.spec,
                cfg.chain_id,
                blk.number,
                blk.timestamp,
                blk.beneficiary,
                blk.basefee,
                blk.prevrandao,
                blk.gas_limit,
                blk.difficulty,
                blk.blob_excess_gas_and_price,
            );
        }

        // 4. Execute the call
        let outcome = evm
            .transact(tx)
            .map_err(|e| anyhow!("L1STATICCALL #{i}: revm transact: {e:?}"))?;

        // revm 38 moved the gas accounting into a `ResultGas` struct shared by all variants,
        // so we read `tx_gas_used()` (post-refund, EIP-7623-floored) off the top-level result
        // once and destructure the variants for output + halt-reason only.
        let gas_used = outcome.result.tx_gas_used();
        debug!(
            "L1STATICCALL #{i} revm outcome: {:?}",
            match &outcome.result {
                ExecutionResult::Success { output, .. } => format!(
                    "Success(output_len={}, gas={})",
                    output.data().len(),
                    gas_used
                ),
                ExecutionResult::Revert { output, .. } =>
                    format!("Revert(output_len={}, gas={})", output.len(), gas_used),
                ExecutionResult::Halt { reason, .. } => format!("Halt({reason:?}, gas={gas_used})"),
            }
        );

        // 5. Three-way assertion: output + gas_used + status. We're on the non-reverted path
        //    (reverted witnesses `continue` above after the gas==0/empty-data check), so
        //    re-execution must report success. A revert here means the witness's `is_reverted`
        //    flag disagrees with revm; reject it rather than caching the revert payload as a
        //    successful return. (A forged success would otherwise be caught later by the L2 block
        //    state-root check, but failing here keeps the cause local and legible.)
        let output: alloy_primitives::Bytes = match outcome.result {
            ExecutionResult::Success { output, .. } => output.into_data(),
            ExecutionResult::Revert { output, .. } => {
                return Err(anyhow!(
                    "L1STATICCALL #{i}: witness marked non-reverted but revm reverted after {gas_used} gas \
                     (target={:?}, block={}, calldata=0x{}, revert_data=0x{})",
                    w.target_address,
                    w.block_number,
                    alloy_primitives::hex::encode(&w.calldata),
                    alloy_primitives::hex::encode(&output),
                ));
            }
            ExecutionResult::Halt { reason, .. } => {
                return Err(anyhow!(
                    "L1STATICCALL #{i}: halted {reason:?} after {gas_used} gas (target={:?}, block={}, calldata=0x{})",
                    w.target_address,
                    w.block_number,
                    alloy_primitives::hex::encode(&w.calldata),
                ));
            }
        };

        if output.as_ref() != w.return_data.as_ref() {
            return Err(anyhow!(
                "L1STATICCALL #{i}: return_data mismatch (target={:?}, block={}, calldata=0x{}, revm_gas={}, witness_gas={}): \
                 revm returned {} bytes (0x{}), witness expects {} bytes (0x{})",
                w.target_address,
                w.block_number,
                alloy_primitives::hex::encode(&w.calldata),
                gas_used,
                w.gas_used,
                output.len(),
                alloy_primitives::hex::encode(&output),
                w.return_data.len(),
                alloy_primitives::hex::encode(&w.return_data),
            ));
        }
        if gas_used != w.gas_used {
            return Err(anyhow!(
                "L1STATICCALL #{i}: gas_used mismatch (target={:?}, block={}, calldata=0x{}): witness={}, revm={}",
                w.target_address,
                w.block_number,
                alloy_primitives::hex::encode(&w.calldata),
                w.gas_used,
                gas_used
            ));
        }

        // 6. Populate cache with verified gas + data
        set_l1_staticcall_value(
            w.target_address,
            w.block_number,
            &w.calldata,
            w.gas_used,
            output.to_vec(),
            false,
        )
        .map_err(|e| anyhow!("L1STATICCALL #{i}: cache write rejected: {e}"))?;
    }

    debug!(
        "L1STATICCALL: verified and cached {} execution witnesses",
        witnesses.len()
    );
    Ok(())
}

/// Test-only helper that populates the L1STATICCALL cache from a slice of witnesses
/// **without** running revm re-execution or state-root verification. Lets unit tests that
/// only care about cache key/value behavior (dedup, calldata-key uniqueness, cache hit on
/// the precompile) skip the heavy revm setup. Production code MUST go through
/// [`verify_and_populate_l1_staticcall_witnesses_with_headers`].
///
/// Gated behind `cfg(any(test, feature = "test-fixtures"))` so the production guest binary
/// never carries it (D5).
#[cfg(any(test, feature = "test-fixtures"))]
pub fn populate_cache_skipping_revm_for_tests(
    witnesses: &[L1StaticCallWitness],
    l1_origin_block_number: u64,
) -> Result<()> {
    let window_floor = l1_origin_block_number.saturating_sub(L1STATICCALL_MAX_BLOCK_LOOKBACK);
    for (i, w) in witnesses.iter().enumerate() {
        ensure!(
            w.block_number >= window_floor && w.block_number <= l1_origin_block_number,
            "L1STATICCALL: witness #{i} at block {} outside lookback window [{}, {}]",
            w.block_number,
            window_floor,
            l1_origin_block_number,
        );
        if w.is_reverted {
            ensure!(
                w.gas_used == 0 && w.return_data.is_empty(),
                "L1STATICCALL: witness #{i} reverted with non-canonical gas/data",
            );
        }
        set_l1_staticcall_value(
            w.target_address,
            w.block_number,
            &w.calldata,
            w.gas_used,
            w.return_data.to_vec(),
            w.is_reverted,
        )
        .map_err(|e| anyhow!("L1STATICCALL #{i}: test cache write rejected: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l1_precompiles::{L1ExecutionWitness, L1StaticCallWitness};
    use alethia_reth_evm::precompiles::l1sload::{clear_l1_storage, set_l1_origin_block_id};
    use alethia_reth_evm::precompiles::l1staticcall::{
        clear_l1_staticcall_cache, l1staticcall_run,
    };
    use alloy_primitives::{Address, B256, Bytes, U256};
    use serial_test::serial;

    // ───────────────────────────────────────────────
    // Helpers
    // ───────────────────────────────────────────────

    /// Test l1_origin that comfortably contains all test block numbers (50..=300) within the
    /// 256-block window. Window floor = 300 - 256 = 44.
    const TEST_L1_ORIGIN: u64 = 300;

    /// Wrapper that passes the shared `TEST_L1_ORIGIN` so individual tests stay focused on
    /// witness-body semantics rather than range-check boilerplate.
    fn verify_test(
        witnesses: &[L1StaticCallWitness],
        state_root_map: &HashMap<u64, B256>,
    ) -> Result<()> {
        verify_and_populate_l1_staticcall_witnesses(witnesses, state_root_map, TEST_L1_ORIGIN)
    }

    /// Test-only wrapper that populates the cache directly from witnesses, skipping the revm
    /// re-execution path. Used by tests that exercise cache-population behavior (dedup,
    /// calldata-key uniqueness, cache lookup) without needing real MPT fixtures.
    fn populate_test_cache(witnesses: &[L1StaticCallWitness]) -> Result<()> {
        populate_cache_skipping_revm_for_tests(witnesses, TEST_L1_ORIGIN)
    }

    fn make_witness(
        target: Address,
        block: u64,
        calldata: &[u8],
        return_data: &[u8],
    ) -> L1StaticCallWitness {
        L1StaticCallWitness {
            target_address: target,
            block_number: block,
            calldata: Bytes::from(calldata.to_vec()),
            return_data: Bytes::from(return_data.to_vec()),
            gas_used: 0,
            is_reverted: false,
            execution_witness: L1ExecutionWitness::default(),
        }
    }

    /// Reset all shared global state (l1origin, l1sload cache, l1staticcall cache).
    fn reset_all() {
        clear_l1_storage();
        clear_l1_staticcall_cache();
    }

    // ───────────────────────────────────────────────
    // Empty-witness fast-path tests
    // ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_verify_empty_witnesses_succeeds() {
        reset_all();
        let state_root_map: HashMap<u64, B256> = HashMap::new();
        let result = verify_test(&[], &state_root_map);
        assert!(result.is_ok(), "Empty witness list should return Ok");
    }

    #[test]
    #[serial]
    fn test_verify_missing_state_root_fails() {
        reset_all();
        let target = Address::from([0xAAu8; 20]);
        let witness = make_witness(target, 50, &[0x01], &[0xFF]);

        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, B256::from([0x11u8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(
            result.is_err(),
            "Should fail when witness references block not in state_root_map"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("no verified state root for block 50"),
            "Error should mention missing state root, got: {err_msg}"
        );
    }

    #[test]
    #[serial]
    fn test_verify_single_witness_succeeds() {
        reset_all();
        let target = Address::from([0xBBu8; 20]);
        let calldata = vec![0x01, 0x02, 0x03];
        let return_data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let witness = make_witness(target, 100, &calldata, &return_data);

        let _state_root_map: HashMap<u64, B256> = HashMap::from([(100, B256::from([0x22u8; 32]))]);

        let result = populate_test_cache(&[witness]);
        assert!(
            result.is_ok(),
            "Single valid witness should succeed: {:?}",
            result.err()
        );

        set_l1_origin_block_id(110);

        let mut input = Vec::with_capacity(52 + calldata.len());
        input.extend_from_slice(target.as_slice());
        input.extend_from_slice(&U256::from(100u64).to_be_bytes::<32>());
        input.extend_from_slice(&calldata);

        let precompile_result = l1staticcall_run(&input, 100_000, 0);
        assert!(
            precompile_result.is_ok(),
            "Cached value should be retrievable via precompile: {:?}",
            precompile_result.err()
        );
        assert_eq!(precompile_result.unwrap().bytes.as_ref(), &return_data);
    }

    #[test]
    #[serial]
    fn test_verify_reverted_witness_skips_revm_and_caches_revert() {
        reset_all();
        let target = Address::from([0xBCu8; 20]);
        let calldata = vec![0xAA, 0xBB];
        let witness = L1StaticCallWitness {
            target_address: target,
            block_number: 100,
            calldata: Bytes::from(calldata.clone()),
            return_data: Bytes::from(vec![]),
            gas_used: 0,
            is_reverted: true,
            execution_witness: L1ExecutionWitness {
                state: vec![Bytes::from(vec![0xFFu8; 8])],
                codes: vec![],
                keys: vec![],
                headers: vec![],
            },
        };

        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, B256::from([0x22u8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(
            result.is_ok(),
            "reverted witness should bypass revm build: {:?}",
            result.err()
        );

        set_l1_origin_block_id(110);

        let mut input = Vec::with_capacity(52 + calldata.len());
        input.extend_from_slice(target.as_slice());
        input.extend_from_slice(&U256::from(100u64).to_be_bytes::<32>());
        input.extend_from_slice(&calldata);

        let precompile_result = l1staticcall_run(&input, 100_000, 0);
        assert!(
            matches!(&precompile_result, Ok(o) if o.is_halt()),
            "cached reverted call should halt: {precompile_result:?}",
        );
    }

    #[test]
    #[serial]
    fn test_verify_multiple_witnesses_succeeds() {
        reset_all();
        let target_a = Address::from([0xAAu8; 20]);
        let target_b = Address::from([0xBBu8; 20]);

        let witness_a = make_witness(target_a, 100, &[0x01], &[0x11, 0x22]);
        let witness_b = make_witness(target_b, 101, &[0x02], &[0x33, 0x44]);
        let witness_c = make_witness(target_a, 102, &[0x03], &[0x55]);

        let _state_root_map: HashMap<u64, B256> = HashMap::from([
            (100, B256::from([0x01u8; 32])),
            (101, B256::from([0x02u8; 32])),
            (102, B256::from([0x03u8; 32])),
        ]);

        let result = populate_test_cache(&[witness_a, witness_b, witness_c]);
        assert!(
            result.is_ok(),
            "Multiple valid witnesses should all succeed: {:?}",
            result.err()
        );

        set_l1_origin_block_id(110);

        let mut input_a = Vec::with_capacity(53);
        input_a.extend_from_slice(target_a.as_slice());
        input_a.extend_from_slice(&U256::from(100u64).to_be_bytes::<32>());
        input_a.push(0x01);
        let res_a = l1staticcall_run(&input_a, 100_000, 0);
        assert!(
            res_a.is_ok(),
            "witness_a should be cached: {:?}",
            res_a.err()
        );
        assert_eq!(res_a.unwrap().bytes.as_ref(), &[0x11, 0x22]);

        let mut input_b = Vec::with_capacity(53);
        input_b.extend_from_slice(target_b.as_slice());
        input_b.extend_from_slice(&U256::from(101u64).to_be_bytes::<32>());
        input_b.push(0x02);
        let res_b = l1staticcall_run(&input_b, 100_000, 0);
        assert!(
            res_b.is_ok(),
            "witness_b should be cached: {:?}",
            res_b.err()
        );
        assert_eq!(res_b.unwrap().bytes.as_ref(), &[0x33, 0x44]);

        let mut input_c = Vec::with_capacity(53);
        input_c.extend_from_slice(target_a.as_slice());
        input_c.extend_from_slice(&U256::from(102u64).to_be_bytes::<32>());
        input_c.push(0x03);
        let res_c = l1staticcall_run(&input_c, 100_000, 0);
        assert!(
            res_c.is_ok(),
            "witness_c should be cached: {:?}",
            res_c.err()
        );
        assert_eq!(res_c.unwrap().bytes.as_ref(), &[0x55]);
    }

    #[test]
    #[serial]
    fn test_verify_populates_cache_correctly() {
        reset_all();
        let target = Address::from([0xCCu8; 20]);
        let calldata = vec![0xCA, 0xFE, 0xBA, 0xBE];
        let return_data = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        let witness = make_witness(target, 200, &calldata, &return_data);

        let _state_root_map: HashMap<u64, B256> = HashMap::from([(200, B256::from([0x44u8; 32]))]);

        let result = populate_test_cache(&[witness]);
        assert!(
            result.is_ok(),
            "Verification should succeed: {:?}",
            result.err()
        );

        set_l1_origin_block_id(210);

        let mut input = Vec::with_capacity(52 + calldata.len());
        input.extend_from_slice(target.as_slice());
        input.extend_from_slice(&U256::from(200u64).to_be_bytes::<32>());
        input.extend_from_slice(&calldata);

        let precompile_result = l1staticcall_run(&input, 100_000, 0);
        assert!(
            precompile_result.is_ok(),
            "Cache should be populated after verify_and_populate: {:?}",
            precompile_result.err()
        );
        assert_eq!(precompile_result.unwrap().bytes.as_ref(), &return_data);
    }

    #[test]
    #[serial]
    fn test_verify_different_calldata_same_target() {
        reset_all();
        let target = Address::from([0xDDu8; 20]);
        let calldata_1 = vec![0x01, 0x02];
        let calldata_2 = vec![0x03, 0x04, 0x05];
        let return_data_1 = vec![0xAA];
        let return_data_2 = vec![0xBB, 0xCC];

        let witness_1 = make_witness(target, 100, &calldata_1, &return_data_1);
        let witness_2 = make_witness(target, 100, &calldata_2, &return_data_2);

        let _state_root_map: HashMap<u64, B256> = HashMap::from([(100, B256::from([0x55u8; 32]))]);

        let result = populate_test_cache(&[witness_1, witness_2]);
        assert!(
            result.is_ok(),
            "Two witnesses for same target should succeed: {:?}",
            result.err()
        );

        set_l1_origin_block_id(110);

        let mut input_1 = Vec::with_capacity(52 + calldata_1.len());
        input_1.extend_from_slice(target.as_slice());
        input_1.extend_from_slice(&U256::from(100u64).to_be_bytes::<32>());
        input_1.extend_from_slice(&calldata_1);
        let res_1 = l1staticcall_run(&input_1, 100_000, 0);
        assert!(res_1.is_ok(), "First calldata should hit cache");
        assert_eq!(res_1.unwrap().bytes.as_ref(), &return_data_1);

        let mut input_2 = Vec::with_capacity(52 + calldata_2.len());
        input_2.extend_from_slice(target.as_slice());
        input_2.extend_from_slice(&U256::from(100u64).to_be_bytes::<32>());
        input_2.extend_from_slice(&calldata_2);
        let res_2 = l1staticcall_run(&input_2, 100_000, 0);
        assert!(res_2.is_ok(), "Second calldata should hit cache");
        assert_eq!(res_2.unwrap().bytes.as_ref(), &return_data_2);
    }

    // ───────────────────────────────────────────────
    // Non-empty-witness path (revm re-execution failure modes)
    // ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_verify_rejects_malformed_witness_state() {
        reset_all();
        let target = Address::from([0xE1u8; 20]);
        let witness = L1StaticCallWitness {
            target_address: target,
            block_number: 100,
            calldata: Bytes::from(vec![0x01]),
            return_data: Bytes::from(vec![0x02]),
            gas_used: 0,
            is_reverted: false,
            execution_witness: L1ExecutionWitness {
                // 0xFF bytes cannot decode as a valid MPT node.
                state: vec![Bytes::from(vec![0xFFu8; 8])],
                codes: vec![],
                keys: vec![],
                headers: vec![],
            },
        };

        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, B256::from([0x11u8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(
            result.is_err(),
            "Malformed witness state should be rejected"
        );
        let err = result.unwrap_err().to_string();
        // surge's eager MptNode parser surfaces the failure at `WitnessDb::build`.
        // raiko2's `SparseState` walks the trie lazily, so the failure surfaces during
        // the revm transact step as a TrieWitnessError. Either path proves the malformed
        // witness state is rejected before being used as truth.
        assert!(
            err.contains("WitnessDb build")
                || err.contains("TrieWitnessError")
                || err.contains("unresolved"),
            "Error should surface a witness-rejection failure, got: {err}"
        );
    }

    #[test]
    #[serial]
    fn test_verify_rejects_mismatched_state_root() {
        reset_all();
        let target = Address::from([0xE2u8; 20]);
        let witness = L1StaticCallWitness {
            target_address: target,
            block_number: 200,
            calldata: Bytes::from(vec![0x01]),
            return_data: Bytes::from(vec![0x02]),
            gas_used: 0,
            is_reverted: false,
            execution_witness: L1ExecutionWitness {
                // 0x80 is RLP for an empty string — valid RLP but its keccak won't match.
                state: vec![Bytes::from(vec![0x80u8])],
                codes: vec![],
                keys: vec![],
                headers: vec![],
            },
        };

        // Trusted root that the witness definitively does not produce.
        let state_root_map: HashMap<u64, B256> = HashMap::from([(200, B256::from([0xAAu8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(
            result.is_err(),
            "Witness whose trie doesn't hash to the trusted state_root should be rejected"
        );
    }

    // ───────────────────────────────────────────────
    // Range + revert-semantics guards
    // ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_verify_rejects_witness_below_window_floor() {
        reset_all();
        let target = Address::from([0xF1u8; 20]);
        let floor = TEST_L1_ORIGIN.saturating_sub(L1STATICCALL_MAX_BLOCK_LOOKBACK);
        let outside = floor.saturating_sub(1);
        let witness = make_witness(target, outside, &[0x01], &[0x02]);
        let state_root_map: HashMap<u64, B256> =
            HashMap::from([(outside, B256::from([0x11u8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("outside lookback window"),
            "expected window-violation error, got: {err}"
        );
    }

    #[test]
    #[serial]
    fn test_verify_rejects_witness_above_l1_origin() {
        reset_all();
        let target = Address::from([0xF2u8; 20]);
        let witness = make_witness(target, TEST_L1_ORIGIN + 1, &[0x01], &[0x02]);
        let state_root_map: HashMap<u64, B256> =
            HashMap::from([(TEST_L1_ORIGIN + 1, B256::from([0x11u8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outside lookback window")
        );
    }

    #[test]
    #[serial]
    fn test_verify_rejects_reverted_with_nonzero_gas() {
        reset_all();
        let target = Address::from([0xF3u8; 20]);
        let witness = L1StaticCallWitness {
            target_address: target,
            block_number: 100,
            calldata: Bytes::from(vec![0x01]),
            return_data: Bytes::from(vec![]),
            gas_used: 12_345,
            is_reverted: true,
            execution_witness: L1ExecutionWitness::default(),
        };
        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, B256::from([0x22u8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-zero gas"));
    }

    #[test]
    #[serial]
    fn test_verify_rejects_reverted_with_nonempty_data() {
        reset_all();
        let target = Address::from([0xF4u8; 20]);
        let witness = L1StaticCallWitness {
            target_address: target,
            block_number: 100,
            calldata: Bytes::from(vec![0x01]),
            return_data: Bytes::from(vec![0xAA]),
            gas_used: 0,
            is_reverted: true,
            execution_witness: L1ExecutionWitness::default(),
        };
        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, B256::from([0x22u8; 32]))]);

        let result = verify_test(&[witness], &state_root_map);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-empty data"));
    }

    #[test]
    #[serial]
    fn test_verify_reverted_witness_without_state_root_succeeds() {
        // Reverted witnesses must NOT require an entry in state_root_map for their block —
        // they don't re-execute against L1 state, so the dependency is misleading.
        reset_all();
        let target = Address::from([0xBCu8; 20]);
        let calldata = vec![0xAA, 0xBB];
        let witness = L1StaticCallWitness {
            target_address: target,
            block_number: 100,
            calldata: Bytes::from(calldata),
            return_data: Bytes::from(vec![]),
            gas_used: 0,
            is_reverted: true,
            execution_witness: L1ExecutionWitness::default(),
        };
        let state_root_map: HashMap<u64, B256> = HashMap::new();

        let result = verify_test(&[witness], &state_root_map);
        assert!(
            result.is_ok(),
            "reverted witness must succeed without a state-root entry: {:?}",
            result.err()
        );
    }

    #[test]
    #[serial]
    fn test_verify_dedups_identical_witnesses() {
        reset_all();
        let target = Address::from([0xDEu8; 20]);
        let calldata = vec![0x12, 0x34];
        let return_data = vec![0x77, 0x88, 0x99];
        let witness = make_witness(target, 150, &calldata, &return_data);

        let result = populate_test_cache(&[witness.clone(), witness.clone(), witness]);
        assert!(
            result.is_ok(),
            "duplicate witnesses must not error: {:?}",
            result.err()
        );

        set_l1_origin_block_id(160);

        let mut input = Vec::with_capacity(52 + calldata.len());
        input.extend_from_slice(target.as_slice());
        input.extend_from_slice(&U256::from(150u64).to_be_bytes::<32>());
        input.extend_from_slice(&calldata);

        let res = l1staticcall_run(&input, 100_000, 0);
        assert!(
            res.is_ok(),
            "deduplicated cache must still serve the value: {:?}",
            res.err()
        );
        assert_eq!(res.unwrap().bytes.as_ref(), &return_data);
    }

    #[test]
    #[serial]
    fn test_l1staticcall_gas_cap_const_matches_evm_limit() {
        assert_eq!(
            L1STATICCALL_GAS_CAP, 30_000_000,
            "L1STATICCALL_GAS_CAP changed — sequencer/witness/guest must move together"
        );
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // Revm-re-execution path tests
    // ───────────────────────────────────────────────────────────────────────────────────────
    //
    // The tests above all hit the cfg(test) empty-witness fast path. The tests below build
    // real MPT tries via `risc0_ethereum_trie::CachedTrie` (the same primitive raiko2's
    // SparseState materializes from at runtime), then drive `verify_and_populate_*` end-to-end
    // — exercising the full revm transact + witness-trie walk path that surge-raiko's #40
    // regression class exposed. They guard against:
    //   * block-env perturbation drifting SLOAD outputs (R7),
    //   * the `return_data` mismatch diagnostic going stale,
    //   * the WitnessDb account-miss path being misused by a malicious prover,
    //   * witness header binding against the trusted L1 chain (H1).

    use alloy_consensus::Header;
    use alloy_consensus::TrieAccount;
    use alloy_primitives::keccak256;
    use alloy_rlp::Encodable;
    use risc0_ethereum_trie::CachedTrie;

    /// Builds a one-account state trie + storage trie for an SLOAD target.
    ///
    /// Returns `(witness, state_root, expected_return)` for an L1STATICCALL on a contract
    /// whose runtime code is `SLOAD(slot 0); RETURN 32 bytes` — the simplest non-trivial
    /// witness shape (one account, one storage slot, one bytecode).
    fn sload_target_witness(
        target: Address,
        block_number: u64,
        stored_value: U256,
    ) -> (L1StaticCallWitness, B256, Vec<u8>) {
        // PUSH1 0 SLOAD PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN — 11 bytes.
        let code: Vec<u8> = vec![
            0x60, 0x00, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
        ];
        let code_hash = keccak256(&code);

        // Storage trie: slot 0 → stored_value (RLP-encoded U256).
        let mut storage_trie = CachedTrie::default();
        let storage_key = keccak256(U256::ZERO.to_be_bytes::<32>());
        let mut storage_value_rlp = Vec::new();
        stored_value.encode(&mut storage_value_rlp);
        storage_trie.insert(storage_key, storage_value_rlp);
        let storage_root = storage_trie.hash();

        // State trie: target → TrieAccount(nonce=0, balance=0, storage_root, code_hash).
        let account = TrieAccount {
            nonce: 0,
            balance: U256::ZERO,
            storage_root,
            code_hash,
        };
        let mut state_trie = CachedTrie::default();
        let address_key = keccak256(target);
        let mut account_rlp = Vec::new();
        account.encode(&mut account_rlp);
        state_trie.insert(address_key, account_rlp);
        let state_root = state_trie.hash();

        let expected_return = stored_value.to_be_bytes::<32>().to_vec();

        // Build the witness with the materialized trie nodes — SparseState walks these to
        // resolve the account + storage lookups during revm re-execution.
        let mut state_nodes = state_trie.rlp_nodes();
        state_nodes.extend(storage_trie.rlp_nodes());

        let witness = L1StaticCallWitness {
            target_address: target,
            block_number,
            calldata: Bytes::new(),
            return_data: Bytes::from(expected_return.clone()),
            // Placeholder — the fixture relies on the verifier surfacing a `gas_used`
            // mismatch to prove revm actually re-executed (vs the empty-witness fast path
            // which would silently succeed against any claimed gas value).
            gas_used: 0,
            is_reverted: false,
            execution_witness: L1ExecutionWitness {
                state: state_nodes,
                codes: vec![Bytes::from(code)],
                keys: vec![
                    Bytes::from(target.as_slice().to_vec()),
                    Bytes::from(U256::ZERO.to_be_bytes::<32>().to_vec()),
                ],
                headers: vec![],
            },
        };
        (witness, state_root, expected_return)
    }

    /// Adds an RLP-encoded ancestor header to an existing SLOAD-target witness for the H1
    /// witness-header-chain-binding tests.
    fn with_extra_header(mut witness: L1StaticCallWitness, header: &Header) -> L1StaticCallWitness {
        let mut hdr_bytes = Vec::new();
        header.encode(&mut hdr_bytes);
        witness.execution_witness.headers = vec![Bytes::from(hdr_bytes)];
        witness
    }

    #[test]
    #[serial]
    fn test_revm_reexecution_produces_correct_return_data_for_sload() {
        // The whole point of this regression guard: revm MUST produce the correct 32-byte
        // SLOAD output. Only gas_used should mismatch (placeholder=0 vs revm's real number).
        reset_all();
        let target = Address::from([0xABu8; 20]);
        let (witness, state_root, _) =
            sload_target_witness(target, 100, U256::from(0xDEAD_BEEFu64));
        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, state_root)]);

        let err = verify_test(&[witness], &state_root_map)
            .expect_err("gas_used=0 in fixture must fail revm's gas check");
        let msg = err.to_string();

        assert!(
            msg.contains("gas_used mismatch"),
            "Expected gas_used mismatch (proves revm re-executed the SLOAD correctly). \
             Got instead: {msg}"
        );
        assert!(
            !msg.contains("return_data mismatch"),
            "return_data mismatch means revm's re-execution drifted from the witness \
             output — exactly the class of failure R7/#40 surfaced. Got: {msg}"
        );
    }

    #[test]
    #[serial]
    fn test_revm_reexecution_ignores_populated_block_env_for_storage_only_target() {
        // Regression for R7: a target that only reads storage must produce byte-identical
        // output whether or not timestamp / beneficiary / prevrandao are populated in the
        // block env. If revm's block-env threading regresses, a pure-SLOAD call would drift
        // and this test fails loudly.
        reset_all();
        let target = Address::from([0xCDu8; 20]);
        let (witness, state_root, _) = sload_target_witness(target, 150, U256::from(42u64));
        let state_root_map: HashMap<u64, B256> = HashMap::from([(150, state_root)]);

        let header = Header {
            number: 150,
            timestamp: 1_776_933_683,
            beneficiary: Address::from([0xEEu8; 20]),
            mix_hash: B256::from([0x11u8; 32]),
            base_fee_per_gas: Some(1_000_000_000),
            excess_blob_gas: Some(0),
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let header_map: HashMap<u64, &Header> = HashMap::from([(150, &header)]);

        let err = verify_and_populate_l1_staticcall_witnesses_with_headers(
            &[witness],
            &state_root_map,
            Some(&header_map),
            TEST_L1_ORIGIN,
        )
        .expect_err("gas_used=0 placeholder must fail the gas assertion");

        let msg = err.to_string();
        assert!(
            msg.contains("gas_used mismatch"),
            "Populated block env must not disturb SLOAD output — expected the only \
             mismatch to be gas_used. Got: {msg}"
        );
        assert!(
            !msg.contains("return_data mismatch"),
            "return_data differed with populated block env (R7 regression class). \
             The full mismatch details are in the error message — read them: {msg}"
        );
    }

    #[test]
    #[serial]
    fn test_revm_reexecution_surfaces_return_data_mismatch_with_diagnostics() {
        // Guarantees the improved `return_data mismatch` error carries both revm's output
        // and the witness's expected value, so a future regression is debuggable from the
        // log alone (no rebuild-with-extra-prints cycle).
        reset_all();
        let target = Address::from([0xEFu8; 20]);
        let (mut witness, state_root, _) = sload_target_witness(target, 200, U256::from(0xBEEFu64));
        // Corrupt the expected return_data so the verifier surfaces a return_data mismatch.
        witness.return_data = Bytes::from(vec![0xFFu8; 32]);
        let state_root_map: HashMap<u64, B256> = HashMap::from([(200, state_root)]);

        let err =
            verify_test(&[witness], &state_root_map).expect_err("corrupted return_data must fail");
        let msg = err.to_string();

        assert!(msg.contains("return_data mismatch"), "Got: {msg}");
        assert!(
            msg.contains("revm returned"),
            "Missing revm output. Got: {msg}"
        );
        assert!(
            msg.contains("witness expects"),
            "Missing witness value. Got: {msg}"
        );
        assert!(
            msg.contains("target="),
            "Missing target address. Got: {msg}"
        );
        assert!(
            msg.contains("block=200"),
            "Missing block number. Got: {msg}"
        );
        // The revm output should be the actual SLOAD'd value as 32 bytes.
        assert!(
            msg.contains(&format!("{:064x}", 0xBEEFu64)),
            "Expected revm output hex to contain the stored value. Got: {msg}"
        );
    }

    #[test]
    #[serial]
    fn test_verify_rejects_witness_header_outside_trusted_chain() {
        // H1 defense-in-depth: a witness header for a block NOT in the trusted `header_map`
        // must be rejected before WitnessDb sees it. A malicious prover could otherwise
        // fabricate header bytes for a block they never visited and lie about its hash.
        reset_all();
        let target = Address::from([0xF5u8; 20]);
        let rogue_header = Header {
            number: 50,
            timestamp: 1_000,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let (mut witness, state_root, _) = sload_target_witness(target, 100, U256::from(42u64));
        witness = with_extra_header(witness, &rogue_header);
        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, state_root)]);
        // Trusted header map contains block 100 (the call's target) but NOT block 50.
        let trusted_header = Header {
            number: 100,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let header_map: HashMap<u64, &Header> = HashMap::from([(100, &trusted_header)]);

        let err = verify_and_populate_l1_staticcall_witnesses_with_headers(
            &[witness],
            &state_root_map,
            Some(&header_map),
            TEST_L1_ORIGIN,
        )
        .expect_err("witness header outside trusted chain must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("not in trusted L1 chain"),
            "expected trusted-chain rejection, got: {msg}"
        );
    }

    #[test]
    #[serial]
    fn test_verify_rejects_witness_header_hash_mismatch() {
        // H1: a witness header whose bytes hash differs from the trusted hash at the same
        // block number must be rejected — the prover cannot swap in a fake header for a
        // block we already know.
        reset_all();
        let target = Address::from([0xF6u8; 20]);
        let witness_header = Header {
            number: 99,
            timestamp: 0xAAA,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let (mut witness, state_root, _) = sload_target_witness(target, 100, U256::from(42u64));
        witness = with_extra_header(witness, &witness_header);
        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, state_root)]);
        // Trusted header for block 99 carries DIFFERENT bytes (different timestamp).
        let trusted_99 = Header {
            number: 99,
            timestamp: 0xBBB,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let trusted_100 = Header {
            number: 100,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let header_map: HashMap<u64, &Header> =
            HashMap::from([(99, &trusted_99), (100, &trusted_100)]);

        let err = verify_and_populate_l1_staticcall_witnesses_with_headers(
            &[witness],
            &state_root_map,
            Some(&header_map),
            TEST_L1_ORIGIN,
        )
        .expect_err("witness header hash mismatch must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("hash mismatch"),
            "expected hash-mismatch rejection, got: {msg}"
        );
    }

    #[test]
    #[serial]
    fn test_verify_accepts_witness_header_matching_trusted_chain() {
        // Inverse of the previous two: a witness header that matches the trusted chain
        // must pass binding. The 3-way assertion will still fail on the gas placeholder,
        // proving the binding check passed and the failure came later in the pipeline.
        reset_all();
        let target = Address::from([0xF7u8; 20]);
        let ancestor = Header {
            number: 99,
            timestamp: 0xCCC,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let (mut witness, state_root, _) = sload_target_witness(target, 100, U256::from(42u64));
        witness = with_extra_header(witness, &ancestor);
        let state_root_map: HashMap<u64, B256> = HashMap::from([(100, state_root)]);
        let trusted_100 = Header {
            number: 100,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let header_map: HashMap<u64, &Header> =
            HashMap::from([(99, &ancestor), (100, &trusted_100)]);

        let err = verify_and_populate_l1_staticcall_witnesses_with_headers(
            &[witness],
            &state_root_map,
            Some(&header_map),
            TEST_L1_ORIGIN,
        )
        .expect_err("gas_used=0 placeholder forces revm gas-check failure");
        let msg = err.to_string();
        assert!(
            !msg.contains("trusted L1 chain") && !msg.contains("hash mismatch"),
            "matching witness header should pass binding check, got: {msg}"
        );
        assert!(
            msg.contains("gas_used mismatch"),
            "binding passed but later check should hit gas_used. Got: {msg}"
        );
    }

    #[test]
    #[serial]
    fn test_verify_catches_missing_account_via_three_way_assertion() {
        // A witness whose state trie hides the called contract still hashes to a valid
        // state_root (the witness lies about an UNRELATED account being the only one in
        // state). revm sees the target as having no code, returns an empty success — but
        // the witness claims non-empty `return_data`. The 3-way assertion must catch the
        // divergence, even if the underlying account-lookup path silently returned None.
        reset_all();
        let unrelated = Address::from([0x99u8; 20]);
        let (other_witness, state_root, _) =
            sload_target_witness(unrelated, 250, U256::from(0xCAFEu64));

        let target = Address::from([0xAAu8; 20]); // different from `unrelated`
        let witness = L1StaticCallWitness {
            target_address: target,
            block_number: 250,
            calldata: Bytes::new(),
            // Lie: claim non-empty return_data even though revm will see an empty call.
            return_data: Bytes::from(vec![0xDEu8, 0xADu8, 0xBEu8, 0xEFu8]),
            gas_used: 50_000,
            is_reverted: false,
            execution_witness: other_witness.execution_witness,
        };

        let state_root_map: HashMap<u64, B256> = HashMap::from([(250, state_root)]);
        let err = verify_test(&[witness], &state_root_map)
            .expect_err("missing-account witness lie must be caught by 3-way assertion");
        let msg = err.to_string();
        assert!(
            msg.contains("return_data mismatch"),
            "expected return_data mismatch (revm returned empty, witness claims non-empty). \
             Got: {msg}"
        );
        assert!(
            msg.contains("target="),
            "error should name the diverging target address, got: {msg}"
        );
    }
}
