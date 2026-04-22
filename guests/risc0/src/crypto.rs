//! Guest-specific crypto hooks for RISC Zero proofs.

use alloy_primitives::keccak256;
use revm_precompile::{install_crypto, Crypto, PrecompileHalt};

#[derive(Debug)]
pub struct Risc0GuestCrypto;

impl Crypto for Risc0GuestCrypto {
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
}

pub fn install_guest_crypto() {
    let _ = install_crypto(Risc0GuestCrypto);
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm_precompile::crypto;

    #[test]
    fn install_guest_crypto_registers_risc0_provider() {
        install_guest_crypto();

        assert_eq!(format!("{:?}", crypto()), "Risc0GuestCrypto");
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
