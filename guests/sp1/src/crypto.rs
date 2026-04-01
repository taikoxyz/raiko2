//! Guest-specific crypto hooks for SP1 proofs.

use alloy_primitives::keccak256;
use num_bigint::BigUint;
use revm_precompile::{install_crypto, Crypto, PrecompileError};
use sp1_curves::{weierstrass::bn254::Bn254, AffinePoint};

#[derive(Debug)]
pub struct Sp1GuestCrypto;

impl Crypto for Sp1GuestCrypto {
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], PrecompileError> {
        let mut point = be_bytes_to_point(p1)?;
        let other = be_bytes_to_point(p2)?;
        point = point + other;
        point_to_be_bytes(point)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], PrecompileError> {
        let mut point = be_bytes_to_point(point)?;
        let scalar = BigUint::from_bytes_le(scalar);
        point = point.sw_scalar_mul(&scalar);
        point_to_be_bytes(point)
    }

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        use sha2::Digest;

        sha2::Sha256::digest(input).into()
    }

    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        mut recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], PrecompileError> {
        use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

        let mut sig = Signature::from_slice(sig).map_err(|_| {
            PrecompileError::other_static("patched k256 deserialize signature failed")
        })?;
        if let Some(sig_normalized) = sig.normalize_s() {
            sig = sig_normalized;
            recid ^= 1;
        }

        let recid = RecoveryId::from_byte(recid)
            .ok_or_else(|| PrecompileError::other_static("invalid recovery ID"))?;
        let recovered_key = VerifyingKey::recover_from_prehash(msg, &sig, recid)
            .map_err(|_| PrecompileError::Secp256k1RecoverFailed)?;

        let mut hash = keccak256(&recovered_key.to_encoded_point(false).as_bytes()[1..]);
        hash[..12].fill(0);
        Ok(*hash)
    }
}

fn be_bytes_to_point(input: &[u8]) -> Result<AffinePoint<Bn254>, PrecompileError> {
    if input.len() != 64 {
        return Err(PrecompileError::Bn254AffineGFailedToCreate);
    }

    let x = BigUint::from_bytes_be(&input[..32]);
    let y = BigUint::from_bytes_be(&input[32..]);
    Ok(AffinePoint::<Bn254>::new(x, y))
}

fn point_to_be_bytes(point: AffinePoint<Bn254>) -> Result<[u8; 64], PrecompileError> {
    let x_bytes = point.x.to_bytes_be();
    let y_bytes = point.y.to_bytes_be();
    if x_bytes.len() > 32 || y_bytes.len() > 32 {
        return Err(PrecompileError::Bn254AffineGFailedToCreate);
    }

    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x[32 - x_bytes.len()..].copy_from_slice(&x_bytes);
    y[32 - y_bytes.len()..].copy_from_slice(&y_bytes);

    Ok(([x, y])
        .concat()
        .try_into()
        .expect("fixed-size point bytes"))
}

pub fn install_guest_crypto() {
    let _ = install_crypto(Sp1GuestCrypto);
}
