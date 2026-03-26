#![allow(missing_docs)]
#![allow(dead_code)]
#![allow(clippy::redundant_pub_crate)]

pub(crate) fn risc0_receipt_json() -> String {
    use risc0_zkvm::{Digest, FakeReceipt, Receipt, ReceiptClaim};

    let claim = ReceiptClaim::ok(Digest::ZERO, vec![0u8]);
    let fake = FakeReceipt::new(claim);
    let receipt: Receipt = fake.try_into().expect("convert FakeReceipt into Receipt");
    serde_json::to_string(&receipt).expect("serialize Receipt to JSON")
}
