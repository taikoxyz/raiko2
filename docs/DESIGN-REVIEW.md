# Raiko2 Design Review - Legacy Alignment

## Overview

This document reviews the alignment between raiko2's design and the latest legacy raiko code (after rebasing on `origin/feat/shasta-merge-main`).

## Key Findings

### 1. DerivationSource Structure Mismatch ❌

**Legacy (lib/src/input/shasta.rs):**

```rust
struct DerivationSource {
    /// @notice Whether this source is from a forced inclusion.
    bool isForcedInclusion;
    /// @notice Blobs that contain the source's manifest data.
    BlobSlice blobSlice;
}
```

**Raiko2 (crates/protocol/src/shasta/types.rs):**

```rust
pub struct DerivationSource {
    pub blob_slice: BlobSlice,
    pub flags: u8,  // Should be is_forced_inclusion: bool
}
```

**Action Required:** Update raiko2 `DerivationSource` to use `is_forced_inclusion: bool` instead of `flags: u8`.

---

### 2. Codec Decode Order Mismatch ❌

**Legacy (lib/src/input/shasta.rs decode_event_data):**

```rust
// Decode order:
1. is_forced_inclusion_u8 (1 byte)
2. blob_hashes_length (2 bytes)
3. blob_hashes (N * 32 bytes)
4. offset (3 bytes)
5. blob_timestamp (6 bytes)
```

**Raiko2 (crates/protocol/src/shasta/codec.rs decode_proposed_event):**

```rust
// Decode order:
1. blob_slice (blob_hashes, offset, timestamp)
2. flags (1 byte)
```

**Action Required:** Update raiko2 codec to match legacy decode order:

- Read `is_forced_inclusion` first
- Then read blob_slice components

---

### 3. Manifest Structure Difference ⚠️

**Legacy (lib/src/manifest/types.rs):**

```rust
pub struct DerivationSourceManifest {
    pub prover_auth_bytes: Bytes,
    pub blocks: Vec<ProtocolBlockManifest>,
}

pub struct ProtocolBlockManifest {
    pub timestamp: u64,
    pub coinbase: Address,
    pub anchor_block_number: u64,
    pub gas_limit: u64,
    pub transactions: Vec<TransactionSigned>,
}
```

**Raiko2 (crates/protocol/src/shasta/manifest.rs):**

```rust
pub struct DerivationSourceManifest {
    pub blocks: Vec<BlockManifest>,
}

pub struct BlockManifest {
    pub timestamp: u64,
    pub coinbase: Address,
    pub anchor_block_number: u64,
    pub gas_limit: u64,
    pub transactions: Bytes,  // Not Vec<TransactionSigned>
}
```

**Differences:**

1. Legacy has `prover_auth_bytes` field, raiko2 doesn't
2. Legacy uses `Vec<TransactionSigned>`, raiko2 uses `Bytes`

**Action Required:** Add `prover_auth_bytes` field to raiko2's `DerivationSourceManifest`.

---

### 4. LibHash Functions Missing ❌

Legacy has comprehensive hashing functions in `lib/src/libhash.rs`:

- `hash_proposal`
- `hash_derivation`
- `hash_core_state`
- `hash_checkpoint`
- `hash_transition_with_metadata`
- `hash_derivation_source`
- `hash_blob_slice`
- `hash_public_input`

**Raiko2 Status:** Missing - no equivalent module exists.

**Action Required:** Port hashing functions to `raiko2-protocol` or `raiko2-primitives`.

---

### 5. Protocol Instance Missing ❌

Legacy has `ProtocolInstance` in `lib/src/protocol_instance.rs` for:

- Block metadata fork handling
- Transition hash calculation
- Public input generation

**Raiko2 Status:** No equivalent - `raiko2-primitives` only has basic types.

**Action Required:** Port `ProtocolInstance` logic to raiko2.

---

### 6. Input Types Comparison

| Type                    | Legacy                                          | Raiko2                               | Match         |
| ----------------------- | ----------------------------------------------- | ------------------------------------ | ------------- |
| `GuestInput`            | Complex with block, chain_spec, parent info     | Simplified with witnesses + manifest | ⚠️ Different  |
| `GuestBatchInput`       | Contains Vec<GuestInput> + TaikoGuestBatchInput | Not implemented                      | ❌ Missing    |
| `TaikoProverData`       | graffiti, actual_prover, designated_prover, etc | Only prover, graffiti                | ⚠️ Incomplete |
| `AggregationGuestInput` | Has proofs, chain_id, verifier_address          | Only proofs                          | ⚠️ Incomplete |

---

### 7. Shasta Aggregation Types

**Legacy (lib/src/input.rs):**

```rust
pub struct ShastaAggregationGuestInput {
    pub proofs: Vec<Proof>,
    pub chain_id: u64,
    pub verifier_address: Address,
}

pub struct ShastaRisc0AggregationGuestInput {
    pub image_id: [u32; 8],
    pub block_inputs: Vec<B256>,
    pub chain_id: u64,
    pub verifier_address: Address,
    pub prover_address: Address,
}

pub struct ShastaSp1AggregationGuestInput {
    pub image_id: [u32; 8],
    pub block_inputs: Vec<B256>,
    pub chain_id: u64,
    pub verifier_address: Address,
    pub prover_address: Address,
}
```

**Raiko2 Status:** Only has basic `AggregationGuestInput` and `ZkAggregationGuestInput`.

**Action Required:** Add Shasta-specific aggregation types.

---

## Summary of Required Changes

### High Priority (Core Functionality)

1. **Fix DerivationSource structure** - Use `is_forced_inclusion: bool` instead of `flags: u8`
2. **Fix codec decode order** - Match legacy byte-by-byte decode sequence
3. **Add missing manifest field** - Include `prover_auth_bytes` in DerivationSourceManifest
4. **Port LibHash functions** - Essential for proof verification

### Medium Priority (Feature Completeness)

5. **Port ProtocolInstance** - Required for proper public input generation
6. **Add Shasta aggregation types** - Required for proof aggregation
7. **Complete TaikoProverData** - Add designated_prover, checkpoint, etc.

### Low Priority (Can Be Deferred)

8. **Align GuestInput structure** - Consider whether simplified structure is intentional
9. **Add GuestBatchInput** - Needed for batch proving mode

---

## Recommendations

1. **Immediate Fix**: Update `DerivationSource` and codec to match legacy exactly
2. **Create raiko2-hash crate or module**: Port all LibHash functions
3. **Review GuestInput design**: Decide if simplified structure is intentional or needs alignment
4. **Document differences**: If intentional simplifications exist, document the rationale

---

## Next Steps

1. Update `raiko2/crates/protocol/src/shasta/types.rs`:

   - Change `flags: u8` to `is_forced_inclusion: bool`

2. Update `raiko2/crates/protocol/src/shasta/codec.rs`:

   - Fix decode order for DerivationSource

3. Update `raiko2/crates/protocol/src/shasta/manifest.rs`:

   - Add `prover_auth_bytes` field

4. Create new file `raiko2/crates/protocol/src/shasta/hash.rs`:

   - Port LibHash functions from legacy

5. Update `raiko2/crates/primitives/src/input.rs`:
   - Add Shasta aggregation input types
   - Complete TaikoProverData fields
