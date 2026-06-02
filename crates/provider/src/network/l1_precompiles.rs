//! L1 precompile preflight fetchers: EIP-1186 storage proofs (for L1SLOAD) and `proof_call`
//! execution witnesses (for L1STATICCALL). `eth_getProof` is universal across L1 ELs;
//! `proof_call` is currently served only by Nethermind ([nethermind#11732]).
//!
//! [nethermind#11732]: https://github.com/NethermindEth/nethermind/pull/11732

use std::collections::HashMap;

use alloy::{eips::BlockId, providers::Provider as AlloyProvider};
use alloy_primitives::{Address, B256, Bytes, U256};
use futures::{StreamExt, TryStreamExt, stream};
use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_primitives_shasta::l1_precompiles::{
    L1_PRECOMPILE_CALLER, L1ExecutionWitness, L1STATICCALL_GAS_CAP, L1StaticCallRecord,
    L1StaticCallWitness, L1StorageProof, set_l1_rpc_fetcher, set_l1_staticcall_rpc_fetcher,
};
use serde::Deserialize;

use super::NetworkProvider;

const L1_PRECOMPILE_CONCURRENCY: usize = 16;

/// Subset of `debug_traceCall`'s response consumed by the L1Staticcall discovery fetcher.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TraceCallResult {
    gas: u64,
    return_value: String,
    failed: bool,
}

impl NetworkProvider {
    /// Install the live L1 fetchers (`eth_getStorageAt` for L1Sload, `debug_traceCall` for
    /// L1Staticcall) into the precompile globals, so the host discovery re-execution can fetch +
    /// record the L1 reads each block makes. The sync precompile-fetcher contract is bridged to
    /// the async L1 client via `block_in_place` + `Handle::block_on`, which requires a multi-thread
    /// tokio runtime worker (the discovery loop must run on one).
    pub(crate) fn install_live_l1_fetchers(&self) {
        let handle = tokio::runtime::Handle::current();

        let sload_provider = self.l1_provider.clone();
        let sload_handle = handle.clone();
        set_l1_rpc_fetcher(move |contract, slot, block_n| {
            let provider = sload_provider.clone();
            let handle = sload_handle.clone();
            tokio::task::block_in_place(move || {
                handle.block_on(async move {
                    let value: U256 = provider
                        .get_storage_at(contract, U256::from_be_bytes(slot.0))
                        .block_id(BlockId::number(block_n))
                        .await
                        .map_err(|e| format!("eth_getStorageAt failed: {e}"))?;
                    Ok(B256::from(value.to_be_bytes::<32>()))
                })
            })
        });

        let call_client = self.l1_client.clone();
        set_l1_staticcall_rpc_fetcher(move |target, block_n, gas_limit, calldata| {
            let client = call_client.clone();
            let handle = handle.clone();
            let calldata = calldata.to_vec();
            tokio::task::block_in_place(move || {
                handle.block_on(async move {
                    let call = serde_json::json!({
                        "from": format!("{L1_PRECOMPILE_CALLER:?}"),
                        "to": format!("{target:?}"),
                        "data": format!("0x{}", hex::encode(&calldata)),
                        "gas": format!("0x{:x}", gas_limit.min(L1STATICCALL_GAS_CAP)),
                    });
                    let block_hex = format!("0x{block_n:x}");
                    // Slim the struct-logger payload — we only read gas/returnValue/failed.
                    let tracer = serde_json::json!({
                        "disableStorage": true,
                        "disableStack": true,
                        "disableMemory": true,
                    });
                    let resp: TraceCallResult = client
                        .request("debug_traceCall", (call, block_hex, tracer))
                        .await
                        .map_err(|e| format!("debug_traceCall failed: {e}"))?;
                    if resp.failed {
                        // NMC `GethLikeTxTracer.MarkAsFailed` contract: reverted calls report
                        // gas=0 and empty data. The guest verifier rejects any reverted record
                        // with non-zero gas or non-empty data, so normalize here regardless of
                        // what the upstream L1 EL chose to report.
                        return Ok((0, Vec::new(), true));
                    }
                    let hex_str = resp
                        .return_value
                        .strip_prefix("0x")
                        .unwrap_or(&resp.return_value);
                    let bytes =
                        hex::decode(hex_str).map_err(|e| format!("decode returnValue: {e}"))?;
                    Ok((resp.gas.min(gas_limit), bytes, false))
                })
            })
        });
    }
}

/// Subset of `proof_call`'s `CallResultWithProof` response (NMC PR #11732 shape) we consume.
/// `witness` carries the re-execution package; a non-null `error` means the call failed in-VM —
/// the witness alone wouldn't let the guest reproduce the sequencer's recorded result, so reject.
#[derive(Debug, Deserialize)]
struct ProofCallResponse {
    #[serde(default)]
    error: Option<ProofCallError>,
    witness: L1ExecutionWitness,
}

/// In-VM error envelope from `proof_call` — matches NMC's `CallErrorEnvelope` shape (codes
/// `ExecutionReverted = 3`, `ExecutionError = -32003`). Typed so the surfaced error message
/// carries the structured code/message instead of an opaque JSON blob.
#[derive(Debug, Deserialize)]
struct ProofCallError {
    code: i32,
    message: String,
    #[serde(default)]
    data: Option<String>,
}

impl NetworkProvider {
    /// Fetch EIP-1186 storage proofs for each `(block, contract, slots)` request via `eth_getProof`,
    /// flattened to one [`L1StorageProof`] per `(contract, slot, block)`.
    pub(crate) async fn fetch_l1_storage_proofs(
        &self,
        requests: &[(u64, Address, Vec<B256>)],
    ) -> RaikoResult<Vec<L1StorageProof>> {
        let per_request: Vec<Vec<L1StorageProof>> = stream::iter(requests.iter().cloned())
            .map(|(block, contract, slots)| async move {
                let proof = self
                    .l1_provider
                    .get_proof(contract, slots.clone())
                    .block_id(BlockId::number(block))
                    .await
                    .map_err(|e| {
                        RaikoError::RPC(format!(
                            "eth_getProof failed (contract={contract:?}, block={block}): {e}"
                        ))
                    })?;
                let block_b256 = B256::from(U256::from(block));
                slots
                    .iter()
                    .map(|slot| {
                        let sp = proof
                            .storage_proof
                            .iter()
                            .find(|p| p.key.as_b256() == *slot)
                            .ok_or_else(|| {
                                RaikoError::RPC(format!(
                                    "missing storage proof for slot {slot:?} \
                                     (contract={contract:?}, block={block})"
                                ))
                            })?;
                        Ok(L1StorageProof {
                            contract_address: contract,
                            storage_key: *slot,
                            block_number: block_b256,
                            value: B256::from(sp.value),
                            account_proof: proof.account_proof.clone(),
                            storage_proof: sp.proof.clone(),
                        })
                    })
                    .collect::<RaikoResult<Vec<_>>>()
            })
            .buffered(L1_PRECOMPILE_CONCURRENCY)
            .try_collect()
            .await?;
        Ok(per_request.into_iter().flatten().collect())
    }

    /// Fetch L1STATICCALL execution witnesses via `proof_call`. The discovery pass already captured
    /// each call's `(target, block, calldata, return_data, gas_used, is_reverted)` from
    /// `debug_traceCall`; `proof_call` only supplies the `execution_witness`. Distinct
    /// `(target, block, calldata)` lookups are deduplicated, then replicated back across the
    /// original record sequence.
    ///
    /// Reverted records are skipped — the guest verifier doesn't need a witness for them (it
    /// only enforces NMC's `gas == 0 && empty-data` contract), and `proof_call` on a reverted
    /// call returns a non-null `error` which `fetch_proof_call_witness` would surface as an RPC
    /// failure. Reverted records get a default empty witness instead.
    pub(crate) async fn fetch_l1_staticcall_witnesses(
        &self,
        records: &[L1StaticCallRecord],
    ) -> RaikoResult<Vec<L1StaticCallWitness>> {
        let mut unique: Vec<(Address, u64, Vec<u8>)> = Vec::new();
        let mut index_of: HashMap<(Address, u64, Vec<u8>), usize> = HashMap::new();
        for r in records {
            if r.is_reverted {
                continue;
            }
            let key = (r.target, r.block_number, r.calldata.clone());
            index_of.entry(key.clone()).or_insert_with(|| {
                unique.push(key);
                unique.len() - 1
            });
        }

        let witnesses: Vec<L1ExecutionWitness> = stream::iter(unique.iter().cloned())
            .map(|(target, block, calldata)| async move {
                self.fetch_proof_call_witness(target, block, &calldata)
                    .await
            })
            .buffered(L1_PRECOMPILE_CONCURRENCY)
            .try_collect()
            .await?;

        Ok(records
            .iter()
            .map(|r| L1StaticCallWitness {
                target_address: r.target,
                block_number: r.block_number,
                calldata: Bytes::from(r.calldata.clone()),
                return_data: Bytes::from(r.return_data.clone()),
                gas_used: r.gas_used,
                is_reverted: r.is_reverted,
                execution_witness: if r.is_reverted {
                    L1ExecutionWitness::default()
                } else {
                    witnesses[index_of[&(r.target, r.block_number, r.calldata.clone())]].clone()
                },
            })
            .collect())
    }

    /// Issue a single `proof_call` against the L1 EL and return its execution witness.
    async fn fetch_proof_call_witness(
        &self,
        target: Address,
        block: u64,
        calldata: &[u8],
    ) -> RaikoResult<L1ExecutionWitness> {
        // `proof_call` runs at the full L1STATICCALL_GAS_CAP — the same budget the guest
        // re-executes under — so the witness covers the same path. For a callee that branches on
        // `gasleft()` this can differ from discovery's `debug_traceCall` budget (min(frame, cap));
        // aligning all three is a devnet follow-up (code-review-2026-06-01 R3). Non-gas-sensitive
        // callees are unaffected.
        let call = serde_json::json!({
            "from": format!("{L1_PRECOMPILE_CALLER:?}"),
            "to": format!("{target:?}"),
            "data": format!("0x{}", hex::encode(calldata)),
            "gas": format!("0x{:x}", L1STATICCALL_GAS_CAP),
        });
        let block_hex = format!("0x{block:x}");
        let resp: ProofCallResponse = self
            .l1_client
            .request("proof_call", (call, block_hex))
            .await
            .map_err(|e| {
                RaikoError::RPC(format!(
                    "proof_call failed (target={target:?}, block={block}): {e}"
                ))
            })?;
        if let Some(error) = resp.error {
            return Err(RaikoError::RPC(format!(
                "proof_call reported in-VM error for target={target:?} block={block} \
                 (code={}, message={}, data={:?})",
                error.code, error.message, error.data,
            )));
        }
        Ok(resp.witness)
    }
}
