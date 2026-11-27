//! Exercises Etna blob packing and V6 derivation.

use super::{
    decode_blob_payload, decode_execution_payload, verify_authenticated_blob_execution_payload,
    verify_blob_authentication, verify_blob_execution_payload,
    verify_blob_range_statement_projection, L1AnchorProof,
};
use alloy_consensus::{proofs::calculate_transaction_root, Header};
use alloy_eips::eip4844::{builder::SidecarBuilder, BYTES_PER_BLOB};
use alloy_primitives::{hex, Address, B256, U256};
use etna_core::{
    comet::{
        codec::decode_canonical_block,
        native::Block as CometBlock,
        verify::{data_hash, header_hash},
        APP_VERSION,
    },
    domain::{chain_id, DeploymentDomain},
};
use flate2::{write::ZlibEncoder, Compress, Compression, FlushCompress, Status};
use raiko2_primitives::{
    blob::util::{blob_to_commitment, blob_to_proof_of_equivalence, commitment_to_version_hash},
    RaikoError,
};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::{
    l1_history::{
        AnchorHeaderWitness, BeaconHeader, BeaconHistoryWitness, L1HistoryLimits, SszProofNode,
    },
    payload::PayloadError,
    projection::ExecutionProjection,
    range::{
        verify_range_projection, RangeBlock, RangeProjectionContext, RangeStatementProjectionError,
    },
    statement::{cut_leaf, LandInput, LandStatement},
};
use reth_ethereum_primitives::TransactionSigned;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read, Write},
    sync::OnceLock,
};
use taiko_client_protocol::shasta::BlobCoder;

fn pack(payload: &[u8]) -> Vec<Vec<u8>> {
    SidecarBuilder::<BlobCoder>::from_slice(payload)
        .take()
        .iter()
        .map(|blob| blob.as_slice().to_vec())
        .collect()
}

#[test]
fn decodes_single_blob_and_ordered_chunks() {
    let small = b"single Etna blob";
    let blobs = pack(small);
    assert_eq!(blobs.len(), 1);
    assert_eq!(decode_blob_payload(&blobs).unwrap(), small);

    let mut payload = vec![0xa7; 130_044];
    payload.extend_from_slice(b"partial final blob");
    let blobs = pack(&payload);
    assert_eq!(blobs.len(), 2);
    assert_eq!(decode_blob_payload(&blobs).unwrap(), payload);
}

#[test]
fn rejects_empty_and_seven_blob_collections() {
    assert!(decode_blob_payload(&[]).is_err());
    let full = pack(&vec![0xa7; 130_044]).pop().unwrap();
    let mut blobs = vec![full; 6];
    blobs.extend(pack(b"last chunk"));
    assert_eq!(blobs.len(), 7);

    assert!(decode_blob_payload(&blobs).is_err());
}

#[test]
fn rejects_short_and_long_raw_blobs() {
    for size in [BYTES_PER_BLOB - 1, BYTES_PER_BLOB + 1] {
        let mut blobs = pack(b"Etna payload");
        blobs[0].resize(size, 0);

        assert!(decode_blob_payload(&blobs).is_err());
    }
}

#[test]
fn rejects_reserved_bits_in_first_and_later_field_elements() {
    for offset in [0, 32] {
        let mut blobs = pack(&[0x55; 64]);
        blobs[0][offset] |= 0x40;

        assert!(decode_blob_payload(&blobs).is_err(), "header at {offset}");
    }
}

#[test]
fn rejects_nonzero_decoded_and_untouched_padding() {
    for offset in [6, 129] {
        let mut blobs = pack(&[0x55]);
        blobs[0][offset] = 1;

        assert!(decode_blob_payload(&blobs).is_err(), "padding at {offset}");
    }
}

#[test]
fn rejects_malformed_blob_version_and_length() {
    let mut wrong_version = pack(b"Etna payload");
    wrong_version[0][1] = 1;
    assert!(decode_blob_payload(&wrong_version).is_err());

    let mut wrong_length = pack(b"Etna payload");
    wrong_length[0][2..5].copy_from_slice(&[0x01, 0xfb, 0xfd]);
    assert!(decode_blob_payload(&wrong_length).is_err());
}

#[test]
fn rejects_short_nonfinal_blob() {
    let mut blobs = pack(b"short first chunk");
    blobs.extend(pack(b"final chunk"));

    assert!(decode_blob_payload(&blobs).is_err());
}

#[test]
fn rejects_empty_final_chunk() {
    let empty = vec![0; BYTES_PER_BLOB];
    assert!(decode_blob_payload(core::slice::from_ref(&empty)).is_err());

    let mut blobs = pack(&vec![0xa7; 130_044]);
    blobs.push(empty);
    assert!(decode_blob_payload(&blobs).is_err());
}

fn v6_payload(decoded: &[u8], compression: Compression) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(vec![0x06], compression);
    encoder.write_all(decoded).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn inflates_stored_and_dynamic_streams() {
    let decoded = b"Etna payload with a repeating pattern.".repeat(1000);
    let stored = v6_payload(&decoded, Compression::none());
    let dynamic = v6_payload(&decoded, Compression::default());
    assert_eq!((stored[3] >> 1) & 0b11, 0);
    assert_eq!((dynamic[3] >> 1) & 0b11, 2);

    let alternate = v6_payload(&decoded, Compression::best());
    assert_ne!(dynamic, alternate);
    for bytes in [stored, dynamic, alternate] {
        assert_eq!(decode_execution_payload(&bytes).unwrap(), decoded);
    }
}

#[test]
fn inflates_small_and_empty_zlib_outputs() {
    for data in [&[][..], &[0x41][..]] {
        let bytes = v6_payload(data, Compression::default());

        assert_eq!(decode_execution_payload(&bytes).unwrap(), data);
    }
}

#[test]
fn rejects_missing_or_wrong_payload_version() {
    let valid = v6_payload(b"Etna", Compression::default());
    let mut wrong_version = valid.clone();
    wrong_version[0] = 0x05;

    for bytes in [&[][..], &valid[1..], wrong_version.as_slice()] {
        assert!(decode_execution_payload(bytes).is_err());
    }
}

#[test]
fn checks_zlib_completion_and_checksum() {
    let valid = v6_payload(b"Etna payload", Compression::none());
    let mut corrupt = valid.clone();
    *corrupt.last_mut().unwrap() ^= 1;

    for bytes in [
        &valid[..1],
        &valid[..2],
        &valid[..valid.len() / 2],
        &valid[..valid.len() - 4],
        &valid[..valid.len() - 1],
        corrupt.as_slice(),
    ] {
        assert!(decode_execution_payload(bytes).is_err());
    }
}

#[test]
fn rejects_unused_compressed_input() {
    let valid = v6_payload(b"Etna payload", Compression::default());
    for suffix in [&[0u8][..], &valid[1..]] {
        let bytes = [valid.as_slice(), suffix].concat();

        assert!(decode_execution_payload(&bytes).is_err());
    }
}

#[test]
fn rejects_preset_dictionary_requests() {
    let dictionary = b"Etna shared dictionary";
    let mut compressor = Compress::new(Compression::default(), true);
    compressor.set_dictionary(dictionary).unwrap();
    let mut bytes = Vec::with_capacity(256);
    bytes.push(0x06);
    assert_eq!(
        compressor
            .compress_vec(dictionary, &mut bytes, FlushCompress::Finish)
            .unwrap(),
        Status::StreamEnd
    );
    assert_ne!(bytes[2] & 0x20, 0, "the test stream requests a dictionary");

    assert!(decode_execution_payload(&bytes).is_err());
}

fn compressed_repeated_bytes(count: usize) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(vec![0x06], Compression::fast());
    let count = u64::try_from(count).unwrap();
    assert_eq!(
        io::copy(&mut io::repeat(0x7e).take(count), &mut encoder).unwrap(),
        count
    );
    encoder.finish().unwrap()
}

#[test]
fn enforces_inclusive_decompression_limit() {
    const MAX_BYTES: usize = 64 * 1024 * 1024;
    let decoded = decode_execution_payload(&compressed_repeated_bytes(MAX_BYTES)).unwrap();
    assert_eq!(decoded.len(), MAX_BYTES);
    assert!(decoded.iter().all(|&byte| byte == 0x7e));
    drop(decoded);

    assert!(decode_execution_payload(&compressed_repeated_bytes(MAX_BYTES + 1)).is_err());
}

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

// Synthetic Fulu state/body openings. Each omitted sibling is a fixed zero commitment.
const PARENT_SLOT: u64 = 13_200_000;
const STATE_HELPERS: [SszProofNode; 7] = {
    let indices = [3, 5, 9, 17, 32, 135, 268];
    let mut result = [SszProofNode {
        index: 0,
        root: B256::ZERO,
    }; 7];
    let mut i = 0;
    while i < indices.len() {
        result[i].index = indices[i];
        i += 1;
    }
    result
};

fn test_tree(index: u64, leaves: &[(u64, B256)]) -> B256 {
    if let Some((_, root)) = leaves.iter().find(|(leaf, _)| *leaf == index) {
        return *root;
    }
    let contains = leaves.iter().any(|(leaf, _)| {
        leaf.ilog2() > index.ilog2() && leaf >> (leaf.ilog2() - index.ilog2()) == index
    });
    if !contains {
        return B256::ZERO;
    }
    let left = test_tree(index * 2, leaves);
    let right = test_tree(index * 2 + 1, leaves);
    B256::from(<[u8; 32]>::from(
        Sha256::new()
            .chain_update(left)
            .chain_update(right)
            .finalize(),
    ))
}

fn slot_root() -> B256 {
    let mut root = B256::ZERO;
    root[..8].copy_from_slice(&PARENT_SLOT.to_le_bytes());
    root
}

fn parent_beacon(raw: &[u8]) -> BeaconHeader {
    let mut version = B256::ZERO;
    version[0] = 6;
    BeaconHeader {
        slot: PARENT_SLOT,
        proposer_index: 0,
        parent_root: B256::ZERO,
        state_root: test_tree(1, &[(66, slot_root()), (269, version)]),
        body_root: test_tree(1, &[(812, alloy_primitives::keccak256(raw))]),
    }
}

fn bind_parent_beacon(headers: &mut [Vec<u8>]) {
    if let [parent, q] = headers {
        let beacon = parent_beacon(parent);
        let leaves = [
            slot_root(),
            B256::ZERO,
            B256::ZERO,
            beacon.state_root,
            beacon.body_root,
            B256::ZERO,
            B256::ZERO,
            B256::ZERO,
        ];
        let leaves: Vec<_> = (8..16).zip(leaves).collect();
        let root_field = alloy_rlp::encode(test_tree(1, &leaves));
        let mut cursor = q.as_slice();
        let alloy_rlp::PayloadView::List(mut fields) =
            alloy_rlp::Header::decode_raw(&mut cursor).unwrap()
        else {
            panic!()
        };
        fields[19] = &root_field;
        *q = list(&fields);
    }
}

fn raw_l1_header(number: u64, parent_hash: B256, state_root: B256) -> Vec<u8> {
    alloy_rlp::encode(Header {
        number,
        parent_hash,
        state_root,
        timestamp: 1_606_824_023 + (PARENT_SLOT + number - 18) * 12,
        base_fee_per_gas: Some(1),
        withdrawals_root: Some(B256::ZERO),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(B256::ZERO),
        ..Default::default()
    })
}

fn fixture_l1_headers(count: usize) -> Vec<Vec<u8>> {
    let mut l1_headers = Vec::new();
    let mut l1_parent = B256::ZERO;
    assert!((1..=2).contains(&count));
    for offset in 0..count {
        let number = 18 + u64::try_from(offset).unwrap();
        let raw = raw_l1_header(
            number,
            l1_parent,
            B256::left_padding_from(&number.to_be_bytes()),
        );
        l1_parent = alloy_primitives::keccak256(&raw);
        l1_headers.push(raw);
    }
    bind_parent_beacon(&mut l1_headers);
    l1_headers
}

struct BlockFixture {
    comet: CometBlock,
    execution: Header,
    transactions: Vec<TransactionSigned>,
    item: Vec<u8>,
    anchor_witness: Option<Vec<u8>>,
}

struct RangeFixture {
    parent: Header,
    blocks: Vec<BlockFixture>,
    context: RangeProjectionContext,
    l1_headers: Vec<Vec<u8>>,
}

impl RangeFixture {
    fn new(count: usize) -> Self {
        let stock_block = hex::decode(
            include_str!("../../../protocol-etna/tests/fixtures/first-block.hex").trim(),
        )
        .unwrap();
        let signed_wire = hex::decode(
            include_str!("../../../protocol-etna/tests/fixtures/projection-legacy.hex").trim(),
        )
        .unwrap();
        let domain = DeploymentDomain {
            l1_chain_id: U256::from(1),
            l2_chain_id: U256::from(167_001),
            inbox: Address::repeat_byte(0x11),
            generation: 2,
        };
        let parent = Header {
            number: 17,
            ..Default::default()
        };
        let parent_el_hash = parent.hash_slow();
        let mut el_hash = parent_el_hash;
        let mut previous_comet = None;
        let mut blocks = Vec::new();
        let l1_headers = fixture_l1_headers(count);
        for (index, raw_l1) in l1_headers.iter().enumerate().take(count) {
            let height = parent.number + u64::try_from(index).unwrap() + 1;
            let mut extra_data = hex!("64000000000002000000000000");
            extra_data[7..].copy_from_slice(&height.to_be_bytes()[2..]);

            // The fixture is a legacy transaction, whose network encoding is its original RLP.
            let (transactions, encoded_transactions) = if index == 0 {
                (Vec::new(), vec![0xc0])
            } else {
                let tx: TransactionSigned = alloy_rlp::decode_exact(&signed_wire).unwrap();
                let item = alloy_rlp::encode(signed_wire.as_slice());
                (vec![tx], list(&[&item]))
            };
            let execution = Header {
                number: height,
                parent_hash: el_hash,
                extra_data: extra_data.into(),
                transactions_root: calculate_transaction_root(&transactions),
                base_fee_per_gas: Some(1),
                withdrawals_root: Some(B256::repeat_byte(0x45)),
                blob_gas_used: Some(0),
                excess_blob_gas: Some(0),
                parent_beacon_block_root: Some(B256::left_padding_from(&height.to_be_bytes())),
                ..Default::default()
            };
            let item = list(&[&alloy_rlp::encode(&execution), &encoded_transactions]);
            let mut envelope = vec![0x06];
            // The range fixture exercises structural anchor carriage on each advance.
            let anchor_witness = list(&[&alloy_rlp::encode(raw_l1.as_slice()), &[0xc0]]);
            let optional_anchor = list(&[&anchor_witness]);
            envelope.extend(list(&[&item, &optional_anchor, &[0xc0], &[0xc0]]));

            let (_, mut comet) = decode_canonical_block(&stock_block).unwrap();
            comet.header.chain_id = chain_id(&domain).unwrap().parse().unwrap();
            comet.header.height = height.try_into().unwrap();
            comet.header.version.app = APP_VERSION;
            comet.header.last_block_id = previous_comet.map(|hash| {
                let mut id = comet.header.last_block_id.unwrap_or_default();
                id.hash = hash;
                id
            });
            comet.data = vec![envelope];
            comet.header.data_hash = Some(data_hash(&comet.data));
            previous_comet = Some(header_hash(&comet.header));
            el_hash = execution.hash_slow();
            blocks.push(BlockFixture {
                comet,
                execution,
                transactions,
                item,
                anchor_witness: Some(anchor_witness),
            });
        }

        Self {
            context: RangeProjectionContext {
                domain,
                origin_height: parent.number,
                parent_height: parent.number,
                last_height: blocks.last().unwrap().execution.number,
                parent_el_hash,
                parent_comet_hash: B256::ZERO,
                last_el_hash: el_hash,
                last_comet_hash: B256::from_slice(previous_comet.unwrap().as_bytes()),
                max_blocks: count,
                max_envelope_bytes: 130_000,
            },
            parent,
            blocks,
            l1_headers,
        }
    }

    fn openings<'a>(&self, headers: &'a [Vec<u8>]) -> Vec<AnchorHeaderWitness<'a>> {
        let q = self.l1_headers.last().unwrap();
        headers
            .iter()
            .map(|raw| {
                let is_parent = raw != q && self.l1_headers.len() == 2;
                AnchorHeaderWitness {
                    raw_header: raw,
                    beacon_header: is_parent.then(|| parent_beacon(&self.l1_headers[0])),
                    payload_siblings: if is_parent { &[B256::ZERO; 9] } else { &[] },
                }
            })
            .collect()
    }

    fn l1_proof<'a>(&'a self, anchors: &'a [AnchorHeaderWitness<'a>]) -> L1AnchorProof<'a> {
        L1AnchorProof {
            reference_header: self.l1_headers.last().unwrap(),
            anchors,
            beacon: anchors
                .iter()
                .find_map(|anchor| anchor.beacon_header)
                .map(|reference| BeaconHistoryWitness {
                    reference,
                    state_helpers: &STATE_HELPERS,
                    summary_length: None,
                    legacy_length: None,
                    periods: &[],
                }),
            limits: L1HistoryLimits {
                max_headers: 131_072,
                max_proof_nodes: 128,
            },
        }
    }

    fn reseal(&mut self) {
        let mut previous = None;
        for block in &mut self.blocks {
            let optional_anchor = block
                .anchor_witness
                .as_ref()
                .map_or_else(|| vec![0xc0], |raw| list(&[raw]));
            let mut envelope = vec![0x06];
            envelope.extend(list(&[&block.item, &optional_anchor, &[0xc0], &[0xc0]]));
            block.comet.data = vec![envelope];
            if let Some(hash) = previous {
                block.comet.header.last_block_id.as_mut().unwrap().hash = hash;
            }
            block.comet.header.data_hash = Some(data_hash(&block.comet.data));
            previous = Some(header_hash(&block.comet.header));
        }
        self.context.last_comet_hash = B256::from_slice(previous.unwrap().as_bytes());
    }

    fn inputs(&self) -> Vec<RangeBlock<'_, TransactionSigned>> {
        self.blocks
            .iter()
            .map(|block| RangeBlock {
                comet_header: &block.comet.header,
                comet_transactions: &block.comet.data,
                execution_header: &block.execution,
                execution_transactions: &block.transactions,
            })
            .collect()
    }

    fn project(&self) -> Vec<ExecutionProjection<'_>> {
        verify_range_projection(&self.inputs(), &self.context, &self.parent).unwrap()
    }

    fn cut_leaves(&self) -> Vec<B256> {
        self.blocks[..self.blocks.len() - 1]
            .iter()
            .map(|block| {
                cut_leaf(
                    U256::from(block.execution.number),
                    block.execution.hash_slow(),
                    B256::from_slice(header_hash(&block.comet.header).as_bytes()),
                    U256::ZERO,
                )
            })
            .collect()
    }

    fn statement<'a>(&self, hashes: &'a [B256], cut_leaves: &'a [B256]) -> LandStatement<'a> {
        let context = self.context;
        LandStatement {
            l1_chain_id: context.domain.l1_chain_id,
            l2_chain_id: context.domain.l2_chain_id,
            inbox: context.domain.inbox,
            registry: Address::repeat_byte(0x65),
            generation: U256::from(context.domain.generation),
            parent_hash: context.parent_el_hash,
            parent_comet_hash: context.parent_comet_hash,
            committee_config_hash: B256::repeat_byte(0x66),
            input: LandInput {
                parent_height: U256::from(context.parent_height),
                last_height: U256::from(context.last_height),
                last_block_hash: context.last_el_hash,
                last_comet_hash: context.last_comet_hash,
                last_state_root: self.blocks.last().unwrap().execution.state_root,
                l1_reference_block_number: U256::from(17 + self.l1_headers.len()),
                forced_inclusion_head: U256::ZERO,
                active_committee_snapshot: U256::from(50),
            },
            l1_reference_block_hash: alloy_primitives::keccak256(self.l1_headers.last().unwrap()),
            blob_hashes: hashes,
            forced_inclusion_head_before: U256::ZERO,
            cut_leaves,
        }
    }

    fn decoded_payload(&self) -> Vec<u8> {
        let items: Vec<_> = self
            .blocks
            .iter()
            .map(|block| block.item.as_slice())
            .collect();
        list(&items)
    }
}

#[test]
fn verifies_a_singleton_with_no_transactions() {
    let fixture = RangeFixture::new(1);
    let projected = fixture.project();
    assert!(projected[0].transaction_bytes().is_empty());
    let blobs = pack(&v6_payload(
        &fixture.decoded_payload(),
        Compression::default(),
    ));

    verify_blob_execution_payload(&blobs, &projected).unwrap();
}

#[test]
fn verifies_ordered_blocks_including_a_signed_transaction() {
    let fixture = RangeFixture::new(2);
    let projected = fixture.project();
    assert_eq!(projected[1].transaction_bytes().len(), 1);
    let decoded = fixture.decoded_payload();
    for compression in [Compression::none(), Compression::default()] {
        let blobs = pack(&v6_payload(&decoded, compression));

        verify_blob_execution_payload(&blobs, &projected).unwrap();
    }
}

#[test]
fn rejects_wrong_items_counts_and_framing_after_successful_blob_derivation() {
    let fixture = RangeFixture::new(2);
    let projected = fixture.project();
    let first = fixture.blocks[0].item.as_slice();
    let second = fixture.blocks[1].item.as_slice();
    let mut substituted = second.to_vec();
    *substituted.last_mut().unwrap() ^= 1;
    let wrapped = alloy_rlp::encode(second);
    let mut nonminimal = fixture.decoded_payload();
    nonminimal[0] += 1;
    nonminimal.insert(1, 0);
    let mut extra_root = fixture.decoded_payload();
    extra_root.push(0xc0);

    for (label, decoded) in [
        ("substituted item", list(&[first, &substituted])),
        ("reordered items", list(&[second, first])),
        ("duplicate item", list(&[first, first])),
        ("missing item", list(&[first])),
        ("extra item", list(&[first, second, second])),
        ("wrapped item", list(&[first, &wrapped])),
        ("nonminimal list prefix", nonminimal),
        ("extra root", extra_root),
    ] {
        let blobs = pack(&v6_payload(&decoded, Compression::default()));
        let error = verify_blob_execution_payload(&blobs, &projected).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<PayloadError>(),
                Some(PayloadError::PayloadMismatch)
            ),
            "{label}: {error:?}"
        );
    }
}

#[test]
fn rejects_empty_projected_range_after_successful_blob_derivation() {
    let blobs = pack(&v6_payload(&[0xc0], Compression::default()));
    let error = verify_blob_execution_payload(&blobs, &[]).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<PayloadError>(),
        Some(PayloadError::EmptyRange)
    ));
}

#[test]
fn propagates_inner_blob_and_compression_failures() {
    let fixture = RangeFixture::new(1);
    let projected = fixture.project();
    let payload = v6_payload(&fixture.decoded_payload(), Compression::default());

    let mut bad_blob = pack(&payload);
    bad_blob[0][0] |= 0x40;
    let mut bad_version = payload.clone();
    bad_version[0] = 0x05;
    let mut trailing = payload;
    trailing.push(0);

    for blobs in [bad_blob, pack(&bad_version), pack(&trailing)] {
        let error = verify_blob_execution_payload(&blobs, &projected).unwrap_err();
        assert!(
            error.downcast_ref::<PayloadError>().is_none(),
            "failure must precede execution-list equality: {error:?}"
        );
    }
}

fn prove_blobs(blobs: Vec<Vec<u8>>) -> (InputDataSource, Vec<B256>) {
    let mut source = InputDataSource {
        tx_data_from_blob: blobs,
        ..InputDataSource::default()
    };
    let mut hashes = Vec::new();
    for blob in &source.tx_data_from_blob {
        let commitment = blob_to_commitment(blob).unwrap();
        let proof = blob_to_proof_of_equivalence(blob, &commitment).unwrap();
        source.blob_commitments.push(commitment.to_vec());
        source.blob_proofs.push(proof.to_vec());
        hashes.push(commitment_to_version_hash(&commitment));
    }
    (source, hashes)
}

fn authenticated_blob_fixture() -> &'static (InputDataSource, Vec<B256>) {
    static FIXTURE: OnceLock<(InputDataSource, Vec<B256>)> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let mut first = vec![0; BYTES_PER_BLOB];
        first[31] = 7;
        let mut second = vec![0; BYTES_PER_BLOB];
        second[63] = 11;
        let fixture = prove_blobs(vec![first, second]);
        assert_ne!(fixture.1[0], fixture.1[1]);
        fixture
    })
}

#[test]
fn authenticates_single_and_multiple_ordered_blobs() {
    let (source, hashes) = authenticated_blob_fixture();
    let singleton = InputDataSource {
        tx_data_from_blob: source.tx_data_from_blob[..1].to_vec(),
        blob_commitments: source.blob_commitments[..1].to_vec(),
        blob_proofs: source.blob_proofs[..1].to_vec(),
        ..InputDataSource::default()
    };

    verify_blob_authentication(&singleton, &hashes[..1]).unwrap();
    verify_blob_authentication(source, hashes).unwrap();
}

#[test]
fn rejects_empty_and_oversized_kzg_batches() {
    assert!(verify_blob_authentication(&InputDataSource::default(), &[]).is_err());

    let (source, hashes) = authenticated_blob_fixture();
    let oversized = InputDataSource {
        tx_data_from_blob: vec![source.tx_data_from_blob[0].clone(); 7],
        blob_commitments: vec![source.blob_commitments[0].clone(); 7],
        blob_proofs: vec![source.blob_proofs[0].clone(); 7],
        ..InputDataSource::default()
    };
    assert!(verify_blob_authentication(&oversized, &[hashes[0]; 7]).is_err());
}

#[test]
fn rejects_each_kzg_collection_count_mismatch() {
    let (source, hashes) = authenticated_blob_fixture();
    let mut missing_blob = source.clone();
    missing_blob.tx_data_from_blob.pop();
    let mut missing_commitment = source.clone();
    missing_commitment.blob_commitments.pop();
    let mut missing_proof = source.clone();
    missing_proof.blob_proofs.pop();

    for malformed in [missing_blob, missing_commitment, missing_proof] {
        assert!(verify_blob_authentication(&malformed, hashes).is_err());
    }
    assert!(verify_blob_authentication(source, &hashes[..1]).is_err());
}

#[test]
fn rejects_legacy_inline_and_forced_source_fields() {
    let (source, hashes) = authenticated_blob_fixture();
    let mut inline = source.clone();
    inline.tx_data_from_calldata.push(0x06);
    let mut forced = source.clone();
    forced.is_forced_inclusion = true;

    assert!(verify_blob_authentication(&inline, hashes).is_err());
    assert!(verify_blob_authentication(&forced, hashes).is_err());
}

#[test]
fn requires_commitments_and_proofs_to_be_exactly_48_bytes() {
    let (source, hashes) = authenticated_blob_fixture();
    for width in [47, 49] {
        let mut bad_commitment_width = source.clone();
        bad_commitment_width.blob_commitments[0].resize(width, 0);
        assert!(verify_blob_authentication(&bad_commitment_width, hashes).is_err());

        let mut bad_proof_width = source.clone();
        bad_proof_width.blob_proofs[0].resize(width, 0);
        assert!(verify_blob_authentication(&bad_proof_width, hashes).is_err());
    }
}

#[test]
fn compares_versioned_hashes_by_position() {
    let (source, hashes) = authenticated_blob_fixture();
    let mut wrong_hash = hashes.clone();
    wrong_hash[0][1] ^= 1;
    assert!(verify_blob_authentication(source, &wrong_hash).is_err());

    let mut reversed_hashes = hashes.clone();
    reversed_hashes.reverse();
    assert!(verify_blob_authentication(source, &reversed_hashes).is_err());

    let mut reversed_source = source.clone();
    reversed_source.tx_data_from_blob.reverse();
    reversed_source.blob_commitments.reverse();
    reversed_source.blob_proofs.reverse();
    assert!(verify_blob_authentication(&reversed_source, hashes).is_err());
    verify_blob_authentication(&reversed_source, &reversed_hashes).unwrap();
}

#[test]
fn rejects_malformed_and_transplanted_proofs() {
    let (source, hashes) = authenticated_blob_fixture();
    let mut malformed = source.clone();
    malformed.blob_proofs[0].fill(0);
    assert!(verify_blob_authentication(&malformed, hashes).is_err());

    let mut swapped = source.clone();
    swapped.blob_proofs.swap(0, 1);
    assert!(verify_blob_authentication(&swapped, hashes).is_err());
}

#[test]
fn checks_the_actual_blob_and_commitment_relation() {
    let (source, hashes) = authenticated_blob_fixture();
    let mut modified = source.clone();
    modified.tx_data_from_blob[0][31] ^= 1;
    assert!(verify_blob_authentication(&modified, hashes).is_err());

    let mut wrong_commitment = source.clone();
    wrong_commitment.blob_commitments[0] = source.blob_commitments[1].clone();
    assert!(verify_blob_authentication(&wrong_commitment, &[hashes[1], hashes[1]]).is_err());
}

#[test]
fn rejects_invalid_raw_kzg_width_and_scalar_encoding() {
    let (source, hashes) = authenticated_blob_fixture();
    let mut short = source.clone();
    short.tx_data_from_blob[0].pop();
    assert!(verify_blob_authentication(&short, hashes).is_err());

    let mut bad_scalar = source.clone();
    bad_scalar.tx_data_from_blob[0][..32].fill(0xff);
    assert!(verify_blob_authentication(&bad_scalar, hashes).is_err());
}

struct AuthenticatedRangeFixture {
    range: RangeFixture,
    source: InputDataSource,
    hashes: Vec<B256>,
}

impl AuthenticatedRangeFixture {
    fn new(count: usize) -> Self {
        let range = RangeFixture::new(count);
        let (source, hashes) = prove_blobs(pack(&v6_payload(
            &range.decoded_payload(),
            Compression::default(),
        )));
        Self {
            range,
            source,
            hashes,
        }
    }

    fn verify_statement(
        &self,
        statement: &LandStatement<'_>,
        cursors: &[u64],
    ) -> anyhow::Result<(Vec<ExecutionProjection<'_>>, B256)> {
        verify_blob_range_statement_projection(
            &self.source,
            &self.range.inputs(),
            &self.range.context,
            statement,
            cursors,
            &self.range.parent,
            self.range
                .l1_proof(&self.range.openings(&self.range.l1_headers)),
        )
    }
}

fn authenticated_range_fixture() -> &'static AuthenticatedRangeFixture {
    static FIXTURE: OnceLock<AuthenticatedRangeFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| AuthenticatedRangeFixture::new(2))
}

#[test]
fn authenticates_and_derives_the_complete_projected_range() {
    let fixture = authenticated_range_fixture();
    let projected = fixture.range.project();
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[1].transaction_bytes().len(), 1);

    verify_authenticated_blob_execution_payload(&fixture.source, &fixture.hashes, &projected)
        .unwrap();
}

#[test]
fn rejects_corrupt_or_transplanted_proofs_for_the_same_execution_payload() {
    let fixture = authenticated_range_fixture();
    let projected = fixture.range.project();
    verify_blob_execution_payload(&fixture.source.tx_data_from_blob, &projected).unwrap();

    let mut corrupted = fixture.source.clone();
    corrupted.blob_proofs[0].fill(0);
    let mut transplanted = fixture.source.clone();
    transplanted.blob_proofs[0] = authenticated_blob_fixture().0.blob_proofs[0].clone();

    for source in [corrupted, transplanted] {
        let error =
            verify_authenticated_blob_execution_payload(&source, &fixture.hashes, &projected)
                .unwrap_err();
        assert!(error.downcast_ref::<RaikoError>().is_some(), "{error:?}");
    }
}

#[test]
fn rejects_tampered_blob_bytes_before_derivation() {
    let fixture = authenticated_range_fixture();
    let projected = fixture.range.project();
    let mut source = fixture.source.clone();
    source.tx_data_from_blob[0][6] ^= 1;

    let error = verify_authenticated_blob_execution_payload(&source, &fixture.hashes, &projected)
        .unwrap_err();
    assert!(error.downcast_ref::<RaikoError>().is_some(), "{error:?}");
}

#[test]
fn rejects_authenticated_payloads_with_substituted_or_reordered_items() {
    let fixture = authenticated_range_fixture();
    let projected = fixture.range.project();
    let first = fixture.range.blocks[0].item.as_slice();
    let second = fixture.range.blocks[1].item.as_slice();
    let mut substituted = second.to_vec();
    *substituted.last_mut().unwrap() ^= 1;

    for decoded in [list(&[first, &substituted]), list(&[second, first])] {
        let (source, hashes) = prove_blobs(pack(&v6_payload(&decoded, Compression::default())));
        verify_blob_authentication(&source, &hashes).unwrap();

        let error =
            verify_authenticated_blob_execution_payload(&source, &hashes, &projected).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<PayloadError>(),
                Some(PayloadError::PayloadMismatch)
            ),
            "{error:?}"
        );
    }
}

fn authenticated_singleton_fixture() -> &'static AuthenticatedRangeFixture {
    static FIXTURE: OnceLock<AuthenticatedRangeFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| AuthenticatedRangeFixture::new(1))
}

#[test]
fn binds_an_authenticated_singleton_to_its_statement() {
    let fixture = authenticated_singleton_fixture();
    let statement = fixture.range.statement(&fixture.hashes, &[]);
    let (projected, digest) = fixture.verify_statement(&statement, &[0]).unwrap();

    assert_eq!(digest, statement.try_digest().unwrap());
    assert_eq!(projected.len(), 1);
    assert_eq!(
        projected[0].execution_item_bytes(),
        fixture.range.blocks[0].item
    );
}

#[test]
fn binds_an_authenticated_range_with_an_interior_cut() {
    let fixture = authenticated_range_fixture();
    let leaves = fixture.range.cut_leaves();
    assert_eq!(leaves.len(), 1);
    let statement = fixture.range.statement(&fixture.hashes, &leaves);
    let (projected, digest) = fixture.verify_statement(&statement, &[0, 0]).unwrap();

    assert_eq!(digest, statement.try_digest().unwrap());
    assert_eq!(projected.len(), 2);
    for (projection, block) in projected.iter().zip(&fixture.range.blocks) {
        assert_eq!(projection.execution_item_bytes(), block.item);
    }
}

#[test]
fn rejects_statement_hash_values_and_counts_that_disagree_with_the_source() {
    let fixture = authenticated_singleton_fixture();
    let expected_hash = fixture.hashes[0];
    let mut wrong_hash = expected_hash;
    wrong_hash[1] ^= 1;
    let statement = fixture
        .range
        .statement(core::slice::from_ref(&wrong_hash), &[]);
    let error = fixture.verify_statement(&statement, &[0]).unwrap_err();
    assert!(
        error.to_string().contains("versioned hash mismatch"),
        "{error:?}"
    );

    for (hashes, message) in [
        (&[][..], "requires between 1 and 6 blobs"),
        (&[expected_hash; 2][..], "counts differ"),
        (&[expected_hash; 7][..], "requires between 1 and 6 blobs"),
    ] {
        let statement = fixture.range.statement(hashes, &[]);
        let error = fixture.verify_statement(&statement, &[0]).unwrap_err();
        assert!(error.to_string().contains(message), "{error:?}");
    }
}

#[test]
fn rejects_statement_hash_reordering_against_individually_authentic_blobs() {
    let fixture = authenticated_singleton_fixture();
    let (source, hashes) = authenticated_blob_fixture();
    verify_blob_authentication(source, hashes).unwrap();
    let reversed = [hashes[1], hashes[0]];
    let statement = fixture.range.statement(&reversed, &[]);
    let error = verify_blob_range_statement_projection(
        source,
        &fixture.range.inputs(),
        &fixture.range.context,
        &statement,
        &[0],
        &fixture.range.parent,
        fixture
            .range
            .l1_proof(&fixture.range.openings(&fixture.range.l1_headers)),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("blob 0 versioned hash mismatch"),
        "{error:?}"
    );
}

#[test]
fn propagates_origin_and_terminal_statement_mismatches() {
    type MutateStatement = fn(&mut LandStatement<'_>);
    let fixture = authenticated_range_fixture();
    let leaves = fixture.range.cut_leaves();
    let mutations: [(&str, MutateStatement); 3] = [
        ("parent_hash", |statement| {
            statement.parent_hash = B256::ZERO;
        }),
        ("last_block_hash", |statement| {
            statement.input.last_block_hash = B256::ZERO;
        }),
        ("last_state_root", |statement| {
            statement.input.last_state_root = B256::repeat_byte(0x6a);
        }),
    ];
    for (expected, mutate) in mutations {
        let mut statement = fixture.range.statement(&fixture.hashes, &leaves);
        mutate(&mut statement);
        let error = fixture.verify_statement(&statement, &[0, 0]).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<RangeStatementProjectionError>(),
                Some(RangeStatementProjectionError::StatementMismatch { field }) if *field == expected
            ),
            "{expected}: {error:?}"
        );
    }
}

#[test]
fn propagates_invalid_cursor_shape_and_cut_leaf() {
    let fixture = authenticated_range_fixture();
    let leaves = fixture.range.cut_leaves();
    let statement = fixture.range.statement(&fixture.hashes, &leaves);
    let error = fixture.verify_statement(&statement, &[0, 2]).unwrap_err();
    assert!(
        matches!(
            error.downcast_ref::<RangeStatementProjectionError>(),
            Some(RangeStatementProjectionError::CursorStep { height: 19, .. })
        ),
        "{error:?}"
    );

    let statement = fixture.range.statement(&fixture.hashes, &[B256::ZERO]);
    let error = fixture.verify_statement(&statement, &[0, 0]).unwrap_err();
    assert!(
        matches!(
            error.downcast_ref::<RangeStatementProjectionError>(),
            Some(RangeStatementProjectionError::CutLeaf { height: 18 })
        ),
        "{error:?}"
    );
}

#[test]
fn rejects_a_statement_whose_authentic_blobs_derive_a_different_range() {
    let fixture = authenticated_range_fixture();
    let singleton = authenticated_singleton_fixture();
    let leaves = fixture.range.cut_leaves();
    let statement = fixture.range.statement(&singleton.hashes, &leaves);
    let error = verify_blob_range_statement_projection(
        &singleton.source,
        &fixture.range.inputs(),
        &fixture.range.context,
        &statement,
        &[0, 0],
        &fixture.range.parent,
        fixture
            .range
            .l1_proof(&fixture.range.openings(&fixture.range.l1_headers)),
    )
    .unwrap_err();
    assert!(
        matches!(
            error.downcast_ref::<PayloadError>(),
            Some(PayloadError::PayloadMismatch)
        ),
        "{error:?}"
    );
}

fn cut_leaves_for_constant_cursor(range: &RangeFixture, cursor: u64) -> Vec<B256> {
    range.blocks[..range.blocks.len() - 1]
        .iter()
        .map(|block| {
            cut_leaf(
                U256::from(block.execution.number),
                block.execution.hash_slow(),
                B256::from_slice(header_hash(&block.comet.header).as_bytes()),
                U256::from(cursor),
            )
        })
        .collect()
}

#[test]
fn phase_one_preserves_nonzero_and_maximum_cursors() {
    let fixture = authenticated_range_fixture();
    for cursor in [7, etna_core::schedule::UINT48_MAX] {
        let cuts = cut_leaves_for_constant_cursor(&fixture.range, cursor);
        let mut statement = fixture.range.statement(&fixture.hashes, &cuts);
        statement.forced_inclusion_head_before = U256::from(cursor);
        statement.input.forced_inclusion_head = U256::from(cursor);
        let (_, digest) = fixture
            .verify_statement(&statement, &[cursor, cursor])
            .unwrap();
        assert_eq!(digest, statement.try_digest().unwrap());
    }
}

#[test]
fn phase_one_rejects_consumption_with_consistent_statement_and_cuts() {
    let fixture = authenticated_range_fixture();
    for cursors in [[0, 1], [1, 1]] {
        let cuts = cut_leaves_for_constant_cursor(&fixture.range, cursors[0]);
        let mut statement = fixture.range.statement(&fixture.hashes, &cuts);
        statement.input.forced_inclusion_head = U256::from(1);
        raiko2_protocol_etna::range::verify_range_statement_projection(
            &fixture.range.inputs(),
            &fixture.range.context,
            &statement,
            &cursors,
            &fixture.range.parent,
        )
        .unwrap();
        let error = fixture
            .verify_statement(&statement, &cursors)
            .expect_err("phase one must reject actual cursor consumption");
        assert!(error.to_string().contains("phase-one Etna"), "{error:?}");
    }
}

#[test]
fn phase_one_rejects_carried_fi_witnesses_without_cursor_consumption() {
    let fixture = authenticated_range_fixture();
    for index in 0..2 {
        let mut range = RangeFixture::new(2);
        let mut envelope = vec![0x06];
        envelope.extend(list(&[
            &range.blocks[index].item,
            &list(&[range.blocks[index].anchor_witness.as_ref().unwrap()]),
            &[0xc0],
            &[0xc2, 0xc1, 0x01],
        ]));
        range.blocks[index].comet.data = vec![envelope];
        let mut previous = None;
        for block in &mut range.blocks {
            if let Some(hash) = previous {
                block.comet.header.last_block_id.as_mut().unwrap().hash = hash;
            }
            block.comet.header.data_hash = Some(data_hash(&block.comet.data));
            previous = Some(header_hash(&block.comet.header));
        }
        range.context.last_comet_hash = B256::from_slice(previous.unwrap().as_bytes());
        let cuts = range.cut_leaves();
        let statement = range.statement(&fixture.hashes, &cuts);
        let (projected, _) = raiko2_protocol_etna::range::verify_range_statement_projection(
            &range.inputs(),
            &range.context,
            &statement,
            &[0, 0],
            &range.parent,
        )
        .unwrap();
        assert!(projected[index].forced_witness().is_some());
        verify_authenticated_blob_execution_payload(&fixture.source, &fixture.hashes, &projected)
            .unwrap();
        let error = verify_blob_range_statement_projection(
            &fixture.source,
            &range.inputs(),
            &range.context,
            &statement,
            &[0, 0],
            &range.parent,
            range.l1_proof(&range.openings(&range.l1_headers)),
        )
        .expect_err("phase one must reject a carried FI witness even with unchanged cursors");
        assert!(error.to_string().contains("phase-one Etna"), "{error:?}");
    }
}

fn verify_l1_range(
    range: &RangeFixture,
    source: &InputDataSource,
    hashes: &[B256],
    raw_headers: &[Vec<u8>],
    max_headers: usize,
) -> anyhow::Result<()> {
    let cuts = range.cut_leaves();
    let statement = range.statement(hashes, &cuts);
    verify_blob_range_statement_projection(
        source,
        &range.inputs(),
        &range.context,
        &statement,
        &vec![0; range.blocks.len()],
        &range.parent,
        L1AnchorProof {
            limits: L1HistoryLimits {
                max_headers,
                max_proof_nodes: 128,
            },
            ..range.l1_proof(&range.openings(raw_headers))
        },
    )?;
    Ok(())
}

#[test]
fn l1_authentication_accepts_generation_first_and_multiple_headers() {
    for fixture in [
        authenticated_singleton_fixture(),
        authenticated_range_fixture(),
    ] {
        let (_, digest) = fixture
            .verify_statement(
                &fixture
                    .range
                    .statement(&fixture.hashes, &fixture.range.cut_leaves()),
                &vec![0; fixture.range.blocks.len()],
            )
            .unwrap();
        assert_ne!(digest, B256::ZERO);
    }
}

#[test]
fn l1_authentication_rejects_wrong_reference_number_hash_and_upper_bits() {
    let fixture = authenticated_range_fixture();
    let cuts = fixture.range.cut_leaves();
    for case in 0..3 {
        let mut statement = fixture.range.statement(&fixture.hashes, &cuts);
        match case {
            0 => statement.input.l1_reference_block_number += U256::from(1),
            1 => statement.l1_reference_block_hash = B256::repeat_byte(0xef),
            _ => statement.input.l1_reference_block_number += U256::from(1) << 64,
        }
        let error = fixture
            .verify_statement(&statement, &[0, 0])
            .expect_err("Q must bind both full-width fields");
        assert!(error.to_string().contains("L1"), "{error:?}");
    }
}

#[test]
fn l1_authentication_rejects_missing_reordered_duplicate_and_substituted_headers() {
    let fixture = authenticated_range_fixture();
    let original = &fixture.range.l1_headers;
    let mut gap = original.clone();
    gap.remove(1);
    let mut swapped = original.clone();
    swapped.swap(0, 1);
    let mut duplicate = original.clone();
    duplicate.insert(1, original[0].clone());
    let mut unlinked = original.clone();
    unlinked[1] = raw_l1_header(
        19,
        B256::repeat_byte(0xfa),
        B256::left_padding_from(&19u64.to_be_bytes()),
    );
    let missing_anchor = original[1..].to_vec();
    for headers in [gap, swapped, duplicate, unlinked, missing_anchor] {
        let error = verify_l1_range(
            &fixture.range,
            &fixture.source,
            &fixture.hashes,
            &headers,
            100,
        )
        .expect_err("every supplied anchor must authenticate to the statement reference");
        assert!(
            error.to_string().contains("L1") || error.to_string().contains("anchor"),
            "{error:?}"
        );
    }
}

#[test]
fn l1_authentication_bounds_count_before_decoding_and_rejects_bad_raw_headers() {
    let fixture = authenticated_range_fixture();
    let original = &fixture.range.l1_headers;
    verify_l1_range(
        &fixture.range,
        &fixture.source,
        &fixture.hashes,
        original,
        original.len() + 1,
    )
    .unwrap();
    for (headers, max, expected) in [
        (vec![], 10, "empty"),
        (vec![vec![0xff]; 2], 1, "count"),
        (vec![vec![0xff]], 0, "count"),
        (vec![vec![0xff]], 2, "L1"),
        (vec![vec![0; 4097]], 2, "L1"),
    ] {
        let error = verify_l1_range(
            &fixture.range,
            &fixture.source,
            &fixture.hashes,
            &headers,
            max,
        )
        .expect_err("invalid or unbounded raw ancestry must be rejected");
        assert!(
            error.to_string().contains(expected),
            "{expected}: {error:?}"
        );
    }
    let mut trailing = original.clone();
    trailing[0].push(0x80);
    assert!(verify_l1_range(
        &fixture.range,
        &fixture.source,
        &fixture.hashes,
        &trailing,
        100
    )
    .is_err());
}

#[test]
fn l1_authentication_rejects_malformed_inner_anchor_witness_framing() {
    let fixture = authenticated_range_fixture();
    let raw = &fixture.range.l1_headers[0];
    let encoded = alloy_rlp::encode(raw.as_slice());
    for witness in [
        vec![0xc0],
        list(&[&encoded]),
        list(&[&encoded, &[0x80]]),
        list(&[&encoded, &[0xc0], &[0xc0]]),
        list(&[raw, &[0xc0]]),
    ] {
        let mut range = RangeFixture::new(2);
        range.blocks[0].anchor_witness = Some(witness);
        range.reseal();
        let projected = range.project();
        verify_authenticated_blob_execution_payload(&fixture.source, &fixture.hashes, &projected)
            .unwrap();
        let error = verify_l1_range(
            &range,
            &fixture.source,
            &fixture.hashes,
            &range.l1_headers,
            100,
        )
        .expect_err("inner anchor shape must be opened, not treated as an opaque list");
        assert!(error.to_string().contains("anchor witness"), "{error:?}");
    }
}

#[test]
fn l1_authentication_rejects_a_different_carried_header_with_the_same_number_and_root() {
    let fixture = authenticated_range_fixture();
    let mut range = RangeFixture::new(2);
    let mut forged: Header = alloy_rlp::decode_exact(&range.l1_headers[0]).unwrap();
    forged.timestamp += 1;
    let encoded = alloy_rlp::encode(alloy_rlp::encode(&forged).as_slice());
    range.blocks[0].anchor_witness = Some(list(&[&encoded, &[0xc0]]));
    range.reseal();
    verify_authenticated_blob_execution_payload(&fixture.source, &fixture.hashes, &range.project())
        .unwrap();
    let error = verify_l1_range(
        &range,
        &fixture.source,
        &fixture.hashes,
        &range.l1_headers,
        100,
    )
    .expect_err("equal number/root cannot substitute different raw L1 bytes");
    assert!(error.to_string().contains("anchor witness"), "{error:?}");
}

#[test]
fn l1_authentication_rejects_an_authentic_chain_root_that_differs_from_the_el_root() {
    let fixture = authenticated_range_fixture();
    let mut range = RangeFixture::new(2);
    let mut parent = B256::ZERO;
    for (index, raw) in range.l1_headers.iter_mut().enumerate() {
        let mut header: Header = alloy_rlp::decode_exact(raw.as_slice()).unwrap();
        header.parent_hash = parent;
        if index == 0 {
            header.state_root = B256::repeat_byte(0xfe);
        }
        *raw = alloy_rlp::encode(&header);
        parent = alloy_primitives::keccak256(&*raw);
    }
    bind_parent_beacon(&mut range.l1_headers);
    range.blocks[0].anchor_witness = Some(list(&[
        &alloy_rlp::encode(range.l1_headers[0].as_slice()),
        &[0xc0],
    ]));
    range.reseal();
    verify_authenticated_blob_execution_payload(&fixture.source, &fixture.hashes, &range.project())
        .unwrap();
    let error = verify_l1_range(
        &range,
        &fixture.source,
        &fixture.hashes,
        &range.l1_headers,
        100,
    )
    .expect_err("the committed EL root must be the authenticated L1 state root");
    assert!(error.to_string().contains("state root"), "{error:?}");
}

#[test]
fn l1_authentication_preserves_unknown_future_raw_header_fields() {
    let fixture = authenticated_range_fixture();
    let mut range = RangeFixture::new(2);
    let mut parent = B256::ZERO;
    for raw in &mut range.l1_headers {
        let mut header: Header = alloy_rlp::decode_exact(raw.as_slice()).unwrap();
        header.parent_hash = parent;
        let encoded = alloy_rlp::encode(&header);
        let mut cursor = encoded.as_slice();
        let alloy_rlp::PayloadView::List(mut fields) =
            alloy_rlp::Header::decode_raw(&mut cursor).unwrap()
        else {
            panic!()
        };
        fields.push(&[0x81, 0xf1]);
        *raw = list(&fields);
        parent = alloy_primitives::keccak256(&*raw);
    }
    bind_parent_beacon(&mut range.l1_headers);
    for (index, block) in range.blocks.iter_mut().enumerate() {
        block.anchor_witness = Some(list(&[
            &alloy_rlp::encode(range.l1_headers[index].as_slice()),
            &[0xc0],
        ]));
    }
    range.reseal();
    verify_l1_range(
        &range,
        &fixture.source,
        &fixture.hashes,
        &range.l1_headers,
        100,
    )
    .unwrap();
}

fn rebuild_execution_items(range: &mut RangeFixture) {
    let mut parent = range.parent.hash_slow();
    for block in &mut range.blocks {
        block.execution.parent_hash = parent;
        parent = block.execution.hash_slow();
        let mut cursor = block.item.as_slice();
        let alloy_rlp::PayloadView::List(fields) =
            alloy_rlp::Header::decode_raw(&mut cursor).unwrap()
        else {
            panic!()
        };
        block.item = list(&[&alloy_rlp::encode(&block.execution), fields[1]]);
    }
    range.context.parent_el_hash = range.parent.hash_slow();
    range.context.last_el_hash = parent;
    range.reseal();
}

#[test]
fn l1_authentication_accepts_covered_and_incoming_inheritance() {
    for incoming in [false, true] {
        let mut range = RangeFixture::new(2);
        // At a non-origin boundary the excluded parent binds the inherited anchor.
        if incoming {
            range.context.origin_height = 16;
            let id = range.blocks[1].comet.header.last_block_id.unwrap();
            range.context.parent_comet_hash = B256::from_slice(id.hash.as_bytes());
            range.blocks[0].comet.header.last_block_id = Some(id);
            range.parent.extra_data = range.blocks[0].execution.extra_data.clone();
            // PBBR follows these fields in header RLP; include them so the
            // excluded parent remains a canonically represented typed header.
            range.parent.base_fee_per_gas = range.blocks[0].execution.base_fee_per_gas;
            range.parent.withdrawals_root = range.blocks[0].execution.withdrawals_root;
            range.parent.blob_gas_used = range.blocks[0].execution.blob_gas_used;
            range.parent.excess_blob_gas = range.blocks[0].execution.excess_blob_gas;
            range.parent.parent_beacon_block_root =
                range.blocks[0].execution.parent_beacon_block_root;
            range.blocks[0].anchor_witness = None;
        }
        range.blocks[1].execution.extra_data = range.blocks[0].execution.extra_data.clone();
        range.blocks[1].execution.parent_beacon_block_root =
            range.blocks[0].execution.parent_beacon_block_root;
        range.blocks[1].anchor_witness = None;
        rebuild_execution_items(&mut range);
        let (source, hashes) = prove_blobs(pack(&v6_payload(
            &range.decoded_payload(),
            Compression::default(),
        )));
        verify_l1_range(&range, &source, &hashes, &range.l1_headers[..1], 100).unwrap();
        // Q is still authenticated as the reference. Opening it a second time as
        // an anchor would add a valid header that neither covered block uses.
        let error = verify_l1_range(&range, &source, &hashes, &range.l1_headers, 100)
            .expect_err("a valid unused Q opening must be rejected");
        assert!(error.to_string().contains("unused L1 anchor"), "{error:?}");
        let missing_anchor = &range.l1_headers[1..];
        assert!(verify_l1_range(&range, &source, &hashes, missing_anchor, 100).is_err());
    }
}
