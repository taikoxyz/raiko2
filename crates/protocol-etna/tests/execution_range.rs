//! Behavioral tests for the complete Etna execution byte projection.
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
use raiko2_protocol_etna::projection::{ExecutionProjection, ProjectionError};
use raiko2_protocol_etna::range::{
    RangeBlock, RangeProjectionContext, RangeProjectionError, verify_range_projection,
};
use tendermint::{
    Hash,
    block::{Header as CometHeader, Id, parts},
};

const ANCHOR: &[u8] = &[0xc1, 0x11];
const COMMITTEE: &[u8] = &[0xc1, 0x22];
const FORCED: &[u8] = &[0xc1, 0x33];

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

fn comet_hash(header: &CometHeader) -> B256 {
    B256::from_slice(header_hash(header).as_bytes())
}

fn comet_parent(hash: B256) -> Id {
    Id {
        hash: Hash::Sha256(hash.into()),
        part_set_header: parts::Header::new(1, Hash::Sha256([0x44; 32])).unwrap(),
    }
}

struct BlockFixture {
    comet_header: CometHeader,
    comet_transactions: [Vec<u8>; 1],
    execution_header: Header,
    transactions: Vec<TxEnvelope>,
}

impl BlockFixture {
    fn execution_item(&self) -> Vec<u8> {
        let encoded: Vec<_> = self
            .transactions
            .iter()
            .map(|tx| alloy_rlp::encode(tx.encoded_2718().as_slice()))
            .collect();
        let items: Vec<_> = encoded.iter().map(Vec::as_slice).collect();
        list(&[&alloy_rlp::encode(&self.execution_header), &list(&items)])
    }

    fn seal_execution(&mut self) {
        self.comet_transactions[0] = [
            vec![6],
            list(&[
                &self.execution_item(),
                &list(&[ANCHOR]),
                &list(&[COMMITTEE]),
                &list(&[FORCED]),
            ]),
        ]
        .concat();
        self.comet_header.data_hash = Some(data_hash(&self.comet_transactions));
    }

    fn input(&self) -> RangeBlock<'_, TxEnvelope> {
        RangeBlock {
            comet_header: &self.comet_header,
            comet_transactions: &self.comet_transactions,
            execution_header: &self.execution_header,
            execution_transactions: &self.transactions,
        }
    }

    fn original_span(&self, expected: &[u8]) -> &[u8] {
        self.comet_transactions[0]
            .windows(expected.len())
            .find(|span| *span == expected)
            .expect("fixture must contain the original bytes")
    }
}

struct RangeFixture {
    context: RangeProjectionContext,
    blocks: Vec<BlockFixture>,
}

impl RangeFixture {
    fn new(origin_height: u64, parent_height: u64, count: u64) -> Self {
        let domain = DeploymentDomain {
            l1_chain_id: U256::from(1),
            l2_chain_id: U256::from(167_001),
            inbox: Address::repeat_byte(0x11),
            generation: 2,
        };
        let parent_el_hash = B256::repeat_byte(0x20);
        let parent_comet_hash = B256::repeat_byte(0x30);
        let last_height = parent_height + count;
        let raw = hex::decode(include_str!("fixtures/first-block.hex").trim()).unwrap();
        let (_, template) = decode_canonical_block(&raw).unwrap();
        let mut comet_header = template.header;
        comet_header.chain_id = chain_id(&domain).unwrap().parse().unwrap();
        comet_header.version.app = APP_VERSION;
        comet_header.app_hash = vec![0x61, 0x62, 0x63].try_into().unwrap();

        // Public signed legacy and EIP-1559 transactions, as used by the per-block tests.
        let transactions = [
            include_str!("fixtures/projection-legacy.hex"),
            include_str!("fixtures/projection-eip1559.hex"),
        ]
        .map(|encoded| {
            TxEnvelope::decode_2718_exact(&hex::decode(encoded.trim()).unwrap()).unwrap()
        });

        let mut el_hash = parent_el_hash;
        let mut previous_comet_hash = parent_comet_hash;
        let mut blocks = Vec::new();
        for (index, height) in (parent_height + 1..=last_height).enumerate() {
            let mut comet_header = comet_header.clone();
            comet_header.height = height.try_into().unwrap();
            comet_header.last_block_id =
                (height > origin_height + 1).then(|| comet_parent(previous_comet_hash));
            let transactions = transactions[..index % 3].to_vec();
            let mut block = BlockFixture {
                comet_header,
                comet_transactions: [Vec::new()],
                execution_header: Header {
                    number: height,
                    parent_hash: el_hash,
                    beneficiary: Address::repeat_byte(0x42),
                    extra_data: hex!("64000000000002000000010203").into(),
                    transactions_root: calculate_transaction_root(&transactions),
                    ..Default::default()
                },
                transactions,
            };
            block.seal_execution();
            el_hash = block.execution_header.hash_slow();
            previous_comet_hash = comet_hash(&block.comet_header);
            blocks.push(block);
        }

        Self {
            context: RangeProjectionContext {
                domain,
                origin_height,
                parent_height,
                last_height,
                parent_el_hash,
                parent_comet_hash,
                last_el_hash: el_hash,
                last_comet_hash: previous_comet_hash,
                max_blocks: count.try_into().unwrap(),
                max_envelope_bytes: 130_000,
            },
            blocks,
        }
    }

    fn inputs(&self) -> Vec<RangeBlock<'_, TxEnvelope>> {
        self.blocks.iter().map(BlockFixture::input).collect()
    }

    fn verify(&self) -> Result<Vec<ExecutionProjection<'_>>, RangeProjectionError> {
        verify_range_projection(&self.inputs(), &self.context)
    }

    fn relink_comet_after(&mut self, index: usize) {
        let mut previous_hash = comet_hash(&self.blocks[index].comet_header);
        for block in &mut self.blocks[index + 1..] {
            block.comet_header.last_block_id = Some(comet_parent(previous_hash));
            previous_hash = comet_hash(&block.comet_header);
        }
        self.context.last_comet_hash = previous_hash;
    }

    fn reseal_execution_from(&mut self, index: usize) {
        self.blocks[index].seal_execution();
        let mut el_hash = self.blocks[index].execution_header.hash_slow();
        for block in &mut self.blocks[index + 1..] {
            block.execution_header.parent_hash = el_hash;
            block.seal_execution();
            el_hash = block.execution_header.hash_slow();
        }
        self.context.last_el_hash = el_hash;
        self.relink_comet_after(index);
    }
}

#[test]
fn projects_three_consecutive_blocks_and_preserves_original_spans() {
    let fixture = RangeFixture::new(17, 17, 3);
    let projections = verify_range_projection(&fixture.inputs(), &fixture.context)
        .expect("the complete linked range must project");

    assert_eq!(projections.len(), 3);
    for ((projection, block), transaction_count) in
        projections.iter().zip(&fixture.blocks).zip([0, 1, 2])
    {
        assert_eq!(
            projection.execution_hash(),
            block.execution_header.hash_slow()
        );
        assert_eq!(projection.anchor_number(), 0x01_02_03);
        assert_eq!(projection.transaction_bytes().len(), transaction_count);
        let item = block.execution_item();
        assert!(core::ptr::eq(
            projection.execution_item_bytes(),
            block.original_span(&item),
        ));
        for (actual, transaction) in projection
            .transaction_bytes()
            .iter()
            .zip(&block.transactions)
        {
            let original = block.original_span(&transaction.encoded_2718());
            assert!(core::ptr::eq(*actual, original));
        }
        for (actual, bytes) in [
            (projection.anchor_witness(), ANCHOR),
            (projection.committee_witness(), COMMITTEE),
            (projection.forced_witness(), FORCED),
        ] {
            assert!(core::ptr::eq(actual.unwrap(), block.original_span(bytes)));
        }
    }
}

#[test]
fn projects_singletons_at_initial_and_nonzero_generation_origins() {
    for origin in [0, 17, (1 << 48) - 2] {
        let fixture = RangeFixture::new(origin, origin, 1);
        let projections = fixture.verify().unwrap();
        assert_eq!(projections.len(), 1);
        assert_eq!(
            projections[0].execution_hash(),
            fixture.context.last_el_hash
        );
    }
}

#[test]
fn projects_from_a_parent_inside_the_generation() {
    let fixture = RangeFixture::new(17, 20, 2);
    let projections = fixture.verify().unwrap();
    assert_eq!(projections.len(), 2);
    assert_eq!(
        projections[0].execution_hash(),
        fixture.blocks[0].execution_header.hash_slow()
    );
    assert_eq!(
        projections[1].execution_hash(),
        fixture.context.last_el_hash
    );
}

#[test]
fn rejects_a_comet_parent_id_at_the_first_generation_height() {
    let mut fixture = RangeFixture::new(17, 17, 3);
    fixture.blocks[0].comet_header.last_block_id =
        Some(comet_parent(fixture.context.parent_comet_hash));
    fixture.relink_comet_after(0);

    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::Projection {
            height: 18,
            source: ProjectionError::CometParent,
        })
    ));
}

#[test]
fn rejects_a_wrong_first_or_interior_comet_parent_inside_the_generation() {
    for (index, expected_height) in [(0, 21), (1, 22)] {
        let mut fixture = RangeFixture::new(17, 20, 3);
        fixture.blocks[index].comet_header.last_block_id =
            Some(comet_parent(B256::repeat_byte(0x55)));
        fixture.relink_comet_after(index);

        assert!(matches!(
            fixture.verify(),
            Err(RangeProjectionError::Projection {
                height,
                source: ProjectionError::CometParent,
            }) if height == expected_height
        ));
    }
}

#[test]
fn checks_the_predecessor_hash_without_part_set_equality() {
    let mut fixture = RangeFixture::new(17, 17, 3);
    let parent = fixture.blocks[1]
        .comet_header
        .last_block_id
        .as_mut()
        .unwrap();
    parent.part_set_header = parts::Header::new(2, Hash::Sha256([0x55; 32])).unwrap();
    fixture.relink_comet_after(1);

    assert_eq!(fixture.verify().unwrap().len(), 3);
}

#[test]
fn rejects_a_wrong_first_or_interior_execution_parent() {
    for (index, expected_height) in [(0, 18), (1, 19)] {
        let mut fixture = RangeFixture::new(17, 17, 3);
        fixture.blocks[index].execution_header.parent_hash = B256::ZERO;
        fixture.reseal_execution_from(index);

        assert!(matches!(
            fixture.verify(),
            Err(RangeProjectionError::Projection {
                height,
                source: ProjectionError::ExecutionParent,
            }) if height == expected_height
        ));
    }
}

#[test]
fn rejects_an_el_height_different_from_its_range_position() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    fixture.blocks[1].execution_header.number = 20;
    fixture.reseal_execution_from(1);

    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::Height,
        })
    ));
}

#[test]
fn rejects_missing_duplicate_and_reordered_blocks() {
    let fixture = RangeFixture::new(17, 17, 3);
    let [first, second, third] = fixture.blocks.as_slice() else {
        panic!("expected three blocks");
    };
    let missing = [first.input(), third.input()];
    assert!(matches!(
        verify_range_projection(&missing, &fixture.context),
        Err(RangeProjectionError::BlockCount {
            expected: 3,
            actual: 2
        })
    ));
    let extra = [first.input(), second.input(), third.input(), third.input()];
    assert!(matches!(
        verify_range_projection(&extra, &fixture.context),
        Err(RangeProjectionError::BlockCount {
            expected: 3,
            actual: 4
        })
    ));
    let duplicate = [first.input(), first.input(), third.input()];
    assert!(matches!(
        verify_range_projection(&duplicate, &fixture.context),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::Height,
        })
    ));
    let reordered = [second.input(), first.input(), third.input()];
    assert!(matches!(
        verify_range_projection(&reordered, &fixture.context),
        Err(RangeProjectionError::Projection {
            height: 18,
            source: ProjectionError::Height,
        })
    ));
}

#[test]
fn rejects_changed_original_transaction_bytes_even_when_comet_commits_them() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    let block = &mut fixture.blocks[1];
    let transaction = block.transactions[0].encoded_2718();
    let envelope = &mut block.comet_transactions[0];
    let offset = envelope
        .windows(transaction.len())
        .position(|span| span == transaction.as_slice())
        .unwrap();
    envelope[offset + transaction.len() - 1] ^= 1;
    block.comet_header.data_hash = Some(data_hash(&block.comet_transactions));
    fixture.relink_comet_after(1);

    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::ExecutionTransactions,
        })
    ));
}

#[test]
fn rejects_an_interior_data_hash_mismatch_with_resealed_comet_ancestry() {
    let mut fixture = RangeFixture::new(17, 17, 3);
    fixture.blocks[1].comet_header.data_hash = None;
    fixture.relink_comet_after(1);

    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::DataHash,
        })
    ));
}

#[test]
fn rejects_a_different_deployment_or_generation_domain() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    let original_domain = fixture.context.domain;
    for unexpected_domain in [
        DeploymentDomain {
            inbox: Address::repeat_byte(0x55),
            ..original_domain
        },
        DeploymentDomain {
            generation: 3,
            ..original_domain
        },
    ] {
        fixture.context.domain = unexpected_domain;
        assert!(matches!(
            fixture.verify(),
            Err(RangeProjectionError::Projection {
                height: 18,
                source: ProjectionError::ChainId,
            })
        ));
    }
}

#[test]
fn rejects_a_different_generation_in_execution_extra_data() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    fixture.blocks[1].execution_header.extra_data = hex!("64000000000003000000010203").into();
    fixture.reseal_execution_from(1);

    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::ExtraData,
        })
    ));
}

#[test]
fn inherits_the_per_block_application_version_policy() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    fixture.blocks[1].comet_header.version.app = APP_VERSION + 1;
    fixture.relink_comet_after(1);

    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::AppVersion,
        })
    ));
}

#[test]
fn rejects_terminal_comet_or_execution_endpoint_mismatches() {
    let mut fixture = RangeFixture::new(17, 17, 3);
    let original = fixture.context;
    fixture.context.last_comet_hash = B256::ZERO;
    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::TerminalCometHash)
    ));
    fixture.context = original;
    fixture.context.last_el_hash = B256::ZERO;
    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::TerminalExecutionHash)
    ));
}

#[test]
fn rejects_empty_inverted_out_of_origin_and_overflowing_intervals() {
    let fixture = RangeFixture::new(17, 17, 1);
    let inputs = fixture.inputs();
    for (origin_height, parent_height, last_height) in [
        (17, 17, 17),
        (17, 18, 17),
        (18, 17, 18),
        ((1 << 48) - 1, (1 << 48) - 1, 1 << 48),
    ] {
        let context = RangeProjectionContext {
            origin_height,
            parent_height,
            last_height,
            ..fixture.context
        };
        assert!(matches!(
            verify_range_projection(&inputs, &context),
            Err(RangeProjectionError::InvalidRange)
        ));
    }
}

#[test]
fn checks_range_budget_before_per_block_projection() {
    let mut fixture = RangeFixture::new(17, 17, 3);
    fixture.context.max_blocks = 2;
    fixture.blocks[0].comet_header.version.app = APP_VERSION + 1;

    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::BlockLimit { actual: 3, max: 2 })
    ));
}

#[test]
fn applies_the_envelope_byte_limit_at_every_height() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    let first_len = fixture.blocks[0].comet_transactions[0].len();
    let last_len = fixture.blocks[1].comet_transactions[0].len();
    assert!(first_len < last_len);

    fixture.context.max_envelope_bytes = last_len;
    assert_eq!(fixture.verify().unwrap().len(), 2);
    fixture.context.max_envelope_bytes = last_len - 1;
    assert!(matches!(
        fixture.verify(),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::EnvelopeSize,
        })
    ));
}
