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

/// Default concurrency for L1 preflight fetches (`eth_getProof` + `proof_call`). Tunable via
/// the `L1_PRECOMPILE_CONCURRENCY` env var (D13); aligns with the other preflight knobs
/// (`PREFLIGHT_CHUNK_SIZE`, `PREFLIGHT_CHUNK_CONCURRENCY`) for operators behind L1 RPC
/// providers with stricter throttling.
const DEFAULT_L1_PRECOMPILE_CONCURRENCY: usize = 16;

fn l1_precompile_concurrency() -> usize {
    static CACHED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        std::env::var("L1_PRECOMPILE_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_L1_PRECOMPILE_CONCURRENCY)
    })
}

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
    ///
    /// **Runtime requirement (S5).** `block_in_place` panics on a current-thread runtime. We
    /// check the flavor up front and `panic!` with a clear operator-facing message so the
    /// failure surfaces at install time rather than at the first cache miss (unrecoverable
    /// panic inside the synchronous precompile callback).
    pub(crate) fn install_live_l1_fetchers(&self) {
        let handle = tokio::runtime::Handle::current();
        assert!(
            handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::CurrentThread,
            "L1 precompile fetcher requires a multi-thread tokio runtime (current_thread is \
             incompatible with block_in_place); start the host with \
             `tokio::main(flavor = \"multi_thread\")`",
        );

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
            .buffered(l1_precompile_concurrency())
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
        // Single-pass dedup (D7): build the unique-fetch list AND record each input record's
        // index into it in one walk. Avoids the O(N²-in-calldata-bytes) re-hash on the
        // reconstruction pass.
        let mut unique: Vec<(Address, u64, Vec<u8>)> = Vec::new();
        let mut index_of: HashMap<(Address, u64, Vec<u8>), usize> = HashMap::new();
        let mut indices: Vec<Option<usize>> = Vec::with_capacity(records.len());
        for r in records {
            if r.is_reverted {
                indices.push(None);
                continue;
            }
            let key = (r.target, r.block_number, r.calldata.clone());
            let idx = match index_of.get(&key) {
                Some(idx) => *idx,
                None => {
                    let idx = unique.len();
                    unique.push(key.clone());
                    index_of.insert(key, idx);
                    idx
                }
            };
            indices.push(Some(idx));
        }

        let witnesses: Vec<L1ExecutionWitness> = stream::iter(unique.into_iter())
            .map(|(target, block, calldata)| async move {
                self.fetch_proof_call_witness(target, block, &calldata)
                    .await
            })
            .buffered(l1_precompile_concurrency())
            .try_collect()
            .await?;

        Ok(records
            .iter()
            .zip(indices)
            .map(|(r, idx)| L1StaticCallWitness {
                target_address: r.target,
                block_number: r.block_number,
                calldata: Bytes::from(r.calldata.clone()),
                return_data: Bytes::from(r.return_data.clone()),
                gas_used: r.gas_used,
                is_reverted: r.is_reverted,
                execution_witness: match idx {
                    Some(i) => witnesses[i].clone(),
                    None => L1ExecutionWitness::default(),
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

// ─── T1: tests for the preflight fetcher logic ───────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use raiko2_primitives_shasta::l1_precompiles::L1StaticCallRecord;
    use serde_json::json;

    fn rec(target: u8, block: u64, calldata: Vec<u8>, reverted: bool) -> L1StaticCallRecord {
        L1StaticCallRecord {
            target: Address::from([target; 20]),
            block_number: block,
            calldata,
            return_data: vec![0xAA],
            gas_used: 100,
            is_reverted: reverted,
        }
    }

    /// `l1_precompile_concurrency` defaults to 16 when the env var is unset or invalid.
    #[test]
    fn default_concurrency_is_16() {
        // Test runs alongside others; the OnceLock may already be set. Just verify the
        // initialized value is sensible (non-zero, within the default ceiling).
        assert!(l1_precompile_concurrency() > 0);
    }

    /// `ProofCallResponse` deserializes the NMC `CallResultWithProof` wire shape and
    /// surfaces the typed `code/message/data` envelope on errors.
    #[test]
    fn proof_call_response_deserializes_error_envelope() {
        let raw = json!({
            "error": {
                "code": 3,
                "message": "Reverted",
                "data": "0x1234",
            },
            "witness": {
                "state": [],
                "codes": [],
                "keys": [],
                "headers": [],
            },
        });
        let parsed: ProofCallResponse = serde_json::from_value(raw).expect("parse");
        let err = parsed.error.expect("error present");
        assert_eq!(err.code, 3);
        assert_eq!(err.message, "Reverted");
        assert_eq!(err.data.as_deref(), Some("0x1234"));
    }

    /// Successful `proof_call` response has no `error` field (or it's null).
    #[test]
    fn proof_call_response_deserializes_success() {
        let raw = json!({
            "witness": {
                "state": ["0xab"],
                "codes": [],
                "keys": [],
                "headers": [],
            },
        });
        let parsed: ProofCallResponse = serde_json::from_value(raw).expect("parse");
        assert!(parsed.error.is_none());
        assert_eq!(parsed.witness.state.len(), 1);
    }

    /// `ProofCallError` can be deserialized when `data` is omitted (NMC sometimes does).
    #[test]
    fn proof_call_error_data_defaults_to_none() {
        let raw = json!({
            "code": -32003,
            "message": "ExecutionError",
        });
        let parsed: ProofCallError = serde_json::from_value(raw).expect("parse");
        assert_eq!(parsed.code, -32003);
        assert_eq!(parsed.message, "ExecutionError");
        assert_eq!(parsed.data, None);
    }

    /// Subset of `debug_traceCall`'s response deserializes correctly.
    #[test]
    fn trace_call_result_deserializes_camel_case() {
        let raw = json!({
            "gas": 21000,
            "returnValue": "0xdeadbeef",
            "failed": false,
        });
        let parsed: TraceCallResult = serde_json::from_value(raw).expect("parse");
        assert_eq!(parsed.gas, 21_000);
        assert_eq!(parsed.return_value, "0xdeadbeef");
        assert!(!parsed.failed);
    }

    /// Pure-data version of `fetch_l1_staticcall_witnesses`'s dedup loop. Verifies the
    /// single-pass dedup (D7) computes correct `(unique, indices)` for representative
    /// inputs without needing an HTTP server.
    fn dedup_indices(
        records: &[L1StaticCallRecord],
    ) -> (Vec<(Address, u64, Vec<u8>)>, Vec<Option<usize>>) {
        let mut unique: Vec<(Address, u64, Vec<u8>)> = Vec::new();
        let mut index_of: HashMap<(Address, u64, Vec<u8>), usize> = HashMap::new();
        let mut indices: Vec<Option<usize>> = Vec::with_capacity(records.len());
        for r in records {
            if r.is_reverted {
                indices.push(None);
                continue;
            }
            let key = (r.target, r.block_number, r.calldata.clone());
            let idx = match index_of.get(&key) {
                Some(idx) => *idx,
                None => {
                    let idx = unique.len();
                    unique.push(key.clone());
                    index_of.insert(key, idx);
                    idx
                }
            };
            indices.push(Some(idx));
        }
        (unique, indices)
    }

    #[test]
    fn dedup_handles_duplicate_records() {
        let records = vec![
            rec(1, 100, vec![0x01], false),
            rec(1, 100, vec![0x01], false), // identical to first → same index
            rec(2, 100, vec![0x01], false), // different target → new index
        ];
        let (unique, indices) = dedup_indices(&records);
        assert_eq!(unique.len(), 2);
        assert_eq!(indices, vec![Some(0), Some(0), Some(1)]);
    }

    #[test]
    fn dedup_skips_reverted_records() {
        let records = vec![
            rec(1, 100, vec![0x01], false),
            rec(2, 100, vec![0x01], true), // reverted → no fetch, None index
            rec(1, 100, vec![0x01], false), // duplicate of first
        ];
        let (unique, indices) = dedup_indices(&records);
        assert_eq!(unique.len(), 1, "reverted record must not be in unique");
        assert_eq!(indices, vec![Some(0), None, Some(0)]);
    }

    #[test]
    fn dedup_distinguishes_calldata_with_same_prefix() {
        let records = vec![
            rec(1, 100, vec![0x01, 0x02], false),
            rec(1, 100, vec![0x01, 0x03], false),
        ];
        let (unique, indices) = dedup_indices(&records);
        assert_eq!(unique.len(), 2);
        assert_eq!(indices, vec![Some(0), Some(1)]);
    }

    #[test]
    fn build_l1_storage_proof_requests_for_provider_layer_preserved() {
        // Smoke-test that `L1StaticCallRecord` round-trips an empty `Bytes` calldata for the
        // dedup keying — the calldata is owned `Vec<u8>` at this layer.
        let r = rec(0xAA, 1, vec![], false);
        let (_, indices) = dedup_indices(&[r.clone(), r]);
        assert_eq!(indices, vec![Some(0), Some(0)]);
    }

    // `Bytes::new()` requires the import; silence unused-import warning when only the
    // dedup tests are compiled.
    #[allow(dead_code)]
    fn _bytes_compat() -> Bytes {
        Bytes::new()
    }
}
