use crate::crypto::install_guest_crypto;
use raiko2_primitives::PrecompileLabInput;
use revm_precompile::{
    blake2, bls12_381, bn254, hash, identity, kzg_point_evaluation, modexp, secp256k1,
};

pub fn execute_precompile(input: &PrecompileLabInput) -> u64 {
    install_guest_crypto();
    assert_eq!(
        input.input.len(),
        usize::try_from(input.input_size).expect("input size too large"),
        "precompile input length does not match input_size"
    );

    let mut accumulator = 0u64;
    for _ in 0..input.target_count {
        let gas_limit = input.target_raw_gas;
        match input.address {
            0x01 => fold_precompile_output(
                &mut accumulator,
                secp256k1::ec_recover_run(&input.input, gas_limit).expect("ecrecover failed"),
            ),
            0x02 => fold_precompile_output(
                &mut accumulator,
                hash::sha256_run(&input.input, gas_limit).expect("sha256 failed"),
            ),
            0x03 => fold_precompile_output(
                &mut accumulator,
                hash::ripemd160_run(&input.input, gas_limit).expect("ripemd160 failed"),
            ),
            0x04 => fold_precompile_output(
                &mut accumulator,
                identity::identity_run(&input.input, gas_limit).expect("identity failed"),
            ),
            0x05 => fold_precompile_output(
                &mut accumulator,
                modexp::osaka_run(&input.input, gas_limit).expect("modexp failed"),
            ),
            0x06 => fold_precompile_output(
                &mut accumulator,
                bn254::run_add(&input.input, bn254::add::ISTANBUL_ADD_GAS_COST, gas_limit)
                    .expect("bn254 add failed"),
            ),
            0x07 => fold_precompile_output(
                &mut accumulator,
                bn254::run_mul(&input.input, bn254::mul::ISTANBUL_MUL_GAS_COST, gas_limit)
                    .expect("bn254 mul failed"),
            ),
            0x08 => fold_precompile_output(
                &mut accumulator,
                bn254::run_pair(
                    &input.input,
                    bn254::pair::ISTANBUL_PAIR_PER_POINT,
                    bn254::pair::ISTANBUL_PAIR_BASE,
                    gas_limit,
                )
                .expect("bn254 pairing failed"),
            ),
            0x09 => fold_precompile_output(
                &mut accumulator,
                blake2::run(&input.input, gas_limit).expect("blake2f failed"),
            ),
            0x0A => fold_precompile_output(
                &mut accumulator,
                kzg_point_evaluation::run(&input.input, gas_limit).expect("kzg point eval failed"),
            ),
            0x0B => fold_precompile_output(
                &mut accumulator,
                bls12_381::g1_add::g1_add(&input.input, gas_limit).expect("bls12 g1add failed"),
            ),
            0x0C => fold_precompile_output(
                &mut accumulator,
                bls12_381::g1_msm::g1_msm(&input.input, gas_limit).expect("bls12 g1msm failed"),
            ),
            0x0E => fold_precompile_output(
                &mut accumulator,
                bls12_381::g2_add::g2_add(&input.input, gas_limit).expect("bls12 g2add failed"),
            ),
            0x0F => fold_precompile_output(
                &mut accumulator,
                bls12_381::g2_msm::g2_msm(&input.input, gas_limit).expect("bls12 g2msm failed"),
            ),
            0x11 => fold_precompile_output(
                &mut accumulator,
                bls12_381::pairing::pairing(&input.input, gas_limit)
                    .expect("bls12 pairing failed"),
            ),
            0x12 => fold_precompile_output(
                &mut accumulator,
                bls12_381::map_fp_to_g1::map_fp_to_g1(&input.input, gas_limit)
                    .expect("bls12 map fp to g1 failed"),
            ),
            0x13 => fold_precompile_output(
                &mut accumulator,
                bls12_381::map_fp2_to_g2::map_fp2_to_g2(&input.input, gas_limit)
                    .expect("bls12 map fp2 to g2 failed"),
            ),
            address => panic!("unsupported precompile address 0x{address:02x}"),
        }
    }
    accumulator
}

fn fold_precompile_output(
    accumulator: &mut u64,
    output: revm_precompile::EthPrecompileOutput,
) {
    fold_bytes(accumulator, &output.gas_used.to_be_bytes());
    fold_bytes(accumulator, &output.bytes);
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

    #[test]
    fn executes_all_direct_body_precompiles() {
        for (case, address, target_raw_gas, input) in [
            ("ecrecover", 0x01, 3_000, ecrecover_input()),
            ("sha256", 0x02, 72, vec![0u8; 32]),
            ("ripemd160", 0x03, 720, vec![0u8; 32]),
            ("identity", 0x04, 18, vec![0u8; 32]),
            ("modexp", 0x05, 500, modexp_input()),
            ("bn128_add", 0x06, 150, vec![0u8; 128]),
            ("bn128_mul", 0x07, 6_000, vec![0u8; 96]),
            ("bn128_pairing", 0x08, 79_000, vec![0u8; 192]),
            ("blake2f", 0x09, 12, blake2f_input()),
            ("point_evaluation", 0x0a, 50_000, kzg_input()),
            ("bls12_g1add", 0x0b, 375, vec![0u8; 256]),
            ("bls12_g1msm", 0x0c, 12_000, vec![0u8; 160]),
            ("bls12_g2add", 0x0e, 600, vec![0u8; 512]),
            ("bls12_g2msm", 0x0f, 22_500, vec![0u8; 288]),
            ("bls12_pairing", 0x11, 70_300, vec![0u8; 384]),
            ("bls12_map_fp_to_g1", 0x12, 5_500, vec![0u8; 64]),
            ("bls12_map_fp2_to_g2", 0x13, 23_800, vec![0u8; 128]),
        ] {
            let input = PrecompileLabInput {
                case: case.to_string(),
                scenario: "precompile".to_string(),
                address,
                target_count: 1,
                input_size: input.len() as u64,
                target_raw_gas,
                input,
            };

            execute_precompile(&input);
        }
    }

    fn ecrecover_input() -> Vec<u8> {
        let mut input = Vec::new();
        input.extend([
            0x6b, 0x6f, 0x6f, 0x74, 0x68, 0x65, 0x6e, 0x65, 0x76, 0x65, 0x72, 0x67, 0x6f,
            0x6e, 0x6e, 0x61, 0x67, 0x69, 0x76, 0x65, 0x79, 0x6f, 0x75, 0x72, 0x6d, 0x69,
            0x6e, 0x64, 0x6f, 0x6e, 0x6e, 0x61,
        ]);
        input.extend([0u8; 31]);
        input.push(27);
        input.extend([
            0xb5, 0x0b, 0xb6, 0x79, 0x5f, 0x31, 0x74, 0x8a, 0x4d, 0x37, 0xc3, 0xa9, 0x7e,
            0xbd, 0x06, 0xa2, 0x2e, 0xa3, 0x37, 0x71, 0x04, 0x0f, 0x5c, 0x05, 0xd6, 0xe2,
            0xbb, 0x2d, 0x38, 0xc6, 0x22, 0x7c, 0x34, 0x3b, 0x66, 0x59, 0xdb, 0x96, 0x99,
            0x59, 0xd9, 0xfd, 0xdb, 0x44, 0xbd, 0x0d, 0xd9, 0xb9, 0xdd, 0x47, 0x66, 0x6a,
            0xb5, 0x28, 0x71, 0x90, 0x1d, 0x17, 0x61, 0xeb, 0x82, 0xec, 0x87, 0x22,
        ]);
        input
    }

    fn modexp_input() -> Vec<u8> {
        let mut input = Vec::new();
        for len in [1u8, 1, 1] {
            input.extend([0u8; 31]);
            input.push(len);
        }
        input.extend([2, 3, 5]);
        input
    }

    fn blake2f_input() -> Vec<u8> {
        let mut input = vec![0u8; 213];
        input[..4].copy_from_slice(&12u32.to_be_bytes());
        input[212] = 1;
        input
    }

    fn kzg_input() -> Vec<u8> {
        let mut input = Vec::new();
        input.extend(hex_bytes(
            "01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
        ));
        input.extend(hex_bytes(
            "73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000",
        ));
        input.extend(hex_bytes(
            "1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
        ));
        input.extend(hex_bytes(
            "8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
        ));
        input.extend(hex_bytes(
            "a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c",
        ));
        input
    }

    fn hex_bytes(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|idx| u8::from_str_radix(&value[idx..idx + 2], 16).expect("hex byte"))
            .collect()
    }
}
