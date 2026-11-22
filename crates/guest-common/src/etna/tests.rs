//! Exercises Etna blob packing and V6 derivation.

use super::{decode_blob_payload, decode_execution_payload, verify_blob_execution_payload};
use alloy_consensus::{proofs::calculate_transaction_root, Header, TxEnvelope};
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
use raiko2_protocol_etna::{
    payload::PayloadError,
    projection::ExecutionProjection,
    range::{verify_range_projection, RangeBlock, RangeProjectionContext},
};
use std::io::{self, Read, Write};
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

struct BlockFixture {
    comet: CometBlock,
    execution: Header,
    transactions: Vec<TxEnvelope>,
    item: Vec<u8>,
}

struct RangeFixture {
    parent: Header,
    blocks: Vec<BlockFixture>,
    context: RangeProjectionContext,
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
        for index in 0..count {
            let height = parent.number + u64::try_from(index).unwrap() + 1;
            let mut extra_data = hex!("64000000000002000000000000");
            extra_data[7..].copy_from_slice(&height.to_be_bytes()[2..]);

            // The fixture is a legacy transaction, whose network encoding is its original RLP.
            let (transactions, encoded_transactions) = if index == 0 {
                (Vec::new(), vec![0xc0])
            } else {
                let tx: TxEnvelope = alloy_rlp::decode_exact(&signed_wire).unwrap();
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
            envelope.extend(list(&[&item, &[0xc2, 0xc1, 0x01], &[0xc0], &[0xc0]]));

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
        }
    }

    fn project(&self) -> Vec<ExecutionProjection<'_>> {
        let inputs: Vec<_> = self
            .blocks
            .iter()
            .map(|block| RangeBlock {
                comet_header: &block.comet.header,
                comet_transactions: &block.comet.data,
                execution_header: &block.execution,
                execution_transactions: &block.transactions,
            })
            .collect();
        verify_range_projection(&inputs, &self.context, &self.parent).unwrap()
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
