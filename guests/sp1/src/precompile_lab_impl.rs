use raiko2_primitives::PrecompileLabInput;
use sha2::{Digest, Sha256};

pub fn execute_precompile(input: &PrecompileLabInput) -> u64 {
    assert_eq!(
        input.input.len(),
        usize::try_from(input.input_size).expect("input size too large"),
        "precompile input length does not match input_size"
    );

    let mut accumulator = 0u64;
    for _ in 0..input.target_count {
        match input.address {
            0x02 => {
                let digest = Sha256::digest(&input.input);
                fold_bytes(&mut accumulator, &digest);
            }
            0x04 => fold_bytes(&mut accumulator, &input.input),
            address => panic!("unsupported precompile address 0x{address:02x}"),
        }
    }
    accumulator
}

fn fold_bytes(accumulator: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *accumulator = accumulator
            .wrapping_mul(16_777_619)
            .wrapping_add(u64::from(*byte));
    }
}

#[cfg(test)]
mod tests {
    use super::execute_precompile;
    use raiko2_primitives::PrecompileLabInput;

    #[test]
    fn executes_identity_precompile() {
        let input = PrecompileLabInput {
            case: "identity".to_string(),
            scenario: "precompile".to_string(),
            address: 0x04,
            target_count: 2,
            input_size: 4,
            target_raw_gas: 18,
            input: vec![1, 2, 3, 4],
        };

        assert_ne!(execute_precompile(&input), 0);
    }

    #[test]
    fn executes_sha256_precompile() {
        let input = PrecompileLabInput {
            case: "sha256".to_string(),
            scenario: "precompile".to_string(),
            address: 0x02,
            target_count: 2,
            input_size: 4,
            target_raw_gas: 72,
            input: vec![1, 2, 3, 4],
        };

        assert_ne!(execute_precompile(&input), 0);
    }
}
