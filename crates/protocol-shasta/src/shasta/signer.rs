//! Deterministic secp256k1 signer with fixed-k signing support.

use alloy_primitives::{Address, Signature, U256, hex};
use k256::{
    AffinePoint, FieldBytes, ProjectivePoint, Scalar,
    elliptic_curve::{
        bigint::U256 as ScalarModulus, ff::PrimeField, ops::Reduce, point::AffineCoordinates,
        scalar::IsHigh, sec1::ToEncodedPoint,
    },
};
use thiserror::Error;

/// Golden touch testnet private key used by the protocol.
pub const GOLDEN_TOUCH_PRIVATE_KEY: &str =
    "0x92954368afd3caa1f3ce3ead0069c1af414054aefe1ef9aeacc1bf426222ce38";

/// Errors raised by [`FixedKSigner`].
#[derive(Debug, Error)]
pub enum FixedKSignerError {
    /// The provided private key is malformed or outside the curve order.
    #[error("invalid private key")]
    InvalidPrivateKey,
    /// Failed to invert the provided `k` value.
    #[error("non-invertible signing scalar")]
    NonInvertibleScalar,
    /// The deterministic signing attempt produced an invalid signature.
    #[error("invalid signature component")]
    ZeroSignatureComponent,
    /// All configured `k` candidates failed to produce a valid signature.
    #[error("unable to sign with provided k candidates")]
    SigningFailed,
}

/// Deterministic secp256k1 signer.
#[derive(Debug, Clone)]
pub struct FixedKSigner {
    secret_scalar: Scalar,
    address: Address,
}

impl FixedKSigner {
    /// Instantiate a signer from a hex-encoded private key.
    pub fn new(private_key_hex: &str) -> Result<Self, FixedKSignerError> {
        let trimmed = private_key_hex
            .strip_prefix("0x")
            .unwrap_or(private_key_hex);
        let bytes = hex::decode_to_array::<_, 32>(trimmed)
            .map_err(|_| FixedKSignerError::InvalidPrivateKey)?;
        let scalar = Option::<Scalar>::from(Scalar::from_repr(bytes.into()))
            .ok_or(FixedKSignerError::InvalidPrivateKey)?;
        if scalar.is_zero().into() {
            return Err(FixedKSignerError::InvalidPrivateKey);
        }

        Ok(Self {
            address: Self::derive_address(&scalar),
            secret_scalar: scalar,
        })
    }

    /// Convenience helper that instantiates the signer using the embedded golden-touch key.
    pub fn golden_touch() -> Result<Self, FixedKSignerError> {
        Self::new(GOLDEN_TOUCH_PRIVATE_KEY)
    }

    /// Returns the signer address derived from the private key.
    pub const fn address(&self) -> Address {
        self.address
    }

    /// Attempt to sign the provided digest using a fixed list of `k` candidates.
    pub fn sign_with_predefined_k(&self, hash: &[u8; 32]) -> Result<Signature, FixedKSignerError> {
        for candidate in [Scalar::ONE, Scalar::from(2_u64)] {
            if let Ok(signature) = self.sign_with_specific_k(candidate, hash) {
                return Ok(signature);
            }
        }
        Err(FixedKSignerError::SigningFailed)
    }

    fn sign_with_specific_k(
        &self,
        k: Scalar,
        hash: &[u8; 32],
    ) -> Result<Signature, FixedKSignerError> {
        let k_point: AffinePoint = (ProjectivePoint::GENERATOR * k).to_affine();
        let x_bytes = k_point.x();
        let y_is_odd = bool::from(k_point.y_is_odd());

        let raw_r = Scalar::from_repr(x_bytes);
        let overflow = !bool::from(raw_r.is_some());
        let r = raw_r.unwrap_or_else(|| <Scalar as Reduce<ScalarModulus>>::reduce_bytes(&x_bytes));
        let kinv =
            Option::<Scalar>::from(k.invert()).ok_or(FixedKSignerError::NonInvertibleScalar)?;

        let hash_bytes: FieldBytes = (*hash).into();
        let e = <Scalar as Reduce<ScalarModulus>>::reduce_bytes(&hash_bytes);
        let mut s = self.secret_scalar.mul(&r).add(&e).mul(&kinv);
        if s.is_zero().into() {
            return Err(FixedKSignerError::ZeroSignatureComponent);
        }

        let mut recovery_id = ((overflow as u8) << 1) | (y_is_odd as u8);
        if bool::from(s.is_high()) {
            s = -s;
            recovery_id ^= 0x01;
        }

        let r_bytes = r.to_bytes();
        let s_bytes = s.to_bytes();
        Ok(Signature::new(
            U256::from_be_slice(r_bytes.as_ref()),
            U256::from_be_slice(s_bytes.as_ref()),
            (recovery_id & 1) == 1,
        ))
    }

    fn derive_address(scalar: &Scalar) -> Address {
        let public_key = (ProjectivePoint::GENERATOR * scalar).to_affine();
        let encoded = public_key.to_encoded_point(false);
        Address::from_raw_public_key(&encoded.as_bytes()[1..])
    }
}
