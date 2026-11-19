//! Behavioral tests for the proof-specific Etna execution projection.
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
use raiko2_protocol_etna::projection::{
    ExecutionProjection, ProjectionContext, ProjectionError, verify_execution_projection,
};
use tendermint::Hash;

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

fn envelope(fields: &[&[u8]]) -> Vec<u8> {
    let mut bytes = vec![6];
    bytes.extend(list(fields));
    bytes
}

fn execution_item(header: &Header, transactions: &[TxEnvelope]) -> Vec<u8> {
    let encoded: Vec<_> = transactions
        .iter()
        .map(|tx| alloy_rlp::encode(tx.encoded_2718().as_slice()))
        .collect();
    let items: Vec<_> = encoded.iter().map(Vec::as_slice).collect();
    list(&[&alloy_rlp::encode(header), &list(&items)])
}

fn signed_transactions() -> [TxEnvelope; 2] {
    // Public signed transactions from Alloy consensus 2.4.2's
    // test_recover_with_buf_multiple_tx_types.
    [
        include_str!("fixtures/projection-legacy.hex"),
        include_str!("fixtures/projection-eip1559.hex"),
    ]
    .map(|encoded| TxEnvelope::decode_2718_exact(&hex::decode(encoded.trim()).unwrap()).unwrap())
}

struct Fixture {
    comet_header: tendermint::block::Header,
    comet_transactions: [Vec<u8>; 1],
    execution_header: Header,
    context: ProjectionContext,
}

impl Fixture {
    fn new(transactions: &[TxEnvelope]) -> Self {
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
            beneficiary: Address::repeat_byte(0x42),
            extra_data: hex!("64000000000002000000010203").into(),
            transactions_root: calculate_transaction_root(transactions),
            ..Default::default()
        };
        let raw = hex::decode(include_str!("fixtures/first-block.hex").trim_end()).unwrap();
        let (_, block) = decode_canonical_block(&raw).unwrap();
        let mut comet_header = block.header;
        comet_header.chain_id = chain_id(&domain).unwrap().parse().unwrap();
        comet_header.height = 18u64.try_into().unwrap();
        comet_header.version.app = APP_VERSION;
        comet_header.last_block_id = None;
        comet_header.app_hash = vec![0x61, 0x62, 0x63].try_into().unwrap();

        let mut fixture = Self {
            comet_header,
            comet_transactions: [Vec::new()],
            execution_header,
            context: ProjectionContext {
                domain,
                origin_height: 17,
                height: 18,
                expected_comet_hash: B256::ZERO,
                parent_comet_hash: B256::repeat_byte(0x30),
                parent_el_hash,
                max_envelope_bytes: 130_000,
            },
        };
        fixture.seal_execution(transactions);
        fixture
    }

    fn seal_execution(&mut self, transactions: &[TxEnvelope]) {
        self.seal_envelope(envelope(&[
            &execution_item(&self.execution_header, transactions),
            &[0xc0],
            &[0xc0],
            &[0xc0],
        ]));
    }

    // Independent test equality context. This fixture does not authenticate a QC.
    fn seal_envelope(&mut self, bytes: Vec<u8>) {
        self.comet_transactions = [bytes];
        self.comet_header.data_hash = Some(data_hash(&self.comet_transactions));
        self.seal_header();
    }

    fn seal_header(&mut self) {
        self.context.expected_comet_hash =
            B256::from_slice(header_hash(&self.comet_header).as_bytes());
    }

    fn verify(
        &self,
        transactions: &[TxEnvelope],
    ) -> Result<ExecutionProjection<'_>, ProjectionError> {
        verify_execution_projection(
            &self.comet_header,
            &self.comet_transactions,
            &self.execution_header,
            transactions,
            &self.context,
        )
    }
}

#[test]
fn recovery_origin_binds_exact_execution_item_with_opaque_app_hash() {
    let fixture = Fixture::new(&[]);
    let projection = fixture
        .verify(&[])
        .expect("recovery origin with a committed non-parent AppHash must bind");
    assert_eq!(
        projection.execution_item_bytes(),
        execution_item(&fixture.execution_header, &[])
    );
    assert_eq!(
        projection.execution_hash(),
        fixture.execution_header.hash_slow()
    );
    assert_eq!(projection.anchor_number(), 0x01_02_03);
    assert_eq!(projection.anchor_witness(), None);
    assert_eq!(projection.committee_witness(), None);
    assert_eq!(projection.forced_witness(), None);
}

#[test]
fn later_height_binds_ordered_legacy_and_typed_transactions() {
    let transactions = signed_transactions();
    let mut fixture = Fixture::new(&transactions);
    fixture.context.origin_height -= 1;
    fixture.comet_header.last_block_id = Some(tendermint::block::Id {
        hash: Hash::Sha256(fixture.context.parent_comet_hash.into()),
        part_set_header: tendermint::block::parts::Header::default(),
    });
    fixture.seal_header();
    let projection = fixture.verify(&transactions).unwrap();
    assert_eq!(
        projection.execution_item_bytes(),
        execution_item(&fixture.execution_header, &transactions)
    );
    let reordered = [transactions[1].clone(), transactions[0].clone()];
    assert!(matches!(
        fixture.verify(&reordered),
        Err(ProjectionError::ExecutionTransactions)
    ));
    assert!(matches!(
        fixture.verify(&transactions[..1]),
        Err(ProjectionError::ExecutionTransactions)
    ));
    let extra = [
        transactions[0].clone(),
        transactions[1].clone(),
        transactions[0].clone(),
    ];
    assert!(matches!(
        fixture.verify(&extra),
        Err(ProjectionError::ExecutionTransactions)
    ));

    fixture.context.parent_comet_hash = B256::repeat_byte(0x31);
    assert!(matches!(
        fixture.verify(&transactions),
        Err(ProjectionError::CometParent)
    ));
    fixture.context.origin_height += 1;
    assert!(matches!(
        fixture.verify(&transactions),
        Err(ProjectionError::CometParent)
    ));
}

#[test]
fn transaction_root_must_match_even_with_committed_header_and_transactions() {
    let transactions = signed_transactions();
    let mut fixture = Fixture::new(&transactions);
    fixture.execution_header.transactions_root = B256::ZERO;
    fixture.seal_execution(&transactions);
    assert!(matches!(
        fixture.verify(&transactions),
        Err(ProjectionError::TransactionRoot)
    ));
}

#[test]
fn legacy_transaction_requires_byte_string_wrapper() {
    let [legacy, _] = signed_transactions();
    let mut fixture = Fixture::new(core::slice::from_ref(&legacy));
    let item = list(&[
        &alloy_rlp::encode(&fixture.execution_header),
        &list(&[&legacy.encoded_2718()]),
    ]);
    fixture.seal_envelope(envelope(&[&item, &[0xc0], &[0xc0], &[0xc0]]));
    assert!(matches!(
        fixture.verify(&[legacy]),
        Err(ProjectionError::Rlp(_))
    ));
}

#[test]
fn witness_substitution_requires_new_data_and_header_commitments() {
    let mut fixture = Fixture::new(&[]);
    let original_hash = fixture.context.expected_comet_hash;
    let item = execution_item(&fixture.execution_header, &[]);
    let replacement = envelope(&[&item, &[0xc1, 0xc0], &[0xc0], &[0xc0]]);
    fixture.comet_transactions = [replacement.clone()];
    assert!(matches!(
        fixture.verify(&[]),
        Err(ProjectionError::DataHash)
    ));

    fixture.seal_envelope(replacement);
    fixture.context.expected_comet_hash = original_hash;
    assert!(matches!(
        fixture.verify(&[]),
        Err(ProjectionError::CometHash)
    ));
}

#[test]
fn nested_witness_fields_are_returned_opaque_for_strict_verification() {
    let mut fixture = Fixture::new(&[]);
    let item = execution_item(&fixture.execution_header, &[]);
    // The inner list boundary is valid, but its single byte cannot begin a complete item.
    let malformed_inner = [0xc1, 0xb8];
    let optional = list(&[&malformed_inner]);
    fixture.seal_envelope(envelope(&[&item, &optional, &optional, &optional]));
    let projection = fixture.verify(&[]).unwrap();
    assert_eq!(
        projection.anchor_witness(),
        Some(malformed_inner.as_slice())
    );
    assert_eq!(
        projection.committee_witness(),
        Some(malformed_inner.as_slice())
    );
    assert_eq!(
        projection.forced_witness(),
        Some(malformed_inner.as_slice())
    );
}

#[test]
fn rejects_wrong_boundary_kinds_arity_and_noncanonical_lengths() {
    let mut fixture = Fixture::new(&[]);
    let header = alloy_rlp::encode(&fixture.execution_header);
    let item = execution_item(&fixture.execution_header, &[]);
    let empty = &[0xc0];
    let wanted = envelope(&[&item, empty, empty, empty]);
    let mut trailing = wanted.clone();
    trailing.push(0xc0);
    let mut wrong_version = wanted;
    wrong_version[0] = 5;
    for malformed in [
        wrong_version,
        trailing,
        vec![6, 0x80],
        vec![6, 0xf8, 0xff],
        envelope(&[&item, empty, empty]),
        envelope(&[&item, empty, empty, empty, empty]),
        envelope(&[&item, &[0x80], empty, empty]),
        envelope(&[&item, &[0xc1, 0x80], empty, empty]),
        envelope(&[&item, &[0xc2, 0xc0, 0xc0], empty, empty]),
        envelope(&[&item, &[0xf8, 0], empty, empty]),
        envelope(&[&item, &[0xc1, 0xc1], empty, empty]),
        envelope(&[&list(&[&header]), empty, empty, empty]),
        envelope(&[&list(&[&header, empty, empty]), empty, empty, empty]),
        envelope(&[&list(&[&header, &[0x80]]), empty, empty, empty]),
        envelope(&[&list(&[&[0x80], empty]), empty, empty, empty]),
    ] {
        fixture.seal_envelope(malformed);
        assert!(fixture.verify(&[]).is_err());
    }
}

#[test]
fn typed_header_must_match_and_round_trip_with_positional_suffix() {
    let mut fixture = Fixture::new(&[]);
    fixture.execution_header.beneficiary = Address::repeat_byte(0x43);
    assert!(matches!(
        fixture.verify(&[]),
        Err(ProjectionError::ExecutionHeader)
    ));

    // The encoder packs the excess blob gas in the blob-gas-used position.
    fixture.execution_header.base_fee_per_gas = Some(1);
    fixture.execution_header.withdrawals_root = Some(B256::ZERO);
    fixture.execution_header.excess_blob_gas = Some(1);
    fixture.seal_execution(&[]);
    assert!(matches!(
        fixture.verify(&[]),
        Err(ProjectionError::HeaderRoundTrip)
    ));
}

#[test]
fn committed_headers_still_require_expected_context() {
    let mut fixture = Fixture::new(&[]);
    fixture.comet_header.version.app += 1;
    fixture.seal_header();
    assert!(matches!(
        fixture.verify(&[]),
        Err(ProjectionError::AppVersion)
    ));

    fixture.comet_header.version.app = APP_VERSION;
    fixture.comet_header.chain_id = "another-chain".parse().unwrap();
    fixture.seal_header();
    assert!(matches!(fixture.verify(&[]), Err(ProjectionError::ChainId)));

    let mut fixture = Fixture::new(&[]);
    fixture.comet_header.height = 19u64.try_into().unwrap();
    fixture.seal_header();
    assert!(matches!(fixture.verify(&[]), Err(ProjectionError::Height)));

    let mut fixture = Fixture::new(&[]);
    fixture.execution_header.number += 1;
    fixture.seal_execution(&[]);
    assert!(matches!(fixture.verify(&[]), Err(ProjectionError::Height)));

    let mut fixture = Fixture::new(&[]);
    fixture.execution_header.parent_hash = B256::ZERO;
    fixture.seal_execution(&[]);
    assert!(matches!(
        fixture.verify(&[]),
        Err(ProjectionError::ExecutionParent)
    ));

    let mut fixture = Fixture::new(&[]);
    for malformed in [
        hex!("63000000000002000000010203").to_vec(),
        hex!("64000000000003000000010203").to_vec(),
        hex!("640000000000020000000102").to_vec(),
        vec![0x64; 124_001],
    ] {
        fixture.execution_header.extra_data = malformed.into();
        assert!(matches!(
            fixture.verify(&[]),
            Err(ProjectionError::ExtraData)
        ));
    }
}

#[test]
fn uint48_limits_apply_without_truncation() {
    let max = (1 << 48) - 1;
    let mut fixture = Fixture::new(&[]);
    fixture.context.origin_height = max - 1;
    fixture.context.height = max;
    fixture.context.domain.generation = max;
    fixture.comet_header.height = max.try_into().unwrap();
    fixture.comet_header.chain_id = chain_id(&fixture.context.domain).unwrap().parse().unwrap();
    fixture.execution_header.number = max;
    fixture.execution_header.extra_data = hex!("64ffffffffffffffffffffffff").into();
    fixture.seal_execution(&[]);
    assert_eq!(fixture.verify(&[]).unwrap().anchor_number(), max);

    fixture.context.domain.generation = max + 1;
    assert!(matches!(
        fixture.verify(&[]),
        Err(ProjectionError::Domain(_))
    ));
    fixture.context.height = max + 1;
    assert!(matches!(fixture.verify(&[]), Err(ProjectionError::Height)));
    fixture.context.origin_height = u64::MAX;
    assert!(matches!(fixture.verify(&[]), Err(ProjectionError::Height)));
}

#[test]
fn bounds_complete_execution_item_and_envelope_independently() {
    let [_, TxEnvelope::Eip1559(mut signed)] = signed_transactions() else {
        panic!("expected a typed transaction fixture");
    };
    let mut fixture = Fixture::new(&[]);
    // A long calldata string keeps all affected RLP prefix widths stable. The
    // signature bytes are carried for codec tests, without any signer assertion.
    signed.tx_mut().input = vec![0xa5; 120_000].into();
    let probe = execution_item(
        &fixture.execution_header,
        &[TxEnvelope::Eip1559(signed.clone())],
    );
    let at_limit = 120_000 + (124_000 - probe.len());

    for (size, data_len) in [(124_000, at_limit), (124_001, at_limit + 1)] {
        signed.tx_mut().input = vec![0xa5; data_len].into();
        let transactions = [TxEnvelope::Eip1559(signed.clone())];
        fixture.execution_header.transactions_root = calculate_transaction_root(&transactions);
        let item = execution_item(&fixture.execution_header, &transactions);
        assert_eq!(item.len(), size);
        fixture.seal_execution(&transactions);
        fixture.context.max_envelope_bytes = fixture.comet_transactions[0].len();
        if size == 124_000 {
            assert_eq!(
                fixture
                    .verify(&transactions)
                    .unwrap()
                    .execution_item_bytes(),
                item
            );
            fixture.context.max_envelope_bytes -= 1;
            assert!(matches!(
                fixture.verify(&transactions),
                Err(ProjectionError::EnvelopeSize)
            ));
        } else {
            assert!(matches!(
                fixture.verify(&transactions),
                Err(ProjectionError::ExecutionItemSize)
            ));
        }
    }
}

#[test]
fn rejects_empty_and_multiple_comet_transactions() {
    let fixture = Fixture::new(&[]);
    let duplicate = [
        fixture.comet_transactions[0].clone(),
        fixture.comet_transactions[0].clone(),
    ];
    for supplied in [&[][..], &duplicate[..]] {
        assert!(matches!(
            verify_execution_projection::<TxEnvelope>(
                &fixture.comet_header,
                supplied,
                &fixture.execution_header,
                &[],
                &fixture.context,
            ),
            Err(ProjectionError::CometTransactionCount)
        ));
    }
}
