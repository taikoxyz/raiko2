//! Guest-specific crypto hooks for SP1 proofs.

use alloy_primitives::keccak256;
use num_bigint::BigUint;
use revm_precompile::{install_crypto, Crypto, PrecompileHalt};
use std::sync::OnceLock;

const BN254_BASE_MODULUS_DEC: &[u8] =
    b"21888242871839275222246405745257275088696311157297823662689037894645226208583";
const BN254_GROUP_ORDER_DEC: &[u8] =
    b"21888242871839275222246405745257275088548364400416034343698204186575808495617";
const BN254_B: u8 = 3;

#[derive(Clone, Debug)]
struct Bn254Affine {
    x: BigUint,
    y: BigUint,
}

impl Bn254Affine {
    fn new(x: BigUint, y: BigUint) -> Self {
        Self { x, y }
    }
}

#[derive(Debug)]
pub struct Sp1GuestCrypto;

impl Crypto for Sp1GuestCrypto {
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        let point = be_bytes_to_point(p1)?;
        let other = be_bytes_to_point(p2)?;
        point_to_be_bytes(g1_add(&point, &other))
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], PrecompileHalt> {
        let point = be_bytes_to_point(point)?;
        let scalar = BigUint::from_bytes_be(scalar) % bn254_group_order();
        point_to_be_bytes(g1_mul(&point, &scalar))
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

#[derive(Clone, Debug)]
enum Bn254G1 {
    Infinity,
    Affine(Bn254Affine),
}

fn bn254_base_modulus() -> &'static BigUint {
    static MODULUS: OnceLock<BigUint> = OnceLock::new();
    MODULUS.get_or_init(|| {
        BigUint::parse_bytes(BN254_BASE_MODULUS_DEC, 10).expect("valid BN254 base modulus")
    })
}

fn bn254_group_order() -> &'static BigUint {
    static ORDER: OnceLock<BigUint> = OnceLock::new();
    ORDER.get_or_init(|| {
        BigUint::parse_bytes(BN254_GROUP_ORDER_DEC, 10).expect("valid BN254 group order")
    })
}

fn be_bytes_to_point(input: &[u8]) -> Result<Bn254G1, PrecompileHalt> {
    if input.len() != 64 {
        return Err(PrecompileHalt::Bn254AffineGFailedToCreate);
    }

    let x = BigUint::from_bytes_be(&input[..32]);
    let y = BigUint::from_bytes_be(&input[32..]);
    if x == BigUint::default() && y == BigUint::default() {
        return Ok(Bn254G1::Infinity);
    }

    let modulus = bn254_base_modulus();
    if &x >= modulus || &y >= modulus {
        return Err(PrecompileHalt::Bn254FieldPointNotAMember);
    }

    let point = Bn254Affine::new(x, y);
    if !is_on_bn254_g1(&point) {
        return Err(PrecompileHalt::Bn254AffineGFailedToCreate);
    }

    Ok(Bn254G1::Affine(point))
}

fn point_to_be_bytes(point: Bn254G1) -> Result<[u8; 64], PrecompileHalt> {
    let Bn254G1::Affine(point) = point else {
        return Ok([0u8; 64]);
    };

    let x_bytes = point.x.to_bytes_be();
    let y_bytes = point.y.to_bytes_be();
    if x_bytes.len() > 32 || y_bytes.len() > 32 {
        return Err(PrecompileHalt::Bn254AffineGFailedToCreate);
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

fn is_on_bn254_g1(point: &Bn254Affine) -> bool {
    let modulus = bn254_base_modulus();
    let y_squared = (&point.y * &point.y) % modulus;
    let x_squared = (&point.x * &point.x) % modulus;
    let x_cubed = (&x_squared * &point.x) % modulus;
    let rhs = (x_cubed + BigUint::from(BN254_B)) % modulus;
    y_squared == rhs
}

fn g1_add(left: &Bn254G1, right: &Bn254G1) -> Bn254G1 {
    match (left, right) {
        (Bn254G1::Infinity, _) => right.clone(),
        (_, Bn254G1::Infinity) => left.clone(),
        (Bn254G1::Affine(left), Bn254G1::Affine(right)) => add_affine_points(left, right),
    }
}

fn add_affine_points(left: &Bn254Affine, right: &Bn254Affine) -> Bn254G1 {
    let modulus = bn254_base_modulus();
    if left.x == right.x {
        if (&left.y + &right.y) % modulus == BigUint::default() {
            return Bn254G1::Infinity;
        }

        return double_affine_point(left);
    }

    let numerator = mod_sub(&right.y, &left.y, modulus);
    let denominator = mod_sub(&right.x, &left.x, modulus);
    let slope = (&numerator * mod_inverse(&denominator, modulus)) % modulus;
    let x = mod_sub(
        &mod_sub(&(&slope * &slope), &left.x, modulus),
        &right.x,
        modulus,
    );
    let y = mod_sub(&(&slope * mod_sub(&left.x, &x, modulus)), &left.y, modulus);

    Bn254G1::Affine(Bn254Affine::new(x, y))
}

fn double_affine_point(point: &Bn254Affine) -> Bn254G1 {
    let modulus = bn254_base_modulus();
    if point.y == BigUint::default() {
        return Bn254G1::Infinity;
    }

    let numerator = (BigUint::from(3u8) * &point.x * &point.x) % modulus;
    let denominator = (BigUint::from(2u8) * &point.y) % modulus;
    let slope = (numerator * mod_inverse(&denominator, modulus)) % modulus;
    let x = mod_sub(
        &mod_sub(&(&slope * &slope), &point.x, modulus),
        &point.x,
        modulus,
    );
    let y = mod_sub(
        &(&slope * mod_sub(&point.x, &x, modulus)),
        &point.y,
        modulus,
    );

    Bn254G1::Affine(Bn254Affine::new(x, y))
}

fn g1_mul(point: &Bn254G1, scalar: &BigUint) -> Bn254G1 {
    if *scalar == BigUint::default() || matches!(point, Bn254G1::Infinity) {
        return Bn254G1::Infinity;
    }

    let mut result = Bn254G1::Infinity;
    let mut addend = point.clone();
    for bit in 0..scalar.bits() {
        if scalar.bit(bit) {
            result = g1_add(&result, &addend);
        }
        addend = g1_add(&addend, &addend);
    }
    result
}

fn mod_inverse(value: &BigUint, modulus: &BigUint) -> BigUint {
    value.modpow(&(modulus - BigUint::from(2u8)), modulus)
}

fn mod_sub(left: &BigUint, right: &BigUint, modulus: &BigUint) -> BigUint {
    let left = left % modulus;
    let right = right % modulus;
    if left >= right {
        left - right
    } else {
        modulus - (right - left)
    }
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
        assert_eq!(crypto().bn254_g1_add(&p1, &p2).unwrap(), expected_add);
        assert!(crypto().bn254_g1_mul(&p1, &[1]).is_ok());
        assert_eq!(crypto().bn254_g1_add(&p1, &[0u8; 64]).unwrap(), p1);
        assert_eq!(
            crypto().bn254_g1_add(&[0u8; 64], &[0u8; 64]).unwrap(),
            [0u8; 64]
        );
        assert_eq!(crypto().bn254_g1_mul(&p1, &[0u8; 32]).unwrap(), [0u8; 64]);
        assert_eq!(crypto().bn254_g1_mul(&[0u8; 64], &[2]).unwrap(), [0u8; 64]);
        assert_eq!(
            crypto().bn254_g1_add(&[0x11u8; 64], &[0u8; 64]),
            Err(PrecompileHalt::Bn254AffineGFailedToCreate)
        );
        assert_eq!(
            crypto().bn254_g1_add(&[0xffu8; 64], &[0u8; 64]),
            Err(PrecompileHalt::Bn254FieldPointNotAMember)
        );

        let doubled = crypto()
            .bn254_g1_add(&p1, &p1)
            .expect("point addition should succeed");
        let mut scalar_two = [0u8; 32];
        scalar_two[31] = 2;
        assert_eq!(
            crypto()
                .bn254_g1_mul(&p1, &scalar_two)
                .expect("point multiplication should succeed"),
            doubled
        );
    }
}
