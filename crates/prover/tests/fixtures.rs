#![allow(missing_docs)]
#![allow(clippy::redundant_pub_crate)]

pub(crate) fn risc0_receipt_json() -> String {
    use risc0_zkvm::{Digest, FakeReceipt, Receipt, ReceiptClaim};

    let claim = ReceiptClaim::ok(Digest::ZERO, vec![0u8]);
    let fake = FakeReceipt::new(claim);
    let receipt: Receipt = fake.try_into().expect("convert FakeReceipt into Receipt");
    serde_json::to_string(&receipt).expect("serialize Receipt to JSON")
}

#[allow(dead_code)]
pub(crate) fn risc0_receipt_image_id_hex() -> String {
    alloy_primitives::hex::encode_prefixed(risc0_zkvm::Digest::ZERO.as_bytes())
}

#[test]
fn fixture_receipt_json_is_non_empty() {
    assert!(!risc0_receipt_json().is_empty());
}
