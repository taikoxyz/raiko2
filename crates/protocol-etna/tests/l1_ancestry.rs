//! Raw L1 header decoding and reference chain binding.

use alloy_consensus::Header as EthereumHeader;
use alloy_primitives::{B256, Bytes, keccak256};
use alloy_rlp::Header;
use raiko2_protocol_etna::l1::{
    L1AncestryContext, L1AncestryError, L1HeaderError, L1HeaderRef, verify_l1_ancestry,
};

fn positional_fields(parent: B256, state_root: B256, number: u64, timestamp: u64) -> Vec<Vec<u8>> {
    // A synthetic wire fixture for the four required positions.
    let mut fields = vec![vec![0x80]; 12];
    fields[0] = alloy_rlp::encode(parent);
    fields[3] = alloy_rlp::encode(state_root);
    fields[8] = alloy_rlp::encode(number);
    fields[11] = alloy_rlp::encode(timestamp);
    fields
}

fn encode_list(fields: &[Vec<u8>]) -> Bytes {
    let mut raw = Vec::new();
    Header {
        list: true,
        payload_length: fields.iter().map(Vec::len).sum(),
    }
    .encode(&mut raw);
    raw.extend(fields.iter().flatten().copied());
    raw.into()
}

fn context(reference: &[u8], reference_number: u64) -> L1AncestryContext {
    L1AncestryContext {
        reference_hash: keccak256(reference),
        reference_number,
        max_headers: 3,
        max_total_bytes: 8192,
    }
}

#[test]
fn authenticates_reference_and_returns_original_header() {
    let state_root = B256::repeat_byte(0x37);
    let raw = encode_list(&positional_fields(B256::ZERO, state_root, 42, 100));
    let mut context = context(&raw, 42);
    context.max_headers = 1;
    context.max_total_bytes = raw.len();

    let headers = [raw];
    let verified = verify_l1_ancestry(&headers, &context).expect("valid reference header");
    let reference = verified.header(42).expect("authenticated reference height");
    assert_eq!(reference.state_root(), state_root);
    assert!(core::ptr::eq(reference.raw(), headers[0].as_ref()));
}

#[test]
fn decodes_ethereum_headers_with_and_without_fork_fields() {
    let legacy = EthereumHeader {
        parent_hash: B256::repeat_byte(0x51),
        state_root: B256::repeat_byte(0x52),
        number: 0x0100_0000_0000_0000,
        timestamp: u64::MAX,
        ..EthereumHeader::default()
    };
    let prague = EthereumHeader {
        base_fee_per_gas: Some(7),
        withdrawals_root: Some(B256::repeat_byte(0x53)),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(B256::repeat_byte(0x54)),
        requests_hash: Some(B256::repeat_byte(0x55)),
        ..legacy.clone()
    };

    for header in [legacy, prague] {
        let raw = alloy_rlp::encode(&header);
        let decoded = L1HeaderRef::decode(&raw).expect("Ethereum header wire encoding");
        assert!(core::ptr::eq(decoded.raw(), raw.as_slice()));
        assert_eq!(decoded.hash(), keccak256(&raw));
        assert_eq!(decoded.parent_hash(), header.parent_hash);
        assert_eq!(decoded.state_root(), header.state_root);
        assert_eq!(decoded.number(), header.number);
        assert_eq!(decoded.timestamp(), header.timestamp);
    }
}

#[test]
fn hash_commits_to_opaque_fields() {
    let mut fields = positional_fields(B256::ZERO, B256::ZERO, 17, 100);
    // The item frames are valid. Their list payloads are deliberately not RLP items.
    fields[2] = vec![0xc1, 0xb8];
    fields.push(vec![0xc1, 0xf8]);
    let raw = encode_list(&fields);
    let original = L1HeaderRef::decode(&raw).expect("opaque field payloads");
    for (index, replacement) in [(2, vec![0x02]), (12, vec![0x03])] {
        let mut mutated_fields = fields.clone();
        mutated_fields[index] = replacement;
        let raw = encode_list(&mutated_fields);
        let mutated = L1HeaderRef::decode(&raw).expect("changed opaque field");
        assert_eq!(mutated.number(), original.number());
        assert_eq!(mutated.timestamp(), original.timestamp());
        assert_eq!(mutated.parent_hash(), original.parent_hash());
        assert_eq!(mutated.state_root(), original.state_root());
        assert_eq!(mutated.hash(), keccak256(&raw));
        assert_ne!(mutated.hash(), original.hash());
    }
}

#[test]
fn enforces_protocol_header_byte_cap() {
    let mut fields = positional_fields(B256::ZERO, B256::ZERO, 42, 100);
    // The outer frame takes three bytes. The final string frame takes three.
    let padding = 4096 - 6 - fields.iter().map(Vec::len).sum::<usize>();
    fields.push(alloy_rlp::encode(vec![0x92; padding].as_slice()));
    let exact = encode_list(&fields);
    assert_eq!(exact.len(), 4096);
    assert!(L1HeaderRef::decode(&exact).is_ok());

    *fields.last_mut().unwrap() = alloy_rlp::encode(vec![0x92; padding + 1].as_slice());
    let oversized = encode_list(&fields);
    assert_eq!(oversized.len(), 4097);
    assert_eq!(
        L1HeaderRef::decode(&oversized).unwrap_err(),
        L1HeaderError::Oversize(4097),
    );
}

#[test]
fn rejects_invalid_header_framing() {
    let fields = positional_fields(B256::ZERO, B256::ZERO, 42, 100);
    let raw = encode_list(&fields);
    let mut trailing = raw.to_vec();
    trailing.push(0x80);
    assert_eq!(
        L1HeaderRef::decode(&trailing).unwrap_err(),
        L1HeaderError::TrailingBytes(1),
    );
    assert_eq!(
        L1HeaderRef::decode(&encode_list(&fields[..11])).unwrap_err(),
        L1HeaderError::TooFewFields(11),
    );
    let mut malformed = fields;
    malformed.push(vec![0x81, 0x01]);
    for bytes in [
        Bytes::from_static(&[0x80]),
        Bytes::from_static(&[0xf8, 0x01, 0x80]),
        raw.slice(..raw.len() - 1),
        encode_list(&malformed),
    ] {
        assert!(matches!(
            L1HeaderRef::decode(&bytes),
            Err(L1HeaderError::Rlp(_))
        ));
    }
}

#[test]
fn rejects_malformed_required_fields() {
    let fields = positional_fields(B256::ZERO, B256::ZERO, 42, 100);
    for (index, item) in [
        (0, alloy_rlp::encode([0x51_u8; 31])),
        (3, alloy_rlp::encode([0x52_u8; 33])),
        (3, vec![0xc0]),
        (8, vec![0x00]),
        (8, vec![0x82, 0x00, 0x80]),
        (11, vec![0xc0]),
        (11, vec![0x89, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
    ] {
        let mut malformed = fields.clone();
        malformed[index] = item;
        assert!(matches!(
            L1HeaderRef::decode(&encode_list(&malformed)),
            Err(L1HeaderError::Field { index: actual, .. }) if actual == index
        ));
    }
}

fn chain() -> [Bytes; 3] {
    let oldest = encode_list(&positional_fields(
        B256::ZERO,
        B256::repeat_byte(0x33),
        40,
        30,
    ));
    let mut fields = positional_fields(keccak256(&oldest), B256::repeat_byte(0x32), 41, 29);
    fields.push(vec![0xc1, 0xf8]);
    let parent = encode_list(&fields);
    let reference = encode_list(&positional_fields(
        keccak256(&parent),
        B256::repeat_byte(0x31),
        42,
        31,
    ));
    [reference, parent, oldest]
}

#[test]
fn finds_only_covered_heights_on_the_reference_chain() {
    let headers = chain();
    let context = context(&headers[0], 42);
    let verified = verify_l1_ancestry(&headers, &context).expect("linked chain");
    assert_eq!(verified.headers().len(), 3);
    for (number, root) in [(42, 0x31), (41, 0x32), (40, 0x33)] {
        let header = verified.header(number).expect("covered height");
        assert_eq!(header.state_root(), B256::repeat_byte(root));
        assert_eq!(header.number(), number);
    }
    for uncovered in [0, 39, 43, u64::MAX] {
        assert!(verified.header(uncovered).is_none());
    }

    let prefix = verify_l1_ancestry(&headers[..2], &context).expect("valid ancestry prefix");
    assert!(prefix.header(40).is_none());
}

#[test]
fn rejects_changed_reference_even_with_same_height() {
    let headers = chain();
    let context = context(&headers[0], 42);
    let changed_root = encode_list(&positional_fields(
        keccak256(&headers[1]),
        B256::repeat_byte(0x62),
        42,
        31,
    ));
    assert!(matches!(
        verify_l1_ancestry(&[changed_root], &context),
        Err(L1AncestryError::HashMismatch { index: 0, .. })
    ));
    let wrong_height = L1AncestryContext {
        reference_number: 41,
        ..context
    };
    assert!(matches!(
        verify_l1_ancestry(&headers, &wrong_height),
        Err(L1AncestryError::NumberMismatch { index: 0, .. })
    ));
}

#[test]
fn rejects_nonmatching_parent_links() {
    let [reference, parent, oldest] = chain();
    let context = context(&reference, 42);
    let foreign = encode_list(&positional_fields(
        keccak256(&oldest),
        B256::repeat_byte(0x63),
        41,
        29,
    ));
    for wrong in [
        vec![reference.clone(), oldest.clone()],
        vec![reference.clone(), oldest, parent],
        vec![reference.clone(), reference.clone()],
        vec![reference, foreign],
    ] {
        assert!(matches!(
            verify_l1_ancestry(&wrong, &context),
            Err(L1AncestryError::HashMismatch { index: 1, .. })
        ));
    }
}

#[test]
fn rejects_a_height_gap_even_if_raw_links_match() {
    let parent = encode_list(&positional_fields(B256::ZERO, B256::ZERO, 40, 1));
    let reference = encode_list(&positional_fields(keccak256(&parent), B256::ZERO, 42, 2));
    let context = context(&reference, 42);
    assert_eq!(
        verify_l1_ancestry(&[reference, parent], &context).unwrap_err(),
        L1AncestryError::NumberMismatch {
            index: 1,
            expected: 41,
            actual: 40,
        }
    );
}

#[test]
fn rejects_witness_budgets_before_parsing_any_header() {
    let malformed = Bytes::from_static(&[0x80]);
    let context = context(&malformed, 0);
    assert_eq!(
        verify_l1_ancestry(&[], &context).unwrap_err(),
        L1AncestryError::Empty,
    );
    let headers = [malformed.clone(), Bytes::from(vec![0; 4097])];
    assert_eq!(
        verify_l1_ancestry(
            &headers,
            &L1AncestryContext {
                max_headers: 1,
                ..context
            },
        )
        .unwrap_err(),
        L1AncestryError::CountLimit { actual: 2, max: 1 },
    );
    assert_eq!(
        verify_l1_ancestry(&headers, &context).unwrap_err(),
        L1AncestryError::Header {
            index: 1,
            error: L1HeaderError::Oversize(4097),
        },
    );
    let headers = [malformed.clone(), malformed];
    assert_eq!(
        verify_l1_ancestry(
            &headers,
            &L1AncestryContext {
                max_total_bytes: 1,
                ..context
            },
        )
        .unwrap_err(),
        L1AncestryError::TotalBytesLimit { max: 1 },
    );
}

#[test]
fn permits_block_zero_but_rejects_an_extra_ancestor() {
    let wrapped = encode_list(&positional_fields(B256::ZERO, B256::ZERO, u64::MAX, 0));
    let genesis = encode_list(&positional_fields(keccak256(&wrapped), B256::ZERO, 0, 0));
    let context = context(&genesis, 0);
    let headers = [genesis, wrapped];
    let verified = verify_l1_ancestry(&headers[..1], &context).expect("block zero");
    assert!(verified.header(0).is_some());
    assert!(verified.header(u64::MAX).is_none());
    assert_eq!(
        verify_l1_ancestry(&headers, &context).unwrap_err(),
        L1AncestryError::BeforeGenesis { index: 1 },
    );
}
