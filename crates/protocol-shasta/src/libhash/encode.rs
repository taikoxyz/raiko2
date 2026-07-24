use alloy_primitives::{Address, B256, U256, b256};

/// Encode a Solidity `uint48` value (carried as `u64`) as a left-padded 32-byte word.
///
/// # Panics
///
/// Panics when `val` does not fit in 48 bits. In guest context a panic halts proving (fail
/// closed); callers are expected to pre-validate with `fits_shasta_uint48`-style guards so
/// out-of-range values surface as validation errors instead.
#[must_use]
pub fn u48_to_b256(val: u64) -> B256 {
    assert!(
        val <= 0xffff_ffff_ffff,
        "value {val} does not fit in uint48"
    );
    u64_to_b256(val)
}

// Helper to encode a u48 (Rust u64 is fine, always left-padded in Solidity as uint256)
#[must_use]
pub fn u64_to_b256(val: u64) -> B256 {
    U256::from(val).into()
}

/// Convert an Address to B256 by zero-padding (equivalent to bytes32(uint256(uint160(address))))
#[must_use]
pub fn address_to_b256(address: Address) -> B256 {
    B256::left_padding_from(address.as_slice())
}

pub(crate) const EMPTY_BYTES_HASH: B256 =
    b256!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");

pub const VERIFY_PROOF_B256: B256 =
    b256!("5645524946595f50524f4f460000000000000000000000000000000000000000");

#[cfg(test)]
mod tests {
    use super::{u48_to_b256, u64_to_b256};

    #[test]
    fn u48_to_b256_accepts_values_up_to_uint48_max() {
        const UINT48_MAX: u64 = (1_u64 << 48) - 1;

        assert_eq!(u48_to_b256(0), u64_to_b256(0));
        assert_eq!(u48_to_b256(UINT48_MAX), u64_to_b256(UINT48_MAX));
    }

    #[test]
    #[should_panic(expected = "does not fit in uint48")]
    fn u48_to_b256_panics_instead_of_truncating_wider_values() {
        // 1 << 48 would silently alias to 0 under the old masking behavior.
        let _ = u48_to_b256(1_u64 << 48);
    }
}
