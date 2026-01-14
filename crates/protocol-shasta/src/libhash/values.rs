use alloy_primitives::{B256, keccak256};

/// Returns `keccak256(abi.encode(value0, value1))`.
///
/// This is equivalent to Solidity's `EfficientHashLib.hash`.
///
/// Equivalent Solidity assembly:
/// ```text
/// assembly {
///     let m := mload(0x40)
///     mstore(m, v0)
///     mstore(add(m, 0x20), v1)
///     result := keccak256(m, 0x40)
/// }
/// ```
pub fn hash_two_values(value0: B256, value1: B256) -> B256 {
    hash_values_impl(&[value0, value1])
}

/// Returns `keccak256(abi.encode(value0, value1, value2))`
pub fn hash_three_values(value0: B256, value1: B256, value2: B256) -> B256 {
    hash_values_impl(&[value0, value1, value2])
}

pub fn hash_four_values(value0: B256, value1: B256, value2: B256, value3: B256) -> B256 {
    hash_values_impl(&[value0, value1, value2, value3])
}

pub fn hash_five_values(
    value0: B256,
    value1: B256,
    value2: B256,
    value3: B256,
    value4: B256,
) -> B256 {
    hash_values_impl(&[value0, value1, value2, value3, value4])
}

pub fn hash_six_values(
    value0: B256,
    value1: B256,
    value2: B256,
    value3: B256,
    value4: B256,
    value5: B256,
) -> B256 {
    hash_values_impl(&[value0, value1, value2, value3, value4, value5])
}

pub(crate) fn hash_values_impl(values: &[B256]) -> B256 {
    let mut data = Vec::with_capacity(values.len() * 32);
    for v in values {
        data.extend_from_slice(v.as_slice());
    }
    keccak256(&data)
}
