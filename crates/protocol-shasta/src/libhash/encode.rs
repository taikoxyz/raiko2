use alloy_primitives::{Address, B256, U256, b256};

// Helper to encode a u48 (Rust u64 is fine, always left-padded in Solidity as uint256)
#[must_use]
pub fn u48_to_b256(val: u64) -> B256 {
    // Truncate to 48 bits
    let val = val & 0xffff_ffff_ffff;
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
