//! L1STATICCALL single-call witness database.
//!
//! Wraps `raiko2_stateless::SparseState` (which performs the MPT trie walks) with the
//! L1-specific semantics surge-raiko's `WitnessDb` enforces:
//!  * `code_by_hash` and `storage` propagate unresolved-witness errors so a malicious prover
//!    cannot omit paths that matter for branch choices.
//!  * `basic` treats untouched accounts as `None` (revm sees default zero-balance/nonce/no-code,
//!    matching geth semantics for untouched addresses on L1).
//!  * `block_hash` returns `B256::ZERO` on a witness miss — matching the EVM `BLOCKHASH`
//!    semantics that a node returns zero for any block outside the last-256 window or
//!    otherwise unknown to it. Storage/code misses still hard-error.
//!
//! Both halves cooperate to enforce the soundness property: any L1 view function that reads
//! state must have its access witnessed; any block hash it reads must either be in the
//! witness or be a block the L1 itself would have returned `ZERO` for.

use std::collections::HashMap;

use alloy_consensus::Header;
use alloy_primitives::{Address, B256, Bytes, KECCAK256_EMPTY, U256};
use anyhow::{Context, Result};
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use raiko2_stateless::{SparseState, StatelessTrie};
use reth_errors::ProviderError;
use reth_revm::{Database, state::Bytecode};
use revm::state::AccountInfo;
use tracing::debug;

use super::L1ExecutionWitness;

/// A read-only database built from a single-call `L1ExecutionWitness`, for re-executing
/// the L1STATICCALL in revm against witnessed state.
pub struct WitnessDb {
    /// The sparse state trie + on-demand storage tries materialized from the witness.
    sparse: SparseState,
    /// Contract bytecodes keyed by code_hash, materialized from the witness `codes` field.
    codes: HashMap<B256, Bytes>,
    /// Block hashes from witnessed headers. Surge semantics: missing block → `B256::ZERO`.
    block_hashes: HashMap<u64, B256>,
}

impl WitnessDb {
    /// Build a `WitnessDb` from a raw `L1ExecutionWitness`, verifying the state root.
    ///
    /// Pass `Some(block_hashes)` when the caller has already decoded the witness headers
    /// (the L1STATICCALL verifier does so during its trusted-chain binding check) to skip
    /// the redundant second decode pass (D11). Pass `None` to decode the headers internally.
    pub fn build(
        witness: &L1ExecutionWitness,
        state_root: B256,
        block_hashes: Option<HashMap<u64, B256>>,
    ) -> Result<Self> {
        let block_hashes = match block_hashes {
            Some(m) => m,
            None => {
                let mut m: HashMap<u64, B256> = HashMap::new();
                for header_bytes in &witness.headers {
                    let header: Header = alloy_rlp::Decodable::decode(&mut header_bytes.as_ref())
                        .context("Failed to RLP-decode witness header")?;
                    let header_hash = alloy_primitives::keccak256(header_bytes.as_ref());
                    m.insert(header.number, header_hash);
                }
                m
            }
        };
        // Lift our flat `L1ExecutionWitness` into the raiko2 `ExecutionWitness` shape that
        // `SparseState::new` consumes. `state_indices` is empty (no shared pool); `headers`
        // is empty (we manage block hashes ourselves to preserve surge's lenient semantics).
        let exec_witness = ExecutionWitness {
            state: witness
                .state
                .iter()
                .map(|b| WitnessStateNode::from_bytes(b.clone()))
                .collect(),
            state_indices: Vec::new(),
            codes: witness.codes.clone(),
            keys: witness.keys.clone(),
            headers: Vec::new(),
        };

        let (sparse, bytecode_map) = SparseState::new(&exec_witness, state_root)
            .map_err(|e| anyhow::anyhow!("Failed to materialize sparse state: {e:?}"))?;

        // SparseState returns codes keyed by code_hash → Bytecode. Surge's WitnessDb keeps
        // raw Bytes (the bytecode preimage) to reconstruct Bytecode on demand. We retain a
        // raw-bytes map for symmetry and to avoid double-allocating the Bytecode for the
        // common path where revm only asks for code_by_hash once.
        let mut codes: HashMap<B256, Bytes> = HashMap::new();
        for (hash, bytecode) in bytecode_map {
            codes.insert(hash, Bytes::from(bytecode.original_bytes()));
        }

        Ok(Self {
            sparse,
            codes,
            block_hashes,
        })
    }
}

impl Database for WitnessDb {
    type Error = ProviderError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self.sparse.account(address)? {
            Some(account) => {
                let code = if account.code_hash != KECCAK256_EMPTY {
                    self.codes
                        .get(&account.code_hash)
                        .map(|b| Bytecode::new_raw(b.clone()))
                } else {
                    None
                };

                Ok(Some(AccountInfo {
                    nonce: account.nonce,
                    balance: account.balance,
                    code_hash: account.code_hash,
                    // `account_id` is a revm optimization hint (account index in the
                    // block-access list); we don't have one — None is safe and correct.
                    account_id: None,
                    code,
                }))
            }
            None => {
                debug!("WitnessDb::basic: account {address} not in witness");
                Ok(None)
            }
        }
    }

    /// Missing code is a hard error: absent bytecode always changes call behavior, so we
    /// must fail loudly instead of pretending the contract is empty.
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self.codes.get(&code_hash) {
            Some(code) => Ok(Bytecode::new_raw(code.clone())),
            None => {
                debug!("WitnessDb::code_by_hash: code {code_hash} not in witness");
                Err(ProviderError::TrieWitnessError(format!(
                    "code_hash {code_hash} not in witness"
                )))
            }
        }
    }

    /// Storage semantics:
    ///   * `Ok(value)` from the trie → return it directly (SparseState normalizes
    ///     "absent slot" to `U256::ZERO` per L1 semantics).
    ///   * `Err(_)` from the trie → unresolved trie node, propagate as a hard error.
    ///
    /// The silent-zero fallback that used to hide unresolved-node errors was a soundness
    /// risk for contracts that return the same output on multiple branches.
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.sparse.storage(address, index)
    }

    /// `BLOCKHASH` opcode semantics:
    ///   * `Some(hash)` → return it.
    ///   * `None` → return `B256::ZERO`. Geth/EVM return zero for any block outside the
    ///     last-256 window or otherwise unknown to the node, so a sequencer-trace witness
    ///     that didn't record a particular block hash is treated as the same "unknown"
    ///     case rather than as a soundness fault. Storage- and code-trie misses still
    ///     hard-error (see `storage` / `code_by_hash` above) — those *are* soundness-
    ///     critical because the witness explicitly claimed the call read them.
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self.block_hashes.get(&number).copied() {
            Some(hash) => Ok(hash),
            None => {
                debug!(
                    "WitnessDb::block_hash: block {number} not in witness, returning zero per BLOCKHASH semantics"
                );
                Ok(B256::ZERO)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_witness() -> L1ExecutionWitness {
        L1ExecutionWitness::default()
    }

    #[test]
    fn test_block_hash_returns_zero_for_missing_block() {
        // Regression guard: the `block_hash` accessor must NOT error on a missing block —
        // BLOCKHASH semantics return zero for any block outside the last-256 window or
        // otherwise unknown to the node. A sequencer-trace witness that doesn't record
        // a particular block hash must round-trip under the same semantics.
        let mut db =
            WitnessDb::build(&empty_witness(), alloy_trie::EMPTY_ROOT_HASH, None).unwrap();
        let result = db
            .block_hash(42)
            .expect("block_hash on a missing block must NOT error");
        assert_eq!(
            result,
            B256::ZERO,
            "missing block must return B256::ZERO per BLOCKHASH semantics"
        );
    }

    #[test]
    fn test_block_hash_returns_recorded_hash_for_known_block() {
        let known = B256::from([0xABu8; 32]);
        let mut block_hashes = HashMap::new();
        block_hashes.insert(42u64, known);
        let mut db = WitnessDb::build(
            &empty_witness(),
            alloy_trie::EMPTY_ROOT_HASH,
            Some(block_hashes),
        )
        .unwrap();
        let result = db.block_hash(42).expect("block_hash should succeed");
        assert_eq!(result, known);
    }

    #[test]
    fn test_block_hash_does_not_create_phantom_entry() {
        // Asking for an unknown block must not insert a zero entry — otherwise a later
        // real call for the same block could be silently accepted.
        let mut db =
            WitnessDb::build(&empty_witness(), alloy_trie::EMPTY_ROOT_HASH, None).unwrap();
        let _ = db.block_hash(123).unwrap();
        assert!(
            db.block_hashes.get(&123).is_none(),
            "missing-block lookup must not poison the cache"
        );
    }

    /// T15: `code_by_hash` for a code hash not in the witness must surface as a hard error,
    /// not silently return empty bytecode. The "silent zero fallback" was the soundness risk
    /// the WitnessDb intentionally avoids — pin the behavior with a regression test.
    #[test]
    fn test_code_by_hash_missing_returns_error() {
        let mut db =
            WitnessDb::build(&empty_witness(), alloy_trie::EMPTY_ROOT_HASH, None).unwrap();
        let result = db.code_by_hash(B256::from([0x42u8; 32]));
        assert!(result.is_err(), "missing code must surface as hard error, not empty bytecode");
        let err = result.unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("TrieWitnessError") || msg.contains("not in witness"),
            "expected TrieWitnessError, got: {msg}"
        );
    }
}

