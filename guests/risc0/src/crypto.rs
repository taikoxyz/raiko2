//! Guest-specific crypto hooks for RISC Zero proofs.
//!
//! Cycle-sensitive paths use `risc0-crypto-evm` and Cargo-patched crates.
//!
//! - BN254 / ecrecover / modexp / p256: `risc0-crypto-evm` (pairing via trait default → substrate-bn).
//! - EVM BLS12-381 (EIP-2537, `0x0b..=0x11`): revm-precompile `blst` + official `risc0/blst` patch
//!   (no Crypto overrides; trait defaults call the blst crypto_backend).
//! - EVM `0x0a` point evaluation: revm trait default selects **blst** when the `blst` feature is
//!   enabled (`c-kzg` > `blst` > arkworks). We do **not** enable `c-kzg` and do **not** override
//!   with kzg-rs — lab KZG vectors via kzg-rs were far more expensive than revm's path.
//! - Blob proof-of-equivalence (proposal path) uses kzg-rs in primitives with the
//!   crates-io `bls12_381` backend (kzg-rs `standard` feature). Do not enable the
//!   risc0/zkcrypto-bls12_381 crates-io patch: zkVM non-Montgomery acceleration
//!   false-negatives some valid PoE verifies (see risc0/zkcrypto-bls12_381#2).

use revm_precompile::{install_crypto, Crypto, PrecompileHalt};

#[derive(Debug)]
pub struct Risc0GuestCrypto;

impl Crypto for Risc0GuestCrypto {
    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        risc0_crypto_evm::sha256(input)
    }

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        risc0_crypto_evm::bn254_g1_add(p1, p2).ok_or(PrecompileHalt::Bn254AffineGFailedToCreate)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        risc0_crypto_evm::bn254_g1_mul(point, scalar)
            .ok_or(PrecompileHalt::Bn254AffineGFailedToCreate)
    }

    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], PrecompileHalt> {
        let address = risc0_crypto_evm::secp256k1_ecrecover(sig, recid, msg)
            .ok_or(PrecompileHalt::Secp256k1RecoverFailed)?;
        let mut output = [0u8; 32];
        output[12..].copy_from_slice(address.as_slice());
        Ok(output)
    }

    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, PrecompileHalt> {
        Ok(risc0_crypto_evm::modexp(base, exp, modulus)
            .unwrap_or_else(|| fallback_modexp(base, exp, modulus)))
    }

    fn secp256r1_verify_signature(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        risc0_crypto_evm::secp256r1_verify(msg, sig, pk)
    }

    // bn254_pairing_check uses the trait default → patched substrate-bn.
    // verify_kzg_proof uses the trait default → revm blst KZG (feature = blst) — see module docs.
}

fn fallback_modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    use num_bigint::BigUint;

    if modulus.is_empty() {
        return Vec::new();
    }

    let modulus_value = BigUint::from_bytes_be(modulus);
    if modulus_value == BigUint::default() {
        return vec![0u8; modulus.len()];
    }

    let result = BigUint::from_bytes_be(base).modpow(&BigUint::from_bytes_be(exp), &modulus_value);
    let result_bytes = result.to_bytes_be();
    if result_bytes.len() >= modulus.len() {
        return result_bytes[result_bytes.len() - modulus.len()..].to_vec();
    }

    let mut output = vec![0u8; modulus.len()];
    output[modulus.len() - result_bytes.len()..].copy_from_slice(&result_bytes);
    output
}

pub fn install_guest_crypto() {
    let _ = install_crypto(Risc0GuestCrypto);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;
    use revm_precompile::crypto;

    #[test]
    fn install_guest_crypto_registers_risc0_provider() {
        install_guest_crypto();

        assert_eq!(format!("{:?}", crypto()), "Risc0GuestCrypto");

        // Host: trait-default revm blst point evaluation (EVM 0x0a with features=blst).
        let commitment = hex!("8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7");
        let z = hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000");
        let y = hex!("1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9");
        let proof = hex!("a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c");
        assert!(crypto()
            .verify_kzg_proof(&z, &y, &commitment, &proof)
            .is_ok());

        if !cfg!(target_os = "zkvm") {
            return;
        }

        assert_eq!(
            crypto().sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );

        let sig = [
            0xb5, 0x0b, 0xb6, 0x79, 0x5f, 0x31, 0x74, 0x8a, 0x4d, 0x37, 0xc3, 0xa9, 0x7e, 0xbd,
            0x06, 0xa2, 0x2e, 0xa3, 0x37, 0x71, 0x04, 0x0f, 0x5c, 0x05, 0xd6, 0xe2, 0xbb, 0x2d,
            0x38, 0xc6, 0x22, 0x7c, 0x34, 0x3b, 0x66, 0x59, 0xdb, 0x96, 0x99, 0x59, 0xd9, 0xfd,
            0xdb, 0x44, 0xbd, 0x0d, 0xd9, 0xb9, 0xdd, 0x47, 0x66, 0x6a, 0xb5, 0x28, 0x71, 0x90,
            0x1d, 0x17, 0x61, 0xeb, 0x82, 0xec, 0x87, 0x22,
        ];
        let msg = [
            0x6b, 0x6f, 0x6f, 0x74, 0x68, 0x65, 0x6e, 0x65, 0x76, 0x65, 0x72, 0x67, 0x6f, 0x6e,
            0x6e, 0x61, 0x67, 0x69, 0x76, 0x65, 0x79, 0x6f, 0x75, 0x72, 0x6d, 0x69, 0x6e, 0x64,
            0x6f, 0x6e, 0x6e, 0x61,
        ];
        assert!(crypto().secp256k1_ecrecover(&sig, 0, &msg).is_ok());
    }
}
