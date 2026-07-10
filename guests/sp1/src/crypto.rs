//! Guest-specific crypto hooks for SP1 proofs.
//!
//! Cycle-sensitive paths must use SP1-patched crates (or kzg-rs on the SP1 BLS
//! backend). Do not reimplement BN254 / BLS / KZG in pure software here — that
//! bypasses precompiles and inflates prove cost.

use alloy_primitives::keccak256;
use kzg_rs::{get_kzg_settings, Bytes32, Bytes48, KzgProof};
use revm_precompile::{install_crypto, Crypto, PrecompileHalt};

#[derive(Debug)]
pub struct Sp1GuestCrypto;

impl Crypto for Sp1GuestCrypto {
    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        use sha2::Digest;

        sha2::Sha256::digest(input).into()
    }

    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        mut recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], PrecompileHalt> {
        use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

        let mut sig = Signature::from_slice(sig).map_err(|_| {
            PrecompileHalt::other_static("patched k256 deserialize signature failed")
        })?;
        if let Some(sig_normalized) = sig.normalize_s() {
            sig = sig_normalized;
            recid ^= 1;
        }

        let recid = RecoveryId::from_byte(recid)
            .ok_or_else(|| PrecompileHalt::other_static("invalid recovery ID"))?;
        let recovered_key = VerifyingKey::recover_from_prehash(msg, &sig, recid)
            .map_err(|_| PrecompileHalt::Secp256k1RecoverFailed)?;

        let mut hash = keccak256(&recovered_key.to_encoded_point(false).as_bytes()[1..]);
        hash[..12].fill(0);
        Ok(*hash)
    }

    /// Route EIP-4844 point evaluation through kzg-rs so SP1's patched
    /// `bls12_381` backend is used instead of revm's arkworks fallback.
    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), PrecompileHalt> {
        verify_kzg_proof_with_kzg_rs(z, y, commitment, proof)
    }

    // bn254_g1_add / bn254_g1_mul / bn254_pairing_check intentionally use the
    // trait defaults → revm substrate-bn backend, which is Cargo-patched to
    // sp1-patches/bn and hits SP1 BN254 syscalls.
}

fn verify_kzg_proof_with_kzg_rs(
    z: &[u8; 32],
    y: &[u8; 32],
    commitment: &[u8; 48],
    proof: &[u8; 48],
) -> Result<(), PrecompileHalt> {
    let commitment = Bytes48::from_slice(commitment)
        .map_err(|_| PrecompileHalt::BlobVerifyKzgProofFailed)?;
    let z = Bytes32::from_slice(z).map_err(|_| PrecompileHalt::BlobVerifyKzgProofFailed)?;
    let y = Bytes32::from_slice(y).map_err(|_| PrecompileHalt::BlobVerifyKzgProofFailed)?;
    let proof =
        Bytes48::from_slice(proof).map_err(|_| PrecompileHalt::BlobVerifyKzgProofFailed)?;
    let settings = get_kzg_settings();
    let ok = KzgProof::verify_kzg_proof(&commitment, &z, &y, &proof, &settings)
        .map_err(|_| PrecompileHalt::BlobVerifyKzgProofFailed)?;
    if !ok {
        return Err(PrecompileHalt::BlobVerifyKzgProofFailed);
    }
    Ok(())
}

pub fn install_guest_crypto() {
    let _ = install_crypto(Sp1GuestCrypto);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;
    use revm_precompile::crypto;

    #[test]
    fn install_guest_crypto_registers_sp1_provider() {
        install_guest_crypto();

        assert_eq!(format!("{:?}", crypto()), "Sp1GuestCrypto");
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

        // BN254 goes through trait defaults → patched substrate-bn.
        let p1 = [
            0x18, 0xb1, 0x8a, 0xcf, 0xb4, 0xc2, 0xc3, 0x02, 0x76, 0xdb, 0x54, 0x11, 0x36, 0x8e,
            0x71, 0x85, 0xb3, 0x11, 0xdd, 0x12, 0x46, 0x91, 0x61, 0x0c, 0x5d, 0x3b, 0x74, 0x03,
            0x4e, 0x09, 0x3d, 0xc9, 0x06, 0x3c, 0x90, 0x9c, 0x47, 0x20, 0x84, 0x0c, 0xb5, 0x13,
            0x4c, 0xb9, 0xf5, 0x9f, 0xa7, 0x49, 0x75, 0x57, 0x96, 0x81, 0x96, 0x58, 0xd3, 0x2e,
            0xfc, 0x0d, 0x28, 0x81, 0x98, 0xf3, 0x72, 0x66,
        ];
        let p2 = [
            0x07, 0xc2, 0xb7, 0xf5, 0x8a, 0x84, 0xbd, 0x61, 0x45, 0xf0, 0x0c, 0x9c, 0x2b, 0xc0,
            0xbb, 0x1a, 0x18, 0x7f, 0x20, 0xff, 0x2c, 0x92, 0x96, 0x3a, 0x88, 0x01, 0x9e, 0x7c,
            0x6a, 0x01, 0x4e, 0xed, 0x06, 0x61, 0x4e, 0x20, 0xc1, 0x47, 0xe9, 0x40, 0xf2, 0xd7,
            0x0d, 0xa3, 0xf7, 0x4c, 0x9a, 0x17, 0xdf, 0x36, 0x17, 0x06, 0xa4, 0x48, 0x5c, 0x74,
            0x2b, 0xd6, 0x78, 0x84, 0x78, 0xfa, 0x17, 0xd7,
        ];
        let expected_add = hex!(
            "2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703\
             301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c915"
        );
        // EVM always feeds 32-byte big-endian scalars (revm pads inputs).
        let mut scalar_one = [0u8; 32];
        scalar_one[31] = 1;
        let mut scalar_two = [0u8; 32];
        scalar_two[31] = 2;

        assert_eq!(crypto().bn254_g1_add(&p1, &p2).unwrap(), expected_add);
        assert!(crypto().bn254_g1_mul(&p1, &scalar_one).is_ok());
        assert_eq!(crypto().bn254_g1_add(&p1, &[0u8; 64]).unwrap(), p1);
        assert_eq!(
            crypto().bn254_g1_add(&[0u8; 64], &[0u8; 64]).unwrap(),
            [0u8; 64]
        );
        assert_eq!(crypto().bn254_g1_mul(&p1, &[0u8; 32]).unwrap(), [0u8; 64]);
        assert_eq!(
            crypto().bn254_g1_mul(&[0u8; 64], &scalar_two).unwrap(),
            [0u8; 64]
        );
        assert!(crypto().bn254_g1_add(&[0x11u8; 64], &[0u8; 64]).is_err());
        assert!(crypto().bn254_g1_add(&[0xffu8; 64], &[0u8; 64]).is_err());

        let doubled = crypto()
            .bn254_g1_add(&p1, &p1)
            .expect("point addition should succeed");
        assert_eq!(
            crypto()
                .bn254_g1_mul(&p1, &scalar_two)
                .expect("point multiplication should succeed"),
            doubled
        );

        // EIP-4844 point-evaluation vector (revm-precompile test case).
        let commitment = hex!("8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7");
        let z = hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000");
        let y = hex!("1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9");
        let proof = hex!("a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c");
        assert!(crypto()
            .verify_kzg_proof(&z, &y, &commitment, &proof)
            .is_ok());
    }
}
