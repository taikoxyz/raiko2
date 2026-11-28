//! Behavioral tests for the complete Etna execution byte projection.
use alloy_consensus::{Header, TxEnvelope, proofs::calculate_transaction_root};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::{Address, B256, Bytes, U256, hex, keccak256};
use etna_core::{
    comet::{
        APP_VERSION,
        codec::decode_canonical_block,
        verify::{data_hash, header_hash},
    },
    domain::{DeploymentDomain, chain_id},
    params::UINT48_MAX,
};
use raiko2_protocol_etna::projection::{ExecutionProjection, ProjectionError};
use raiko2_protocol_etna::range::{
    RangeBlock, RangeProjectionContext, RangeProjectionError, RangeStatementProjectionError,
    verify_range_projection, verify_range_statement_projection,
};
use raiko2_protocol_etna::statement::{LandInput, LandStatement, LandStatementError};
use tendermint::{
    Hash,
    block::{Header as CometHeader, Id, parts},
};

const FIRST_ANCHOR_NUMBER: u64 = 0x01_02_03;
const ANCHOR: &[u8] = &[0xc1, 0x11];
const FORCED: &[u8] = &[0xc1, 0x33];
const BLOBS: [B256; 1] = [B256::repeat_byte(0x79)];

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

fn anchor_extra_data(generation: u64, anchor_number: u64) -> [u8; 29] {
    let mut extra_data = [0; 29];
    extra_data[0] = 100;
    extra_data[1..7].copy_from_slice(&generation.to_be_bytes()[2..]);
    extra_data[7..13].copy_from_slice(&anchor_number.to_be_bytes()[2..]);
    extra_data
}

struct BlockFixture {
    comet_header: CometHeader,
    comet_transactions: [Vec<u8>; 1],
    execution_header: Header,
    transactions: Vec<TxEnvelope>,
    carries_anchor: bool,
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
        let anchor = if self.carries_anchor {
            list(&[ANCHOR])
        } else {
            vec![0xc0]
        };
        self.comet_transactions[0] = [
            vec![6],
            list(&[&self.execution_item(), &anchor, &list(&[FORCED])]),
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
    parent_execution_header: Header,
    blocks: Vec<BlockFixture>,
}

impl RangeFixture {
    fn new(origin_height: u64, parent_height: u64, count: u64) -> Self {
        let domain = DeploymentDomain {
            l1_chain_id: U256::from(1),
            l2_chain_id: U256::from(167_001),
            inbox: Address::repeat_byte(0x11),
            generation: if origin_height == 0 { 0 } else { 2 },
        };
        let parent_execution_header = if parent_height > origin_height {
            let anchor_number = FIRST_ANCHOR_NUMBER - 1;
            Header {
                number: parent_height,
                extra_data: anchor_extra_data(domain.generation, anchor_number).into(),
                base_fee_per_gas: Some(1),
                withdrawals_root: Some(B256::repeat_byte(0x67)),
                blob_gas_used: Some(0),
                excess_blob_gas: Some(0),
                parent_beacon_block_root: Some(B256::left_padding_from(
                    &anchor_number.to_be_bytes(),
                )),
                ..Default::default()
            }
        } else {
            Header {
                number: parent_height,
                extra_data: if origin_height == 0 {
                    Bytes::new()
                } else {
                    anchor_extra_data(domain.generation - 1, UINT48_MAX).into()
                },
                ..Default::default()
            }
        };
        let parent_el_hash = parent_execution_header.hash_slow();
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
            let anchor_number = FIRST_ANCHOR_NUMBER + (height - parent_height - 1);
            let transactions = transactions[..index % 3].to_vec();
            let mut block = BlockFixture {
                comet_header,
                comet_transactions: [Vec::new()],
                execution_header: Header {
                    number: height,
                    parent_hash: el_hash,
                    beneficiary: Address::repeat_byte(0x42),
                    extra_data: anchor_extra_data(domain.generation, anchor_number).into(),
                    transactions_root: calculate_transaction_root(&transactions),
                    base_fee_per_gas: Some(1),
                    withdrawals_root: Some(B256::repeat_byte(0x67)),
                    blob_gas_used: Some(0),
                    excess_blob_gas: Some(0),
                    parent_beacon_block_root: Some(B256::left_padding_from(
                        &anchor_number.to_be_bytes(),
                    )),
                    ..Default::default()
                },
                transactions,
                carries_anchor: true,
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
            parent_execution_header,
            blocks,
        }
    }

    fn inputs(&self) -> Vec<RangeBlock<'_, TxEnvelope>> {
        self.blocks.iter().map(BlockFixture::input).collect()
    }

    fn verify(&self) -> Result<Vec<ExecutionProjection<'_>>, RangeProjectionError> {
        verify_range_projection(&self.inputs(), &self.context, &self.parent_execution_header)
    }

    fn statement<'a>(&self, before: u64, after: u64, cut_leaves: &'a [B256]) -> LandStatement<'a> {
        LandStatement {
            l1_chain_id: self.context.domain.l1_chain_id,
            l2_chain_id: self.context.domain.l2_chain_id,
            inbox: self.context.domain.inbox,
            generation: U256::from(self.context.domain.generation),
            parent_hash: self.context.parent_el_hash,
            parent_comet_hash: self.context.parent_comet_hash,
            config_hash: B256::repeat_byte(0x59),
            input: LandInput {
                parent_height: U256::from(self.context.parent_height),
                last_height: U256::from(self.context.last_height),
                last_block_hash: self.context.last_el_hash,
                last_comet_hash: self.context.last_comet_hash,
                last_state_root: self.blocks.last().unwrap().execution_header.state_root,
                l1_reference_block_number: U256::from(2_050),
                forced_inclusion_head: U256::from(after),
            },
            l1_reference_block_hash: B256::repeat_byte(0x56),
            blob_hashes: &BLOBS,
            forced_inclusion_head_before: U256::from(before),
            cut_leaves,
        }
    }

    fn verify_statement(
        &self,
        statement: &LandStatement<'_>,
        post_block_cursors: &[u64],
    ) -> Result<(Vec<ExecutionProjection<'_>>, B256), RangeStatementProjectionError> {
        verify_range_statement_projection(
            &self.inputs(),
            &self.context,
            statement,
            post_block_cursors,
            &self.parent_execution_header,
        )
    }

    fn bind_parent(&mut self) {
        self.context.parent_el_hash = self.parent_execution_header.hash_slow();
        self.blocks[0].execution_header.parent_hash = self.context.parent_el_hash;
        self.reseal_execution_from(0);
    }

    fn set_parent_anchor(&mut self, number: u64, root: Option<B256>) {
        self.parent_execution_header.extra_data =
            anchor_extra_data(self.context.domain.generation, number).into();
        self.parent_execution_header.parent_beacon_block_root = root;
        self.bind_parent();
    }

    fn set_anchor(&mut self, index: usize, number: u64, root: Option<B256>, carries: bool) {
        let block = &mut self.blocks[index];
        block.execution_header.extra_data =
            anchor_extra_data(self.context.domain.generation, number).into();
        block.execution_header.parent_beacon_block_root = root;
        block.carries_anchor = carries;
        self.reseal_execution_from(index);
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

// Encode the four ABI words independently of statement::cut_leaf.
fn cut(height: u64, el_hash: B256, comet_hash: B256, cursor: u64) -> B256 {
    let mut words = [0; 128];
    words[24..32].copy_from_slice(&height.to_be_bytes());
    words[32..64].copy_from_slice(el_hash.as_slice());
    words[64..96].copy_from_slice(comet_hash.as_slice());
    words[120..128].copy_from_slice(&cursor.to_be_bytes());
    keccak256(words)
}

fn cut_for(block: &BlockFixture, cursor: u64) -> B256 {
    cut(
        block.execution_header.number,
        block.execution_header.hash_slow(),
        comet_hash(&block.comet_header),
        cursor,
    )
}

fn expected_digest(statement: &LandStatement<'_>) -> B256 {
    let integer = |value: U256| value.to_be_bytes::<32>();
    let label = |bytes: &[u8]| {
        let mut word = [0; 32];
        word[..bytes.len()].copy_from_slice(bytes);
        word
    };
    let address = |value: Address| {
        let mut word = [0; 32];
        word[12..].copy_from_slice(value.as_slice());
        word
    };
    let packed_hash =
        |hashes: &[B256]| keccak256(hashes.iter().flat_map(|hash| hash.0).collect::<Vec<_>>()).0;
    let input = &statement.input;
    let cut_hash = keccak256(
        [
            label(b"ETNA_LAND_CUTS_V1"),
            integer(statement.generation),
            integer(input.parent_height),
            integer(input.last_height),
            packed_hash(statement.cut_leaves),
        ]
        .concat(),
    );

    keccak256(
        [
            label(b"TAIKO_ETNA_LAND_V1"),
            integer(statement.l1_chain_id),
            integer(statement.l2_chain_id),
            address(statement.inbox),
            integer(statement.generation),
            integer(input.parent_height),
            statement.parent_hash.0,
            statement.parent_comet_hash.0,
            statement.config_hash.0,
            integer(input.last_height),
            input.last_block_hash.0,
            input.last_comet_hash.0,
            input.last_state_root.0,
            integer(input.l1_reference_block_number),
            statement.l1_reference_block_hash.0,
            packed_hash(statement.blob_hashes),
            integer(statement.forced_inclusion_head_before),
            integer(input.forced_inclusion_head),
            cut_hash.0,
        ]
        .concat(),
    )
}

#[test]
fn binds_singleton_statement_at_nonzero_generation_origin() {
    let fixture = RangeFixture::new(17, 17, 1);
    let statement = fixture.statement(UINT48_MAX - 1, UINT48_MAX, &[]);
    let (projections, digest) = fixture.verify_statement(&statement, &[UINT48_MAX]).unwrap();

    assert_eq!(digest, expected_digest(&statement));
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].comet_hash(), fixture.context.last_comet_hash);
    assert_eq!(projections[0].transaction_bytes(), &[] as &[&[u8]]);
    assert!(core::ptr::eq(
        projections[0].execution_item_bytes(),
        fixture.blocks[0].original_span(&fixture.blocks[0].execution_item()),
    ));
}

#[test]
fn binds_historical_range_to_derived_cuts_and_preserves_original_spans() {
    let fixture = RangeFixture::new(17, 20, 3);
    let cursors = [31, 32, 32];
    let leaves = [
        cut_for(&fixture.blocks[0], 31),
        cut_for(&fixture.blocks[1], 32),
    ];
    let statement = fixture.statement(31, 32, &leaves);
    let (projections, digest) = fixture.verify_statement(&statement, &cursors).unwrap();

    assert_eq!(digest, expected_digest(&statement));
    assert_eq!(projections.len(), 3);
    for (projection, block) in projections.iter().zip(&fixture.blocks) {
        assert_eq!(projection.comet_hash(), comet_hash(&block.comet_header));
        assert_eq!(
            projection.execution_hash(),
            block.execution_header.hash_slow()
        );
        assert!(core::ptr::eq(
            projection.execution_item_bytes(),
            block.original_span(&block.execution_item()),
        ));
        assert_eq!(
            projection.transaction_bytes().len(),
            block.transactions.len()
        );
        for (actual, tx) in projection
            .transaction_bytes()
            .iter()
            .zip(&block.transactions)
        {
            assert!(core::ptr::eq(
                *actual,
                block.original_span(&tx.encoded_2718())
            ));
        }
        for (actual, bytes) in [
            (projection.anchor_witness(), ANCHOR),
            (projection.forced_witness(), FORCED),
        ] {
            assert!(core::ptr::eq(actual.unwrap(), block.original_span(bytes)));
        }
    }
}

#[test]
fn rejects_equal_count_cut_leaves_with_substituted_fields_or_order() {
    let fixture = RangeFixture::new(17, 20, 3);
    let first = &fixture.blocks[0];
    let height = first.execution_header.number;
    let el = first.execution_header.hash_slow();
    let comet = comet_hash(&first.comet_header);
    let second_leaf = cut_for(&fixture.blocks[1], 32);
    for leaves in [
        [cut(height, el, B256::ZERO, 31), second_leaf],
        [cut(height, B256::ZERO, comet, 31), second_leaf],
        [cut(height + 1, el, comet, 31), second_leaf],
        [cut(height, el, comet, 32), second_leaf],
        [second_leaf, cut_for(first, 31)],
    ] {
        let statement = fixture.statement(31, 32, &leaves);
        assert!(statement.try_digest().is_ok());
        assert!(matches!(
            fixture.verify_statement(&statement, &[31, 32, 32]),
            Err(RangeStatementProjectionError::CutLeaf { height: 21 })
        ));
    }
}

#[test]
fn rejects_cut_count_mismatch_before_projecting_any_block() {
    let mut fixture = RangeFixture::new(17, 17, 1);
    fixture.blocks[0].comet_header.version.app = APP_VERSION + 1;
    let leaves = [cut_for(&fixture.blocks[0], 31)];
    let statement = fixture.statement(31, 31, &leaves);
    assert!(matches!(
        fixture.verify_statement(&statement, &[31]),
        Err(RangeStatementProjectionError::Statement(
            LandStatementError::CutCount
        ))
    ));
}

#[test]
fn rejects_statement_substitutions_for_each_bound_field() {
    type StatementSubstitution = fn(&mut LandStatement<'_>);

    let fixture = RangeFixture::new(17, 17, 1);
    let cases: [(&str, StatementSubstitution); 11] = [
        ("l1_chain_id", |s| s.l1_chain_id += U256::ONE << 255),
        ("l2_chain_id", |s| s.l2_chain_id += U256::ONE << 192),
        ("inbox", |s| s.inbox = Address::ZERO),
        ("generation", |s| s.generation += U256::ONE << 128),
        ("parent_height", |s| s.input.parent_height -= U256::ONE),
        ("last_height", |s| s.input.last_height += U256::ONE),
        ("parent_hash", |s| s.parent_hash = B256::ZERO),
        ("parent_comet_hash", |s| s.parent_comet_hash = B256::ZERO),
        ("last_block_hash", |s| s.input.last_block_hash = B256::ZERO),
        ("last_comet_hash", |s| s.input.last_comet_hash = B256::ZERO),
        ("last_state_root", |s| {
            s.input.last_state_root = B256::repeat_byte(0xee);
        }),
    ];
    for (name, substitute) in cases {
        let mut statement = fixture.statement(31, 31, &[]);
        substitute(&mut statement);
        assert!(
            matches!(
                fixture.verify_statement(&statement, &[31]),
                Err(RangeStatementProjectionError::StatementMismatch { field }) if field == name
            ),
            "{name}"
        );
    }
}

#[test]
fn rejects_missing_extra_and_out_of_bounds_cursors() {
    let fixture = RangeFixture::new(17, 17, 1);
    let statement = fixture.statement(31, 31, &[]);
    for cursors in [&[][..], &[31, 31][..]] {
        assert!(matches!(
            fixture.verify_statement(&statement, cursors),
            Err(RangeStatementProjectionError::CursorCount { expected: 1, actual })
                if actual == cursors.len()
        ));
    }
    assert!(matches!(
        fixture.verify_statement(&statement, &[UINT48_MAX + 1]),
        Err(RangeStatementProjectionError::CursorBounds { height: 18 })
    ));
    let mut statement = statement;
    statement.forced_inclusion_head_before = U256::ONE << 200;
    assert!(matches!(
        fixture.verify_statement(&statement, &[31]),
        Err(RangeStatementProjectionError::OriginCursor)
    ));
}

#[test]
fn rejects_decreasing_or_skipped_cursors_and_mismatched_endpoints() {
    let fixture = RangeFixture::new(17, 17, 3);
    let leaves = [
        cut_for(&fixture.blocks[0], 31),
        cut_for(&fixture.blocks[1], 32),
    ];

    for (before, cursors, invalid_height) in [
        (30, [31, 30, 31], 19),
        (30, [31, 33, 33], 19),
        (32, [31, 32, 32], 18),
    ] {
        let statement = fixture.statement(before, cursors[2], &leaves);
        assert!(matches!(
            fixture.verify_statement(&statement, &cursors),
            Err(RangeStatementProjectionError::CursorStep { height, .. }) if height == invalid_height
        ));
    }

    for after in [33, UINT48_MAX + 1] {
        let statement = fixture.statement(30, after, &leaves);
        assert!(matches!(
            fixture.verify_statement(&statement, &[31, 32, 32]),
            Err(RangeStatementProjectionError::TerminalCursor)
        ));
    }
}

#[test]
fn retains_range_rejection_for_resealed_comet_and_execution_links() {
    for index in [0, 1] {
        let mut fixture = RangeFixture::new(17, 20, 3);
        fixture.blocks[index].comet_header.last_block_id = Some(comet_parent(B256::ZERO));
        fixture.relink_comet_after(index);
        let leaves = [
            cut_for(&fixture.blocks[0], 31),
            cut_for(&fixture.blocks[1], 31),
        ];
        let statement = fixture.statement(31, 31, &leaves);
        let expected_height = fixture.blocks[index].execution_header.number;
        assert!(matches!(
            fixture.verify_statement(&statement, &[31, 31, 31]),
            Err(RangeStatementProjectionError::Range(RangeProjectionError::Projection {
                height,
                source: ProjectionError::CometParent,
            })) if height == expected_height
        ));

        let mut fixture = RangeFixture::new(17, 20, 3);
        fixture.blocks[index].execution_header.parent_hash = B256::ZERO;
        fixture.reseal_execution_from(index);
        let leaves = [
            cut_for(&fixture.blocks[0], 31),
            cut_for(&fixture.blocks[1], 31),
        ];
        let statement = fixture.statement(31, 31, &leaves);
        assert!(matches!(
            fixture.verify_statement(&statement, &[31, 31, 31]),
            Err(RangeStatementProjectionError::Range(RangeProjectionError::Projection {
                height,
                source: ProjectionError::ExecutionParent,
            })) if height == expected_height
        ));
    }
}

#[test]
fn includes_external_l1_fields_and_blob_order_in_statement_digest() {
    let fixture = RangeFixture::new(17, 17, 1);
    let blobs = [B256::repeat_byte(0x70), B256::repeat_byte(0x71)];
    let reversed_blobs = [blobs[1], blobs[0]];
    let baseline = || LandStatement {
        blob_hashes: &blobs,
        ..fixture.statement(31, 31, &[])
    };
    let (_, digest) = fixture.verify_statement(&baseline(), &[31]).unwrap();
    for statement in [
        LandStatement {
            config_hash: B256::repeat_byte(0x33),
            ..baseline()
        },
        LandStatement {
            l1_reference_block_hash: B256::repeat_byte(0x34),
            ..baseline()
        },
        LandStatement {
            blob_hashes: &reversed_blobs,
            ..baseline()
        },
    ] {
        let (_, changed) = fixture.verify_statement(&statement, &[31]).unwrap();
        assert_eq!(changed, expected_digest(&statement));
        assert_ne!(digest, changed);
    }
}

#[test]
fn projects_three_consecutive_blocks_and_preserves_original_spans() {
    let fixture = RangeFixture::new(17, 17, 3);
    let projections = verify_range_projection(
        &fixture.inputs(),
        &fixture.context,
        &fixture.parent_execution_header,
    )
    .expect("the complete linked range must project");

    assert_eq!(projections.len(), 3);
    for ((projection, block), transaction_count) in
        projections.iter().zip(&fixture.blocks).zip([0, 1, 2])
    {
        assert_eq!(
            projection.execution_hash(),
            block.execution_header.hash_slow()
        );
        assert_eq!(
            projection.anchor_number(),
            FIRST_ANCHOR_NUMBER + u64::try_from(transaction_count).unwrap()
        );
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
        assert_eq!(projections[0].anchor_witness(), Some(ANCHOR));
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
        verify_range_projection(&missing, &fixture.context, &fixture.parent_execution_header),
        Err(RangeProjectionError::BlockCount {
            expected: 3,
            actual: 2
        })
    ));
    let extra = [first.input(), second.input(), third.input(), third.input()];
    assert!(matches!(
        verify_range_projection(&extra, &fixture.context, &fixture.parent_execution_header),
        Err(RangeProjectionError::BlockCount {
            expected: 3,
            actual: 4
        })
    ));
    let duplicate = [first.input(), first.input(), third.input()];
    assert!(matches!(
        verify_range_projection(
            &duplicate,
            &fixture.context,
            &fixture.parent_execution_header
        ),
        Err(RangeProjectionError::Projection {
            height: 19,
            source: ProjectionError::Height,
        })
    ));
    let reordered = [second.input(), first.input(), third.input()];
    assert!(matches!(
        verify_range_projection(
            &reordered,
            &fixture.context,
            &fixture.parent_execution_header
        ),
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
    fixture.blocks[1].execution_header.extra_data =
        hex!("6400000000000300000001020300000000000000000000000000000000").into();
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
            verify_range_projection(&inputs, &context, &fixture.parent_execution_header),
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

#[test]
fn projects_an_inherited_anchor_between_advances() {
    let mut fixture = RangeFixture::new(17, 17, 4);
    let previous_root = fixture.blocks[1].execution_header.parent_beacon_block_root;
    fixture.set_anchor(2, FIRST_ANCHOR_NUMBER + 1, previous_root, false);

    let projections = fixture.verify().unwrap();
    assert_eq!(projections.len(), 4);
    assert_eq!(projections[2].anchor_number(), FIRST_ANCHOR_NUMBER + 1);
    assert_eq!(projections[2].anchor_witness(), None);
    assert_eq!(projections[3].anchor_number(), FIRST_ANCHOR_NUMBER + 3);
    assert_eq!(projections[3].anchor_witness(), Some(ANCHOR));
}

#[test]
fn rejects_a_missing_witness_in_an_origin_singleton() {
    for origin in [0, 17] {
        let mut fixture = RangeFixture::new(origin, origin, 1);
        let root = fixture.blocks[0].execution_header.parent_beacon_block_root;
        fixture.set_anchor(0, FIRST_ANCHOR_NUMBER, root, false);

        let error = fixture
            .verify()
            .expect_err("the first block of a generation must carry an anchor witness");
        assert!(
            matches!(error, RangeProjectionError::MissingAnchorWitness { height } if height == origin + 1),
            "{error:?}"
        );
    }
}

#[test]
fn rejects_a_missing_witness_on_an_anchor_advance() {
    let mut fixture = RangeFixture::new(17, 20, 2);
    let root = fixture.blocks[1].execution_header.parent_beacon_block_root;
    fixture.set_anchor(1, FIRST_ANCHOR_NUMBER + 1, root, false);

    let error = fixture
        .verify()
        .expect_err("an anchor advance must carry an anchor witness");
    assert!(
        matches!(
            error,
            RangeProjectionError::MissingAnchorWitness { height: 22 }
        ),
        "{error:?}"
    );
}

#[test]
fn rejects_a_witness_for_an_inherited_anchor() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    let previous_root = fixture.blocks[0].execution_header.parent_beacon_block_root;
    fixture.set_anchor(1, FIRST_ANCHOR_NUMBER, previous_root, true);

    let error = fixture
        .verify()
        .expect_err("an inherited anchor must omit the witness");
    assert!(
        matches!(
            error,
            RangeProjectionError::UnexpectedAnchorWitness { height: 19 }
        ),
        "{error:?}"
    );
}

#[test]
fn rejects_an_anchor_number_regression_even_with_a_witness() {
    let mut fixture = RangeFixture::new(17, 17, 2);
    let root = fixture.blocks[1].execution_header.parent_beacon_block_root;
    fixture.set_anchor(1, FIRST_ANCHOR_NUMBER - 1, root, true);

    let error = fixture
        .verify()
        .expect_err("a new witness cannot permit a regressing anchor number");
    assert!(
        matches!(
            error,
            RangeProjectionError::AnchorRegression {
                height: 19,
                previous: FIRST_ANCHOR_NUMBER,
                actual,
            } if actual == FIRST_ANCHOR_NUMBER - 1
        ),
        "{error:?}"
    );
}

#[test]
fn requires_matching_present_nonzero_roots_for_inheritance() {
    let root = Some(B256::repeat_byte(0x61));
    let zero = Some(B256::ZERO);
    // Keep the preceding covered block valid so this reaches inheritance.
    // Missing/zero carried roots are rejected earlier and tested separately.
    for actual in [Some(B256::repeat_byte(0x62)), None, zero] {
        let previous = root;
        let mut fixture = RangeFixture::new(17, 17, 2);
        fixture.set_anchor(0, FIRST_ANCHOR_NUMBER, previous, true);
        fixture.set_anchor(1, FIRST_ANCHOR_NUMBER, actual, false);

        let error = fixture
            .verify()
            .expect_err("inherited anchor roots must be present, nonzero and equal");
        assert!(
            matches!(
                error,
                RangeProjectionError::InheritedAnchorRoot { height: 19 }
            ),
            "{previous:?} to {actual:?}: {error:?}"
        );
    }
}

#[test]
fn projects_an_inherited_anchor_from_the_excluded_parent() {
    let mut fixture = RangeFixture::new(17, 20, 2);
    let root = fixture.parent_execution_header.parent_beacon_block_root;
    fixture.set_anchor(0, FIRST_ANCHOR_NUMBER - 1, root, false);

    let projections = fixture.verify().unwrap();
    assert_eq!(projections[0].anchor_witness(), None);
    assert_eq!(projections[1].anchor_witness(), Some(ANCHOR));
}

#[test]
fn propagates_anchor_transition_failure_through_landing_statement() {
    let mut fixture = RangeFixture::new(17, 20, 2);
    let root = fixture.blocks[1].execution_header.parent_beacon_block_root;
    fixture.set_anchor(1, FIRST_ANCHOR_NUMBER + 1, root, false);
    let leaves = [cut_for(&fixture.blocks[0], 31)];
    let statement = fixture.statement(31, 31, &leaves);

    let error = fixture
        .verify_statement(&statement, &[31, 31])
        .expect_err("statement verification must also check anchor transitions");
    assert!(
        matches!(
            error,
            RangeStatementProjectionError::Range(RangeProjectionError::MissingAnchorWitness {
                height: 22,
            })
        ),
        "{error:?}"
    );
}

#[test]
fn binds_parent_height_even_when_its_hash_matches() {
    for parent_height in [17, 20] {
        let mut fixture = RangeFixture::new(17, parent_height, 1);
        fixture.parent_execution_header.number += 1;
        fixture.bind_parent();

        let error = fixture
            .verify()
            .expect_err("the excluded parent's height must match");
        assert!(
            matches!(error, RangeProjectionError::ParentExecutionHeight { expected, actual }
                if expected == parent_height && actual == parent_height + 1),
            "{error:?}"
        );
    }
}

#[test]
fn rejects_changes_to_the_committed_parent_header() {
    for parent_height in [17, 20] {
        for changed_field in ["state root", "anchor root", "extra data"] {
            let mut fixture = RangeFixture::new(17, parent_height, 1);
            let parent = &mut fixture.parent_execution_header;
            match changed_field {
                "state root" => parent.state_root = B256::repeat_byte(0x80),
                "anchor root" => parent.parent_beacon_block_root = Some(B256::repeat_byte(0x81)),
                "extra data" => parent.extra_data = vec![0x82; 13].into(),
                _ => unreachable!(),
            }

            let error = fixture
                .verify()
                .expect_err("the excluded parent's hash must match");
            assert!(
                matches!(error, RangeProjectionError::ParentExecutionHash),
                "changed {changed_field}: {error:?}"
            );
        }
    }
}

#[test]
fn checks_bound_mid_generation_parent_extra_data() {
    let original = anchor_extra_data(2, FIRST_ANCHOR_NUMBER - 1);
    let mut wrong_fee = original;
    wrong_fee[0] = 99;
    let mut longer = original.to_vec();
    longer.push(0);

    for (label, extra_data) in [
        ("short", original[..12].to_vec()),
        ("long", longer),
        ("fee share", wrong_fee.to_vec()),
        (
            "generation high byte",
            anchor_extra_data(0x0100_0000_0002, FIRST_ANCHOR_NUMBER - 1).to_vec(),
        ),
    ] {
        let mut fixture = RangeFixture::new(17, 20, 1);
        fixture.parent_execution_header.extra_data = extra_data.into();
        fixture.bind_parent();

        let error = fixture
            .verify()
            .expect_err("mid-generation parent must have Etna extra data");
        assert!(
            matches!(error, RangeProjectionError::ParentExecutionExtraData),
            "invalid {label}: {error:?}"
        );
    }
}

#[test]
fn projects_a_generation_origin_with_legacy_parent_extra_data() {
    let mut fixture = RangeFixture::new(17, 17, 1);
    fixture.parent_execution_header.extra_data = vec![0x83; 32].into();
    fixture.bind_parent();

    let projections = fixture.verify().unwrap();
    assert_eq!(projections[0].anchor_witness(), Some(ANCHOR));
}

#[test]
fn checks_uint48_anchor_extremes_on_the_incoming_edge() {
    let root = Some(B256::repeat_byte(0x84));
    for (previous, current, carries) in [
        (0, 0, false),
        (0, UINT48_MAX, true),
        (UINT48_MAX, UINT48_MAX, false),
    ] {
        let mut fixture = RangeFixture::new(17, UINT48_MAX - 1, 1);
        fixture.set_parent_anchor(previous, root);
        fixture.set_anchor(0, current, root, carries);

        let projections = fixture.verify().unwrap();
        assert_eq!(projections[0].anchor_number(), current);
        assert_eq!(projections[0].anchor_witness(), carries.then_some(ANCHOR));
    }
}

#[test]
fn rejects_an_incoming_anchor_number_regression() {
    let mut fixture = RangeFixture::new(17, 20, 1);
    let root = fixture.blocks[0].execution_header.parent_beacon_block_root;
    fixture.set_anchor(0, FIRST_ANCHOR_NUMBER - 2, root, true);

    let error = fixture
        .verify()
        .expect_err("the first anchor must not regress");
    assert!(
        matches!(
            error,
            RangeProjectionError::AnchorRegression {
                height: 21,
                previous,
                actual,
            } if previous == FIRST_ANCHOR_NUMBER - 1 && actual == FIRST_ANCHOR_NUMBER - 2
        ),
        "{error:?}"
    );
}

#[test]
fn rejects_omitting_a_witness_for_an_incoming_anchor_advance() {
    let mut fixture = RangeFixture::new(17, 20, 1);
    let root = fixture.blocks[0].execution_header.parent_beacon_block_root;
    fixture.set_anchor(0, FIRST_ANCHOR_NUMBER, root, false);

    let error = fixture
        .verify()
        .expect_err("an incoming advance needs a witness");
    assert!(
        matches!(
            error,
            RangeProjectionError::MissingAnchorWitness { height: 21 }
        ),
        "{error:?}"
    );
}

#[test]
fn rejects_redundant_witness_when_first_block_inherits_the_parent_anchor() {
    let mut fixture = RangeFixture::new(17, 20, 1);
    let root = fixture.parent_execution_header.parent_beacon_block_root;
    fixture.set_anchor(0, FIRST_ANCHOR_NUMBER - 1, root, true);

    let error = fixture
        .verify()
        .expect_err("incoming inheritance must omit the witness");
    assert!(
        matches!(
            error,
            RangeProjectionError::UnexpectedAnchorWitness { height: 21 }
        ),
        "{error:?}"
    );
}

#[test]
fn checks_inherited_roots_on_the_incoming_edge() {
    let root = Some(B256::repeat_byte(0x85));
    let zero = Some(B256::ZERO);
    for (previous, actual) in [
        (root, Some(B256::repeat_byte(0x86))),
        (None, root),
        (root, None),
        (None, None),
        (zero, root),
        (root, zero),
        (zero, zero),
    ] {
        let mut fixture = RangeFixture::new(17, 20, 1);
        let anchor = FIRST_ANCHOR_NUMBER - 1;
        fixture.set_parent_anchor(anchor, previous);
        fixture.set_anchor(0, anchor, actual, false);

        let error = fixture
            .verify()
            .expect_err("incoming inheritance needs matching nonzero roots");
        assert!(
            matches!(
                error,
                RangeProjectionError::InheritedAnchorRoot { height: 21 }
            ),
            "{previous:?} to {actual:?}: {error:?}"
        );
    }
}

#[test]
fn rejects_a_typed_parent_root_alias_of_the_committed_withdrawals_root() {
    let mut fixture = RangeFixture::new(17, 20, 1);
    let parent = &mut fixture.parent_execution_header;
    let withdrawals_root = parent.withdrawals_root;
    parent.blob_gas_used = None;
    parent.excess_blob_gas = None;
    parent.parent_beacon_block_root = None;
    fixture.bind_parent();
    fixture.set_anchor(0, FIRST_ANCHOR_NUMBER - 1, withdrawals_root, false);

    // These two optional B256 slots serialize identically when the earlier
    // slot and the intervening integer slots are omitted.
    fixture.parent_execution_header.withdrawals_root = None;
    fixture.parent_execution_header.parent_beacon_block_root = withdrawals_root;
    assert_eq!(
        fixture.parent_execution_header.hash_slow(),
        fixture.context.parent_el_hash
    );
    let error = fixture
        .verify()
        .expect_err("inheritance must use the committed anchor root");
    assert!(
        matches!(
            error,
            RangeProjectionError::ParentExecutionEncoding(ProjectionError::HeaderRoundTrip)
        ),
        "{error:?}"
    );
}

#[test]
fn propagates_a_parent_hash_failure_through_the_landing_statement() {
    let mut fixture = RangeFixture::new(17, 20, 2);
    fixture.parent_execution_header.state_root = B256::repeat_byte(0x87);
    let leaves = [cut_for(&fixture.blocks[0], 31)];
    let statement = fixture.statement(31, 31, &leaves);

    let error = fixture
        .verify_statement(&statement, &[31, 31])
        .expect_err("the statement verifier must also bind the excluded parent");
    assert!(
        matches!(
            error,
            RangeStatementProjectionError::Range(RangeProjectionError::ParentExecutionHash)
        ),
        "{error:?}"
    );
}

#[test]
fn rejects_missing_or_zero_carried_roots_at_generation_start() {
    for origin in [0, 17] {
        for root in [None, Some(B256::ZERO)] {
            let mut fixture = RangeFixture::new(origin, origin, 1);
            fixture.set_anchor(0, FIRST_ANCHOR_NUMBER, root, true);
            let error = fixture
                .verify()
                .expect_err("a carried anchor needs a nonzero root");
            assert!(
                matches!(error, RangeProjectionError::CarriedAnchorRoot { height } if height == origin + 1),
                "{error:?}"
            );
        }
    }
}

#[test]
fn rejects_missing_or_zero_carried_roots_on_advances() {
    for (parent, index) in [(17, 1), (20, 0), (20, 1)] {
        for root in [None, Some(B256::ZERO)] {
            let mut fixture = RangeFixture::new(17, parent, 2);
            let anchor = FIRST_ANCHOR_NUMBER + u64::try_from(index).unwrap();
            fixture.set_anchor(index, anchor, root, true);
            let error = fixture
                .verify()
                .expect_err("an advancing anchor needs a nonzero root");
            assert!(
                matches!(error, RangeProjectionError::CarriedAnchorRoot { height } if height == parent + u64::try_from(index).unwrap() + 1),
                "{error:?}"
            );
        }
    }
}
