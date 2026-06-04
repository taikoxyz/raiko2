//! L1SLOAD MPT proof verification and precompile cache population.
//!
//! Walks backward from the L1 origin header (the root of trust, verified on-chain via
//! the Proposal event's `originBlockHash`) through `l1_headers` to derive trusted state
//! roots for each block in the range, then verifies every `L1StorageProof` against the
//! matching state root before populating the alethia-reth-evm precompile cache.

use alethia_reth_evm::precompiles::l1sload::{
    clear_l1_storage, set_l1_origin_block_id, set_l1_storage_value,
};
use alloy_consensus::{Header, TrieAccount};
use alloy_primitives::{B256, Bytes, U256, keccak256};
use alloy_rlp::{Buf, Decodable, Header as RlpHeader};
use alloy_trie::{Nibbles, proof::verify_proof};
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::collections::HashMap;
use tracing::{debug, trace};

use super::L1StorageProof;

/// Verify L1SLOAD proofs via MPT against header-chain state roots **without** populating
/// the precompile cache. Useful for host-side preflight (S7) — surface MPT mismatches at the
/// fetch boundary rather than letting them blow up deep inside the guest verifier.
///
/// Returns `Ok(())` only if every proof verifies against its trusted state root.
pub fn verify_l1sload_proofs(
    l1_storage_proofs: &[L1StorageProof],
    l1_origin_header: &Header,
    l1_headers: &[Header],
) -> Result<()> {
    if l1_storage_proofs.is_empty() {
        debug!("L1SLOAD: no proofs to verify, skipping");
        return Ok(());
    }

    let l1_origin_block_number = l1_origin_header.number;

    debug!(
        "L1SLOAD: verifying {} proofs (l1_origin={}, headers={})",
        l1_storage_proofs.len(),
        l1_origin_block_number,
        l1_headers.len()
    );

    // Build verified block_number → state_root map by walking backward from L1 origin.
    let state_root_map = build_verified_state_root_map(l1_origin_header, l1_headers)?;

    debug!(
        "L1SLOAD: built state root map with {} entries (l1_origin={}, headers={})",
        state_root_map.len(),
        l1_origin_block_number,
        l1_headers.len()
    );

    for (i, proof) in l1_storage_proofs.iter().enumerate() {
        let requested_block = block_number_from_b256(&proof.block_number)?;

        let state_root = state_root_map.get(&requested_block).ok_or_else(|| {
            anyhow!(
                "No verified state root for L1 block {} (l1_origin={}, available blocks: {:?})",
                requested_block,
                l1_origin_block_number,
                state_root_map.keys().collect::<Vec<_>>()
            )
        })?;

        if let Err(e) = verify_l1_proof(proof, *state_root) {
            bail!(
                "L1SLOAD proof verification failed for proof #{} \
                 (contract={:?}, key={:?}, block={}, state_root={:?}): {}",
                i,
                proof.contract_address,
                proof.storage_key,
                requested_block,
                state_root,
                e
            );
        }
    }

    debug!(
        "L1SLOAD: verified {} storage proofs",
        l1_storage_proofs.len()
    );
    Ok(())
}

/// Verify L1SLOAD proofs via MPT against header-chain state roots, then populate the cache.
///
/// Walks backward from the L1 origin header (the root of trust, verified on-chain via
/// the Proposal event's originBlockHash) through `l1_headers` to derive trusted state roots
/// for each block in the range. After verification, writes each `(addr, key, block) → value`
/// triple to the precompile cache so the L2 EVM can serve it.
///
/// **Precondition**: the caller must have already set the L1 origin context via
/// [`set_l1sload_origin`]. Re-setting it here would mask a context-unset bug at the call
/// site rather than surfacing it.
pub fn verify_and_populate_l1sload_proofs(
    l1_storage_proofs: &[L1StorageProof],
    l1_origin_header: &Header,
    l1_headers: &[Header],
) -> Result<()> {
    verify_l1sload_proofs(l1_storage_proofs, l1_origin_header, l1_headers)?;
    for proof in l1_storage_proofs {
        set_l1_storage_value(
            proof.contract_address,
            proof.storage_key,
            proof.block_number,
            proof.value,
        );
    }
    if !l1_storage_proofs.is_empty() {
        debug!(
            "L1SLOAD: cached {} verified storage proofs",
            l1_storage_proofs.len()
        );
    }
    Ok(())
}

/// Set the L1 origin context (the upper bound of the `[origin − 256, origin]` lookback
/// window). Cache population is the **sole responsibility** of
/// [`verify_and_populate_l1sload_proofs`] (S4): writing unverified proof values to the cache
/// before MPT verification runs would let an observer see untrusted state if a future code
/// path read between the two calls. Keep origin-setup separate from cache-population.
pub fn set_l1sload_origin(l1_origin_block_number: u64) {
    debug!("L1 precompiles: origin context set (l1_origin={l1_origin_block_number})");
    set_l1_origin_block_id(l1_origin_block_number);
}

/// Backward-compatible alias for [`set_l1sload_origin`] — older callers used
/// `populate_l1sload_cache(&[], origin)` to install the origin context as a side effect.
/// Behaviour is now strict: this is a pure origin setter, NOT a proof-cache pre-populator.
/// The `_proofs` parameter is asserted empty to catch any caller still relying on the
/// removed (unverified) pre-write loop.
#[deprecated(note = "use set_l1sload_origin; cache population happens in verify_and_populate_l1sload_proofs")]
pub fn populate_l1sload_cache(_proofs: &[L1StorageProof], l1_origin_block_number: u64) {
    debug_assert!(
        _proofs.is_empty(),
        "populate_l1sload_cache no longer writes proofs to the cache; pass &[] and call \
         verify_and_populate_l1sload_proofs to populate"
    );
    set_l1sload_origin(l1_origin_block_number);
}

/// Clear L1SLOAD cache and block-range context.
#[inline(always)]
pub fn clear_l1sload_cache() {
    clear_l1_storage();
}

/// Build a verified map of `block_number → state_root` by walking backward from the L1 origin.
///
/// The L1 origin header is the root of trust — its hash is verified on-chain against the
/// Proposal event's `originBlockHash` (set by EVM `blockhash()` in the Inbox contract).
///
/// `l1_headers` must be ordered oldest→newest, ending just below L1 origin (i.e. the last
/// header's hash must equal `l1_origin_header.parent_hash` when walking backward).
/// We walk in reverse, verifying parent_hash linkage at each step.
pub fn build_verified_state_root_map(
    l1_origin_header: &Header,
    l1_headers: &[Header],
) -> Result<HashMap<u64, B256>> {
    let mut state_root_map = HashMap::new();

    // The L1 origin's state root is trusted (verified via anchor linkage against on-chain proposal).
    let l1_origin_number = l1_origin_header.number;
    state_root_map.insert(l1_origin_number, l1_origin_header.state_root);

    if l1_headers.is_empty() {
        return Ok(state_root_map);
    }

    // Cap the backward walk at 256 so a prover cannot extend the verified window beyond
    // the L1/L2-precompile-accepted `[l1_origin − 256, l1_origin]` range.
    ensure!(
        l1_headers.len() <= 256,
        "L1 headers exceed 256-block lookback cap ({} provided)",
        l1_headers.len(),
    );

    // Guard against the otherwise-silent `u64` wrap on `l1_origin_number - 1` when
    // origin == 0 (impossible in production but reachable via test fixtures).
    ensure!(
        l1_origin_number >= 1,
        "L1 origin block number must be >= 1 for backward walk"
    );

    // Headers are ordered oldest→newest and do NOT include the L1 origin itself.
    // Walk in reverse (newest→oldest), starting from the origin's parent_hash since
    // the highest header in l1_headers is block (l1_origin - 1).
    let mut expected_hash = l1_origin_header.parent_hash;
    let mut expected_number = l1_origin_number - 1;
    for header in l1_headers.iter().rev() {
        if header.number != expected_number {
            bail!(
                "L1 header block number mismatch: expected {}, got {}",
                expected_number,
                header.number
            );
        }
        let header_hash = header.hash_slow();
        if header_hash != expected_hash {
            bail!(
                "L1 header chain broken at block {}: hash={:?}, expected={:?}",
                header.number,
                header_hash,
                expected_hash
            );
        }
        state_root_map.insert(header.number, header.state_root);
        expected_hash = header.parent_hash;
        // Use `saturating_sub` to avoid `attempt to subtract with overflow` when the walk
        // reaches block 0 (the genesis block). The next loop iteration — if any — will fail
        // the number check at line `expected_number` because the saturated value re-uses 0,
        // so soundness is preserved while a panic on perfectly-deep chains is avoided.
        expected_number = expected_number.saturating_sub(1);
    }

    Ok(state_root_map)
}

/// Convert a B256 block number to u64
fn block_number_from_b256(block_number: &B256) -> Result<u64> {
    let u256 = U256::from_be_bytes(block_number.0);
    u256.try_into()
        .map_err(|_| anyhow!("L1SLOAD block number exceeds u64: {:?}", block_number))
}

/// Verify L1 storage and account proof against a given state root using MPT proof verification.
/// For non-existent accounts/storage should return zero, given that the provided proofs are empty.
fn verify_l1_proof(proof: &L1StorageProof, state_root: B256) -> Result<()> {
    let account_key = keccak256(proof.contract_address.as_slice());
    let account_rlp = get_and_verify_value(account_key, state_root, &proof.account_proof)?;

    // If account doesn't exist, storage must be zero
    let actual_value = if account_rlp.is_empty() {
        // Account doesn't exist on L1, value must be zero
        B256::ZERO
    } else {
        // Account exists, check storage
        let storage_root = get_storage_root(&account_rlp).with_context(|| {
            format!(
                "Failed to extract storage root for contract {:?}",
                proof.contract_address
            )
        })?;
        let storage_key_hash = keccak256(proof.storage_key.as_slice());
        let storage_rlp =
            get_and_verify_value(storage_key_hash, storage_root, &proof.storage_proof)
                .with_context(|| {
                    format!(
                        "Failed to verify storage proof for contract {:?}, key {:?}",
                        proof.contract_address, proof.storage_key
                    )
                })?;

        // Compare with claimed value
        if storage_rlp.is_empty() {
            B256::ZERO
        } else {
            let mut rlp_slice = storage_rlp.as_slice();
            B256::from(U256::decode(&mut rlp_slice).with_context(|| {
                format!(
                    "Failed to decode storage value for contract {:?}, key {:?}, raw bytes: 0x{}",
                    proof.contract_address,
                    proof.storage_key,
                    hex::encode(&storage_rlp)
                )
            })?)
        }
    };

    if actual_value != proof.value {
        bail!(
            "Value mismatch: expected {:?}, got {:?}",
            proof.value,
            actual_value
        );
    }

    Ok(())
}

/// Get value and verify proof.
/// Single-pass: extracts the leaf value first, then verifies once.
fn get_and_verify_value(key_hash: B256, root: B256, proof: &[Bytes]) -> Result<Vec<u8>> {
    let nibbles = Nibbles::unpack(key_hash);
    let proof_refs: Vec<&Bytes> = proof.iter().collect();

    // Handle empty proof array (proves non-existence at the root level)
    if proof.is_empty() {
        verify_proof(root, nibbles, None, proof_refs)?;
        return Ok(Vec::new());
    }

    // Try to extract a leaf value from the proof. If the proof terminates at a
    // leaf node, we verify existence. If extraction fails (branch/extension node
    // termination), we verify non-existence.
    match get_leaf_value(proof) {
        Ok(value) if !value.is_empty() => {
            // The terminal node is a leaf. If our key maps to it, this is an existence proof.
            // But a non-existence proof can also terminate at a *divergent* leaf — a different
            // key sharing a prefix — in which case existence verification against our key
            // fails. Fall back to verifying non-existence rather than rejecting an honest
            // absence proof. (For a fixed root a proof can validly prove at most one of the
            // two, so this never turns a real existence proof into a spurious zero.)
            if verify_proof(
                root,
                nibbles.clone(),
                Some(value.clone()),
                proof_refs.clone(),
            )
            .is_ok()
            {
                Ok(value)
            } else {
                verify_proof(root, nibbles, None, proof_refs)?;
                Ok(Vec::new())
            }
        }
        _ => {
            // No value extractable (non-existent key) — verify non-existence
            verify_proof(root, nibbles, None, proof_refs)?;
            Ok(Vec::new())
        }
    }
}

/// Extract value from leaf node in an MPT proof.
///
/// Distinguishes node types by RLP structure (matching alloy-trie's TrieNode::decode):
/// 1. Element count: 17 elements = branch node, 2 elements = leaf/extension
/// 2. HP (hex prefix) flag: 0x0/0x1 = extension, 0x2/0x3 = leaf
///
/// Returns Ok(value) only for leaf nodes. Returns Err for branch/extension nodes,
/// which signals non-existence to the caller.
fn get_leaf_value(proof: &[Bytes]) -> Result<Vec<u8>> {
    let last_node = proof.last().ok_or_else(|| anyhow!("Empty proof"))?;
    let mut data = last_node.as_ref();

    // Decode the list header
    let list_header = RlpHeader::decode(&mut data).with_context(|| {
        format!(
            "Failed to decode list header from proof node: 0x{}",
            hex::encode(last_node)
        )
    })?;

    if !list_header.list {
        bail!(
            "Last proof node is not a list, raw bytes: 0x{}",
            hex::encode(last_node)
        );
    }

    // Count elements to distinguish node types:
    // - 17 elements = branch node (non-existence proof terminates here)
    // - 2 elements = leaf or extension node
    // This matches alloy-trie's TrieNode::decode logic (nodes/mod.rs).
    let mut count_data = data.get(..list_header.payload_length).ok_or_else(|| {
        anyhow!(
            "Proof node truncated: payload_length {} exceeds remaining data {}",
            list_header.payload_length,
            data.len()
        )
    })?;
    let mut element_count = 0u32;
    while !count_data.is_empty() {
        let header = RlpHeader::decode(&mut count_data).with_context(|| {
            format!(
                "Failed to decode element {} in proof node: 0x{}",
                element_count,
                hex::encode(last_node)
            )
        })?;
        count_data.advance(header.payload_length);
        element_count += 1;
    }

    if element_count != 2 {
        bail!(
            "Last proof node has {} elements (expected 2 for leaf/extension). \
             This is a branch node, meaning the key does not exist at this path.",
            element_count
        );
    }

    // 2-element node: decode [path, value]
    let path_header = RlpHeader::decode(&mut data)
        .with_context(|| format!("Failed to decode path header: 0x{}", hex::encode(last_node)))?;

    // Check the HP (hex prefix) to distinguish leaf from extension nodes.
    // The first nibble of the compact-encoded path encodes the node type:
    //   0x0 or 0x1 → extension node
    //   0x2 or 0x3 → leaf node
    let path_bytes = data.get(..path_header.payload_length).ok_or_else(|| {
        anyhow!(
            "Proof node truncated: path payload_length {} exceeds remaining data {}",
            path_header.payload_length,
            data.len()
        )
    })?;
    if !path_bytes.is_empty() {
        let hp_flag = path_bytes[0] >> 4;
        if hp_flag < 2 {
            bail!(
                "Last proof node is an extension node (HP flag=0x{:x}), not a leaf. \
                 This indicates the key does not exist at this path.",
                hp_flag
            );
        }
    }

    data.advance(path_header.payload_length);

    // Decode the value element header to get its payload
    let value_header = RlpHeader::decode(&mut data)
        .with_context(|| "Failed to decode value header".to_string())?;

    // In an MPT leaf node [path, value], when the 2-element list is decoded,
    // the value field is the PAYLOAD only (not including the RLP header).
    let value = data
        .get(..value_header.payload_length)
        .ok_or_else(|| {
            anyhow!(
                "Proof node truncated: value payload_length {} exceeds remaining data {}",
                value_header.payload_length,
                data.len()
            )
        })?
        .to_vec();

    trace!(
        "Extracted leaf value: {} bytes (RLP-encoded) from leaf node",
        value.len()
    );
    Ok(value)
}

/// Extract the storage root from an account's RLP (`[nonce, balance, storage_root, code_hash]`).
fn get_storage_root(account_rlp: &[u8]) -> Result<B256> {
    let account = TrieAccount::decode(&mut &account_rlp[..]).with_context(|| {
        format!(
            "Failed to decode account RLP: 0x{}",
            hex::encode(account_rlp)
        )
    })?;
    Ok(account.storage_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Header;
    use alloy_primitives::{Address, B256, Bytes, U256};
    use alloy_rlp::Encodable;
    use serial_test::serial;

    // ───────────────────────────────────────────────
    // Lock poison recovery
    // ───────────────────────────────────────────────

    /// Regression test for the lock-poison hardening in `acquire_l1_precompile_lock`.
    ///
    /// Before the fix, the acquire function called `.expect(...)` on the
    /// `LockResult`, which panicked on `PoisonError`. A single panic during preflight
    /// discovery (e.g., an L1 RPC flake mid-execution) would poison the global
    /// `L1_PRECOMPILE_EXECUTION_LOCK` and every subsequent proof attempt would fail with
    /// `L1 precompile execution lock poisoned`. The fix routes through `into_inner()` so
    /// the next acquirer takes ownership of the inner guard and can clear state.
    ///
    /// This test deliberately panics inside a thread holding the lock to poison it, then
    /// asserts that the next `acquire_l1_precompile_lock()` call still succeeds (returns
    /// a guard instead of panicking).
    #[test]
    fn acquire_l1_precompile_lock_recovers_from_poison() {
        // Force the lock into a poisoned state by panicking while holding it. We use a
        // separate thread so the panic doesn't tear down the test process; the join
        // returns Err(_) carrying the panic payload, which we discard.
        let poisoner = std::thread::spawn(|| {
            let _guard = super::super::acquire_l1_precompile_lock();
            panic!("intentional panic to poison the lock");
        });
        let _ = poisoner.join(); // Err(_) carrying the panic; ignored.

        // Sanity: the underlying mutex IS poisoned now.
        assert!(
            super::super::L1_PRECOMPILE_EXECUTION_LOCK.is_poisoned(),
            "lock must be poisoned after a thread panics while holding it"
        );

        // The hardened acquire_l1_precompile_lock() should NOT panic — it should return a
        // valid guard via PoisonError::into_inner().
        let guard = super::super::acquire_l1_precompile_lock();
        drop(guard);

        // After successful acquisition (and drop), the lock remains poisoned for any
        // direct `.lock()` call, but acquire_l1_precompile_lock() keeps recovering —
        // verify the second acquisition also works to lock in the regression.
        let guard2 = super::super::acquire_l1_precompile_lock();
        drop(guard2);
    }

    // ───────────────────────────────────────────────
    // Helpers
    // ───────────────────────────────────────────────

    /// Build a Header with a given number, state_root, and parent_hash.
    fn make_header(number: u64, state_root: B256, parent_hash: B256) -> Header {
        Header {
            number,
            state_root,
            parent_hash,
            ..Default::default()
        }
    }

    /// RLP-encode an Ethereum account: [account_nonce, balance, storage_root, code_hash].
    fn rlp_encode_account(
        account_nonce: u64,
        balance: U256,
        storage_root: B256,
        code_hash: B256,
    ) -> Vec<u8> {
        use alloy_rlp::BytesMut;
        let mut buf = BytesMut::new();

        let mut fields = BytesMut::new();
        account_nonce.encode(&mut fields);
        balance.encode(&mut fields);
        storage_root.encode(&mut fields);
        code_hash.encode(&mut fields);

        alloy_rlp::Header {
            list: true,
            payload_length: fields.len(),
        }
        .encode(&mut buf);
        buf.extend_from_slice(&fields);
        buf.to_vec()
    }

    // ───────────────────────────────────────────────
    // block_number_from_b256
    // ───────────────────────────────────────────────

    #[test]
    fn test_block_number_from_b256_valid() {
        let bn = B256::from(U256::from(12345u64));
        assert_eq!(block_number_from_b256(&bn).unwrap(), 12345u64);
    }

    #[test]
    fn test_block_number_from_b256_zero() {
        let bn = B256::ZERO;
        assert_eq!(block_number_from_b256(&bn).unwrap(), 0u64);
    }

    #[test]
    fn test_block_number_from_b256_max_u64() {
        let bn = B256::from(U256::from(u64::MAX));
        assert_eq!(block_number_from_b256(&bn).unwrap(), u64::MAX);
    }

    #[test]
    fn test_block_number_from_b256_overflow() {
        let too_big = U256::from(u64::MAX) + U256::from(1);
        let bn = B256::from(too_big);
        assert!(block_number_from_b256(&bn).is_err());
    }

    // ───────────────────────────────────────────────
    // build_verified_state_root_map
    // ───────────────────────────────────────────────

    #[test]
    fn test_state_root_map_origin_only() {
        let origin = make_header(100, B256::from([1u8; 32]), B256::ZERO);
        let map = build_verified_state_root_map(&origin, &[]).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map[&100], B256::from([1u8; 32]));
    }

    #[test]
    fn test_state_root_map_single_parent() {
        let parent = make_header(100, B256::from([0xAAu8; 32]), B256::ZERO);
        let parent_hash = parent.hash_slow();

        let origin = make_header(101, B256::from([0xBBu8; 32]), parent_hash);

        let map = build_verified_state_root_map(&origin, &[parent]).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map[&101], B256::from([0xBBu8; 32]));
        assert_eq!(map[&100], B256::from([0xAAu8; 32]));
    }

    #[test]
    fn test_state_root_map_chain_of_three() {
        let h98 = make_header(98, B256::from([1u8; 32]), B256::ZERO);
        let h98_hash = h98.hash_slow();

        let h99 = make_header(99, B256::from([2u8; 32]), h98_hash);
        let h99_hash = h99.hash_slow();

        let origin = make_header(100, B256::from([3u8; 32]), h99_hash);

        let map = build_verified_state_root_map(&origin, &[h98, h99]).unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(map[&98], B256::from([1u8; 32]));
        assert_eq!(map[&99], B256::from([2u8; 32]));
        assert_eq!(map[&100], B256::from([3u8; 32]));
    }

    #[test]
    fn test_state_root_map_broken_chain() {
        let wrong_parent = make_header(99, B256::from([1u8; 32]), B256::ZERO);
        let origin = make_header(100, B256::from([2u8; 32]), B256::from([0xFFu8; 32]));

        let result = build_verified_state_root_map(&origin, &[wrong_parent]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("header chain broken")
        );
    }

    #[test]
    fn test_state_root_map_wrong_block_number() {
        let h98 = make_header(98, B256::from([1u8; 32]), B256::ZERO);
        let h98_hash = h98.hash_slow();

        let h_wrong = make_header(97, B256::from([2u8; 32]), h98_hash);
        let h_wrong_hash = h_wrong.hash_slow();

        let origin = make_header(100, B256::from([3u8; 32]), h_wrong_hash);

        let result = build_verified_state_root_map(&origin, &[h98, h_wrong]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("block number mismatch")
        );
    }

    // ───────────────────────────────────────────────
    // get_storage_root
    // ───────────────────────────────────────────────

    #[test]
    fn test_get_storage_root_valid_account() {
        let expected_storage_root = B256::from([0xCCu8; 32]);
        let code_hash = B256::from([0xDDu8; 32]);
        let account_rlp = rlp_encode_account(1, U256::from(1000), expected_storage_root, code_hash);

        let result = get_storage_root(&account_rlp).unwrap();
        assert_eq!(result, expected_storage_root);
    }

    #[test]
    fn test_get_storage_root_zero_nonce_zero_balance() {
        let expected_storage_root = B256::from([0xABu8; 32]);
        let code_hash = B256::from([0xEFu8; 32]);
        let account_rlp = rlp_encode_account(0, U256::ZERO, expected_storage_root, code_hash);

        let result = get_storage_root(&account_rlp).unwrap();
        assert_eq!(result, expected_storage_root);
    }

    #[test]
    fn test_get_storage_root_large_balance() {
        let expected_storage_root = B256::from([0x11u8; 32]);
        let code_hash = B256::from([0x22u8; 32]);
        let balance = U256::from(100) * U256::from(10).pow(U256::from(18));
        let account_rlp = rlp_encode_account(42, balance, expected_storage_root, code_hash);

        let result = get_storage_root(&account_rlp).unwrap();
        assert_eq!(result, expected_storage_root);
    }

    #[test]
    fn test_get_storage_root_invalid_rlp() {
        let garbage = vec![0xFF, 0x01, 0x02];
        assert!(get_storage_root(&garbage).is_err());
    }

    #[test]
    fn test_get_storage_root_not_a_list() {
        let mut buf = alloy_rlp::BytesMut::new();
        B256::ZERO.encode(&mut buf);
        assert!(get_storage_root(&buf).is_err());
    }

    // ───────────────────────────────────────────────
    // get_leaf_value
    // ───────────────────────────────────────────────

    #[test]
    fn test_get_leaf_value_empty_proof() {
        let result = get_leaf_value(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_leaf_value_leaf_node() {
        let mut buf = alloy_rlp::BytesMut::new();

        let path = vec![0x20]; // HP flag = 2 (leaf), empty path
        let value = vec![0x80, 0x90, 0xA0]; // all >= 0x80

        let mut inner = alloy_rlp::BytesMut::new();
        alloy_primitives::Bytes::from(path.clone()).encode(&mut inner);
        alloy_primitives::Bytes::from(value.clone()).encode(&mut inner);

        alloy_rlp::Header {
            list: true,
            payload_length: inner.len(),
        }
        .encode(&mut buf);
        buf.extend_from_slice(&inner);

        let proof = vec![Bytes::from(buf.to_vec())];
        let result = get_leaf_value(&proof).unwrap();
        assert_eq!(result, value);
    }

    #[test]
    fn test_get_leaf_value_extension_node_rejected() {
        let mut buf = alloy_rlp::BytesMut::new();

        let path = vec![0x00, 0xAB]; // HP flag = 0 (extension), with nibble
        let value = vec![0x01, 0x02, 0x03];

        let mut inner = alloy_rlp::BytesMut::new();
        alloy_rlp::Header {
            list: false,
            payload_length: path.len(),
        }
        .encode(&mut inner);
        inner.extend_from_slice(&path);
        alloy_rlp::Header {
            list: false,
            payload_length: value.len(),
        }
        .encode(&mut inner);
        inner.extend_from_slice(&value);

        alloy_rlp::Header {
            list: true,
            payload_length: inner.len(),
        }
        .encode(&mut buf);
        buf.extend_from_slice(&inner);

        let proof = vec![Bytes::from(buf.to_vec())];
        let result = get_leaf_value(&proof);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("extension node"));
    }

    #[test]
    fn test_get_leaf_value_branch_node_rejected() {
        let mut inner = alloy_rlp::BytesMut::new();
        for _ in 0..17 {
            alloy_rlp::Header {
                list: false,
                payload_length: 0,
            }
            .encode(&mut inner);
        }

        let mut buf = alloy_rlp::BytesMut::new();
        alloy_rlp::Header {
            list: true,
            payload_length: inner.len(),
        }
        .encode(&mut buf);
        buf.extend_from_slice(&inner);

        let proof = vec![Bytes::from(buf.to_vec())];
        let result = get_leaf_value(&proof);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("branch node"));
    }

    // ───────────────────────────────────────────────
    // verify_and_populate_l1sload_proofs (integration)
    // ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_verify_empty_proofs_succeeds() {
        clear_l1sload_cache();
        let origin = make_header(100, B256::ZERO, B256::ZERO);
        let result = verify_and_populate_l1sload_proofs(&[], &origin, &[]);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_verify_proof_missing_state_root() {
        clear_l1sload_cache();
        let origin = make_header(100, B256::from([1u8; 32]), B256::ZERO);

        let proof = L1StorageProof {
            contract_address: Address::from([1u8; 20]),
            storage_key: B256::from([2u8; 32]),
            block_number: B256::from(U256::from(50u64)),
            value: B256::ZERO,
            account_proof: vec![],
            storage_proof: vec![],
        };

        let result = verify_and_populate_l1sload_proofs(&[proof], &origin, &[]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No verified state root")
        );
    }

    // ───────────────────────────────────────────────
    // set_l1sload_origin + verify_and_populate_l1sload_proofs cache writeback
    // ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_set_origin_only() {
        clear_l1sload_cache();
        set_l1sload_origin(110);
        // No proofs to populate yet — origin set is the only effect.
    }

    #[test]
    #[serial]
    fn test_set_origin_then_direct_cache_write_round_trips_via_precompile() {
        // After S4 the proof-population side effect lives in `verify_and_populate_l1sload_proofs`,
        // not in the origin setter. This test exercises the round trip without the MPT step by
        // writing the cache value directly via `set_l1_storage_value` — the precompile-side
        // contract is the same.
        clear_l1sload_cache();

        let addr = Address::from([0xAAu8; 20]);
        let key = B256::from([0xBBu8; 32]);
        let block_num = B256::from(U256::from(100u64));
        let value = B256::from([0xCCu8; 32]);

        set_l1sload_origin(110);
        set_l1_storage_value(addr, key, block_num, value);

        // Verify the value was cached by invoking the precompile.
        use alethia_reth_evm::precompiles::l1sload::l1sload_run;
        let mut input = vec![0u8; 84];
        input[0..20].copy_from_slice(addr.as_slice());
        input[20..52].copy_from_slice(key.as_slice());
        input[52..84].copy_from_slice(block_num.as_slice());

        let result = l1sload_run(&input, 5000, 0);
        assert!(
            result.is_ok(),
            "Cached value should be retrievable via precompile"
        );
        assert_eq!(result.unwrap().bytes.as_ref(), value.as_slice());

        clear_l1sload_cache();
    }

    // ───────────────────────────────────────────────
    // acquire_l1_precompile_lock
    // ───────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_acquire_lock_returns_guard() {
        let guard = super::super::acquire_l1_precompile_lock();
        drop(guard);
    }

    // ───────────────────────────────────────────────
    // verify_l1_proof with real MPT data (non-existent account)
    // ───────────────────────────────────────────────

    #[test]
    fn test_verify_proof_nonexistent_account_empty_proof() {
        let empty_root = keccak256([]);

        let proof = L1StorageProof {
            contract_address: Address::from([0x42u8; 20]),
            storage_key: B256::from([0x01u8; 32]),
            block_number: B256::from(U256::from(100u64)),
            value: B256::ZERO,
            account_proof: vec![],
            storage_proof: vec![],
        };

        let result = verify_l1_proof(&proof, empty_root);
        let empty_trie_root = keccak256([0x80u8]);
        let result2 = verify_l1_proof(&proof, empty_trie_root);
        assert!(
            result.is_ok() || result2.is_ok(),
            "Non-existent account with zero value should verify against empty trie"
        );
    }

    #[test]
    #[serial]
    fn test_verify_proof_value_mismatch_fails() {
        clear_l1sload_cache();
        let origin = make_header(100, B256::from([1u8; 32]), B256::ZERO);

        let proof = L1StorageProof {
            contract_address: Address::from([0x42u8; 20]),
            storage_key: B256::from([0x01u8; 32]),
            block_number: B256::from(U256::from(100u64)),
            value: B256::from([0xFFu8; 32]),
            account_proof: vec![],
            storage_proof: vec![],
        };

        let result = verify_and_populate_l1sload_proofs(&[proof], &origin, &[]);
        assert!(result.is_err(), "Value mismatch should fail verification");
    }

    /// Regression: a non-existence proof that terminates at a *divergent* leaf must verify as
    /// absence (empty value), not be rejected. A single-entry trie's root IS the leaf for its
    /// one key, so a proof for any other key is that same leaf node — the divergent-leaf case.
    #[test]
    fn test_get_and_verify_value_accepts_divergent_leaf_nonexistence() {
        use risc0_ethereum_trie::CachedTrie;

        let mut trie = CachedTrie::default();
        let existing = keccak256(B256::from(U256::from(1u64)).as_slice());
        let mut value_rlp = Vec::new();
        U256::from(42u64).encode(&mut value_rlp);
        trie.insert(existing, value_rlp);
        let root = trie.hash();
        let proof: Vec<Bytes> = trie.rlp_nodes();

        // A different key whose hash diverges from `existing`; its proof terminates at the
        // existing key's leaf.
        let absent = keccak256(B256::from(U256::from(999u64)).as_slice());
        let out = get_and_verify_value(absent, root, &proof)
            .expect("divergent-leaf non-existence must verify, not error");
        assert!(
            out.is_empty(),
            "absent key must resolve to empty, got {} bytes",
            out.len()
        );
    }

    // ── T10: build_verified_state_root_map boundary cases ─────────────

    /// `l1_headers.len() > 256` must be rejected — the cap defends the trust window.
    #[test]
    fn test_state_root_map_rejects_over_256_headers() {
        let mut headers = Vec::with_capacity(257);
        // Build a deeper-but-detached chain (parent_hash linkage doesn't matter — the cap
        // check fires first).
        for i in 0..257u64 {
            headers.push(make_header(i, B256::from([(i as u8); 32]), B256::ZERO));
        }
        let origin = make_header(257, B256::from([0x77u8; 32]), B256::ZERO);
        let err = build_verified_state_root_map(&origin, &headers).unwrap_err();
        assert!(
            err.to_string().contains("exceed 256-block lookback cap"),
            "expected 256-cap rejection, got: {err}"
        );
    }

    /// Exactly 256 ancestors must be accepted (boundary inclusive).
    #[test]
    fn test_state_root_map_accepts_exactly_256_headers() {
        // Build a real linked chain of 256 ancestors + 1 origin so the parent_hash walk
        // succeeds.
        let mut headers: Vec<Header> = Vec::with_capacity(256);
        let mut prev_hash = B256::ZERO;
        for i in 0..256u64 {
            let h = make_header(i, B256::from([(i as u8); 32]), prev_hash);
            prev_hash = h.hash_slow();
            headers.push(h);
        }
        let origin = make_header(256, B256::from([0xAAu8; 32]), prev_hash);
        let map = build_verified_state_root_map(&origin, &headers).expect("256 = boundary OK");
        assert_eq!(map.len(), 257, "origin + 256 ancestors = 257 entries");
    }

    /// `l1_origin_number == 0` with non-empty headers must bail (underflow on backward walk).
    #[test]
    fn test_state_root_map_rejects_origin_zero_with_headers() {
        let origin = make_header(0, B256::from([0x11u8; 32]), B256::ZERO);
        let bogus_ancestor = make_header(0, B256::from([0x22u8; 32]), B256::ZERO);
        let err = build_verified_state_root_map(&origin, &[bogus_ancestor]).unwrap_err();
        assert!(
            err.to_string().contains("must be >= 1 for backward walk"),
            "expected underflow guard, got: {err}"
        );
    }

    /// `l1_origin_number == 0` with empty headers must SUCCEED — origin is the only entry.
    #[test]
    fn test_state_root_map_origin_zero_empty_headers_ok() {
        let origin = make_header(0, B256::from([0x11u8; 32]), B256::ZERO);
        let map = build_verified_state_root_map(&origin, &[]).expect("origin=0, no ancestors");
        assert_eq!(map.len(), 1);
        assert_eq!(map[&0], B256::from([0x11u8; 32]));
    }
}
