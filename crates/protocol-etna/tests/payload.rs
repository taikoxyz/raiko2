//! Etna blob payload verification against real committed execution projections.

use alloy_consensus::{Header, TxEnvelope, proofs::calculate_transaction_root};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::{Address, B256, U256, hex};
use etna_core::{
    comet::{
        APP_VERSION,
        codec::decode_canonical_block,
        verify::{data_hash, header_hash},
    },
    domain::{DeploymentDomain, chain_id},
};
use raiko2_protocol_etna::{
    payload::{PayloadError, verify_decompressed_execution_payload},
    projection::{ExecutionProjection, ProjectionContext, verify_execution_projection},
};

fn list(items: &[&[u8]]) -> Vec<u8> {
    let mut encoded = Vec::new();
    alloy_rlp::Header {
        list: true,
        payload_length: items.iter().map(|item| item.len()).sum(),
    }
    .encode(&mut encoded);
    for item in items {
        encoded.extend_from_slice(item);
    }
    encoded
}

fn signed_transaction() -> TxEnvelope {
    let raw = hex::decode(include_str!("fixtures/projection-legacy.hex").trim()).unwrap();
    TxEnvelope::decode_2718_exact(&raw).unwrap()
}

struct Fixture {
    comet_header: tendermint::block::Header,
    comet_transactions: [Vec<u8>; 1],
    execution_header: Header,
    execution_item: Vec<u8>,
    context: ProjectionContext,
}

impl Fixture {
    fn new(transactions: &[TxEnvelope], timestamp: u64) -> Self {
        let domain = DeploymentDomain {
            l1_chain_id: U256::from(1),
            l2_chain_id: U256::from(167_001),
            inbox: Address::repeat_byte(0x11),
            generation: 2,
        };
        let parent_el_hash = B256::repeat_byte(0x20);
        let execution_header = Header {
            number: 18,
            parent_hash: parent_el_hash,
            extra_data: hex!("6400000000000200000001020300000000000000000000000000000000").into(),
            timestamp,
            transactions_root: calculate_transaction_root(transactions),
            ..Default::default()
        };
        let encoded_transactions: Vec<_> = transactions
            .iter()
            .map(|tx| alloy_rlp::encode(tx.encoded_2718().as_slice()))
            .collect();
        let tx_items: Vec<_> = encoded_transactions.iter().map(Vec::as_slice).collect();
        let execution_item = list(&[&alloy_rlp::encode(&execution_header), &list(&tx_items)]);
        let mut envelope = vec![6];
        envelope.extend(list(&[&execution_item, &[0xc0], &[0xc0], &[0xc0]]));
        let comet_transactions = [envelope];

        let raw = hex::decode(include_str!("fixtures/first-block.hex").trim_end()).unwrap();
        let (_, block) = decode_canonical_block(&raw).unwrap();
        let mut comet_header = block.header;
        comet_header.chain_id = chain_id(&domain).unwrap().parse().unwrap();
        comet_header.height = 18u64.try_into().unwrap();
        comet_header.version.app = APP_VERSION;
        comet_header.last_block_id = None;
        comet_header.data_hash = Some(data_hash(&comet_transactions));
        let context = ProjectionContext {
            domain,
            origin_height: 17,
            height: 18,
            expected_comet_hash: B256::from_slice(header_hash(&comet_header).as_bytes()),
            parent_comet_hash: B256::ZERO,
            parent_el_hash,
            max_envelope_bytes: 130_000,
        };
        Self {
            comet_header,
            comet_transactions,
            execution_header,
            execution_item,
            context,
        }
    }

    fn project(&self, transactions: &[TxEnvelope]) -> ExecutionProjection<'_> {
        verify_execution_projection(
            &self.comet_header,
            &self.comet_transactions,
            &self.execution_header,
            transactions,
            &self.context,
        )
        .unwrap()
    }
}

#[test]
fn accepts_empty_transaction_block() {
    let fixture = Fixture::new(&[], 1);
    let range = [fixture.project(&[])];
    let decoded = list(&[&fixture.execution_item]);

    verify_decompressed_execution_payload(&decoded, &range).unwrap();
}

#[test]
fn accepts_ordered_items_with_a_signed_transaction() {
    let first = Fixture::new(&[], 1);
    let transactions = [signed_transaction()];
    let second = Fixture::new(&transactions, 2);
    let range = [first.project(&[]), second.project(&transactions)];
    let decoded = list(&[&first.execution_item, &second.execution_item]);

    verify_decompressed_execution_payload(&decoded, &range).unwrap();
}

#[test]
fn rejects_substituted_reordered_duplicate_missing_and_extra_items() {
    let first = Fixture::new(&[], 1);
    let second = Fixture::new(&[], 2);
    let third = Fixture::new(&[], 3);
    let range = [first.project(&[]), second.project(&[])];
    let (a, b, c) = (
        first.execution_item.as_slice(),
        second.execution_item.as_slice(),
        third.execution_item.as_slice(),
    );

    for (label, items) in [
        ("substituted", vec![a, c]),
        ("reordered", vec![b, a]),
        ("duplicate", vec![a, a]),
        ("missing", vec![a]),
        ("extra", vec![a, b, c]),
    ] {
        assert!(
            matches!(
                verify_decompressed_execution_payload(&list(&items), &range),
                Err(PayloadError::PayloadMismatch)
            ),
            "{label} execution item"
        );
    }
}

#[test]
fn rejects_noncanonical_truncated_or_extra_rlp() {
    let fixture = Fixture::new(&[], 1);
    let range = [fixture.project(&[])];
    let canonical = list(&[&fixture.execution_item]);

    let wrapped = alloy_rlp::encode(fixture.execution_item.as_slice());
    let mut nonminimal = canonical.clone();
    // Insert a zero as the most significant byte of the long list length.
    assert!(nonminimal[0] >= 0xf8);
    nonminimal[0] += 1;
    nonminimal.insert(1, 0);
    let mut extra_root = canonical.clone();
    extra_root.push(0xc0);

    let wrapped = list(&[&wrapped]);
    for (label, decoded) in [
        ("wrapped item", wrapped.as_slice()),
        ("nonminimal list length", &nonminimal),
        ("truncated prefix", &canonical[..1]),
        ("truncated item", &canonical[..canonical.len() - 1]),
        ("extra root", &extra_root),
    ] {
        assert!(
            matches!(
                verify_decompressed_execution_payload(decoded, &range),
                Err(PayloadError::PayloadMismatch)
            ),
            "{label}"
        );
    }
}

#[test]
fn rejects_empty_range_even_for_canonical_empty_list() {
    assert!(matches!(
        verify_decompressed_execution_payload(&[0xc0], &[]),
        Err(PayloadError::EmptyRange)
    ));
}

fn sized_item_fixture(size: usize, probe_input_size: usize) -> (Fixture, [TxEnvelope; 1]) {
    let TxEnvelope::Legacy(mut transaction) = signed_transaction() else {
        unreachable!()
    };
    // Keep the probe and the final item in the same RLP length classes.
    // Projection binds these encoded transaction bytes to the envelope.
    transaction.tx_mut().input = vec![0xa7; probe_input_size].into();
    let probe = Fixture::new(&[TxEnvelope::Legacy(transaction.clone())], 1);
    let input_size = size + probe_input_size - probe.execution_item.len();
    transaction.tx_mut().input = vec![0xa7; input_size].into();

    let transactions = [TxEnvelope::Legacy(transaction)];
    let fixture = Fixture::new(&transactions, 1);
    assert_eq!(fixture.execution_item.len(), size);
    (fixture, transactions)
}

#[test]
fn enforces_inclusive_decompressed_cap_with_outer_prefix() {
    const MAX_BYTES: usize = 64 * 1024 * 1024;
    const MAX_ITEM_BYTES: usize = 124_000;
    const PREFIX: [u8; 5] = [0xfb, 0x03, 0xff, 0xff, 0xfb];

    let payload_length = MAX_BYTES - PREFIX.len();
    let full_items = payload_length / MAX_ITEM_BYTES;
    let tail_length = payload_length % MAX_ITEM_BYTES;
    let (full, full_txs) = sized_item_fixture(MAX_ITEM_BYTES, 120_000);
    let (tail, tail_txs) = sized_item_fixture(tail_length, 24_000);

    let mut range: Vec<_> = (0..full_items).map(|_| full.project(&full_txs)).collect();
    range.push(tail.project(&tail_txs));
    let mut decoded = Vec::with_capacity(MAX_BYTES + 1);
    decoded.extend_from_slice(&PREFIX);
    for _ in 0..full_items {
        decoded.extend_from_slice(&full.execution_item);
    }
    decoded.extend_from_slice(&tail.execution_item);
    assert_eq!(decoded.len(), MAX_BYTES);
    verify_decompressed_execution_payload(&decoded, &range).unwrap();

    decoded.push(0);
    assert!(matches!(
        verify_decompressed_execution_payload(&decoded, &range),
        Err(PayloadError::DecompressedSize)
    ));

    let (longer_tail, longer_txs) = sized_item_fixture(tail_length + 1, 24_000);
    *range.last_mut().unwrap() = longer_tail.project(&longer_txs);
    assert!(matches!(
        verify_decompressed_execution_payload(&[], &range),
        Err(PayloadError::DecompressedSize)
    ));
}
