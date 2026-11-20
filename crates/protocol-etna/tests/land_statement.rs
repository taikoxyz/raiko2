//! Golden ABI vectors for the Etna land statement and cuts.

use alloy_primitives::{B256, U256, address, b256, hex};
use alloy_sol_types::SolValue;
use etna_core::schedule::UINT48_MAX;
use raiko2_protocol_etna::statement::{
    LandInput, LandStatement, LandStatementError, cut_hash, cut_leaf,
};

const FIRST_LEAF: B256 = b256!("28e9b5992610c677081b2c4a3ba51d9ffea50c5e8cac87c1729b3dca5a78e89d");
const SECOND_LEAF: B256 = b256!("3fedf08b3ce07640e06b80b4cadef13c394b341d290abd82724d16ac57159d16");
const LEAVES: [B256; 2] = [FIRST_LEAF, SECOND_LEAF];
const TWO_LEAF_CUT: B256 =
    b256!("73e5703132dd6659749643ba340d403822b2ff8d3bc5b4ade2c22cbc8aa21b18");

const BLOBS: [B256; 2] = [
    b256!("0177777777777777777777777777777777777777777777777777777777777777"),
    b256!("0188888888888888888888888888888888888888888888888888888888888888"),
];

fn wide_uint(bit: usize, low: u64) -> U256 {
    (U256::from(1) << bit) + U256::from(low)
}

// The high bits exercise the ABI widths independently of the validity of a
// deployment or settlement range.
fn vector_statement() -> LandStatement<'static> {
    LandStatement {
        l1_chain_id: wide_uint(255, 0x11),
        l2_chain_id: wide_uint(192, 0x12),
        inbox: address!("0102030405060708090a0b0c0d0e0f1011121314"),
        registry: address!("35363738393a3b3c3d3e3f404142434445464748"),
        generation: wide_uint(235, 0x15),
        parent_hash: B256::repeat_byte(0x17),
        parent_comet_hash: B256::repeat_byte(0x18),
        committee_config_hash: B256::repeat_byte(0x19),
        input: LandInput {
            parent_height: wide_uint(208, 0x16),
            last_height: wide_uint(208, 0x19),
            last_block_hash: B256::repeat_byte(0x1b),
            last_comet_hash: B256::repeat_byte(0x1c),
            last_state_root: B256::repeat_byte(0x1d),
            l1_reference_block_number: wide_uint(224, 0x1e),
            forced_inclusion_head: wide_uint(252, 0x54),
            active_committee_snapshot: wide_uint(240, 0x23),
        },
        l1_reference_block_hash: B256::repeat_byte(0x1f),
        blob_hashes: &BLOBS,
        forced_inclusion_head_before: wide_uint(252, 0x52),
        cut_leaves: &LEAVES,
    }
}

fn fixture(text: &str) -> Vec<u8> {
    hex::decode(text.lines().collect::<String>()).unwrap()
}

fn statement_for_range(parent: u64, last: u64, cut_leaves: &[B256]) -> LandStatement<'_> {
    let mut statement = vector_statement();
    statement.input.parent_height = U256::from(parent);
    statement.input.last_height = U256::from(last);
    LandStatement {
        cut_leaves,
        ..statement
    }
}

#[test]
fn checked_digest_rejects_a_missing_interior_cut_leaf() {
    let statement = statement_for_range(1000, 1003, &LEAVES[..1]);

    assert_eq!(statement.try_digest(), Err(LandStatementError::CutCount));
}

#[test]
fn checked_digest_matches_raw_for_a_valid_range_and_cut_count() {
    let statement = statement_for_range(1000, 1003, &LEAVES);

    assert_eq!(statement.try_digest(), Ok(statement.digest()));
}

#[test]
fn checked_digest_accepts_one_block_with_no_cuts() {
    for parent in [0, UINT48_MAX - 1] {
        let statement = statement_for_range(parent, parent + 1, &[]);

        assert_eq!(statement.try_digest(), Ok(statement.digest()));
    }
}

#[test]
fn checked_digest_rejects_an_extra_endpoint_cut_leaf() {
    let cuts = [1000, 1001, 1002, 1003].map(|height| {
        cut_leaf(
            U256::from(height),
            B256::repeat_byte(0x51),
            B256::repeat_byte(0x52),
            U256::ZERO,
        )
    });
    for leaves in [&cuts[..3], &cuts[1..]] {
        let statement = statement_for_range(1000, 1003, leaves);

        assert_eq!(statement.try_digest(), Err(LandStatementError::CutCount));
    }
}

#[test]
fn checked_digest_rejects_invalid_height_ranges() {
    for (parent, last) in [
        (1000, 1000),
        (1001, 1000),
        (UINT48_MAX, UINT48_MAX + 1),
        (UINT48_MAX + 1, UINT48_MAX + 2),
    ] {
        let statement = statement_for_range(parent, last, &[]);

        assert_eq!(
            statement.try_digest(),
            Err(LandStatementError::InvalidRange)
        );
    }
    assert_eq!(
        vector_statement().try_digest(),
        Err(LandStatementError::InvalidRange)
    );
}

#[test]
fn checked_digest_rejects_cut_count_that_would_truncate_on_32_bit() {
    let statement = statement_for_range(0, (1_u64 << 32) + 3, &LEAVES);

    assert_eq!(statement.try_digest(), Err(LandStatementError::CutCount));
}

#[test]
fn encodes_the_exact_21_word_statement_preimage() {
    let expected = fixture(include_str!("fixtures/land-statement.hex"));
    let preimage: [u8; 672] = vector_statement().preimage();

    assert_eq!(preimage.as_slice(), expected);
}

#[test]
fn hashes_the_statement_preimage_with_the_v1_layout() {
    let digest = vector_statement().digest();

    assert_eq!(
        digest,
        b256!("957553e9ac714349a0ab05c026715927edbf5ea07210ed9013bf9c4cbfed3be6")
    );
    assert_ne!(
        digest,
        b256!("ca8fc5ed98640ce4f2d1b358d10b85c597bb36b0c8e58d6342ef2ec0e0ba1f8d")
    );
}

#[test]
fn encodes_the_eight_field_land_input() {
    let expected = fixture(include_str!("fixtures/land-input.hex"));

    assert_eq!(vector_statement().input.abi_encode(), expected);
}

#[test]
fn hashes_the_four_word_cut_leaves() {
    assert_eq!(
        cut_leaf(
            wide_uint(208, 0x17),
            B256::repeat_byte(0x51),
            B256::repeat_byte(0x52),
            wide_uint(252, 0x53),
        ),
        FIRST_LEAF,
    );
    assert_eq!(
        cut_leaf(
            wide_uint(208, 0x18),
            B256::repeat_byte(0x61),
            B256::repeat_byte(0x62),
            wide_uint(252, 0x54),
        ),
        SECOND_LEAF,
    );
}

#[test]
fn hashes_packed_cut_leaves_in_slice_order() {
    let statement = vector_statement();
    let generation = statement.generation;
    let parent = statement.input.parent_height;
    let last = statement.input.last_height;
    let one = U256::from(1);

    assert_eq!(
        cut_hash(generation, parent, parent + one, &[]),
        b256!("87fceec023f7aabd0dec5d0f33b41e51c6ceec65ce230fe7dab96393824280dc"),
    );
    assert_eq!(
        cut_hash(generation, parent, last - one, &[FIRST_LEAF]),
        b256!("4c0bddfaf78a6ef310202d702c4044f111800105f64ec11cff337886f5877113"),
    );
    assert_eq!(cut_hash(generation, parent, last, &LEAVES), TWO_LEAF_CUT);
    assert_eq!(
        cut_hash(generation, parent, last, &[SECOND_LEAF, FIRST_LEAF]),
        b256!("e633fd63b8baf7ed69b50ac521ad4c409bf429ae3c600bc65d37953ee96aedb5"),
    );
}

#[test]
fn commits_the_cut_hash_to_generation_and_both_range_endpoints() {
    let statement = vector_statement();
    let generation = statement.generation;
    let parent = statement.input.parent_height;
    let last = statement.input.last_height;
    let one = U256::from(1);

    // Deliberately retain the same leaves while varying each ABI domain word.
    assert_eq!(
        cut_hash(generation + one, parent, last, &LEAVES),
        b256!("6e97367696d13dc04826f28ed51f4aeb551b3bbc19703ba41e25dc9086f33739"),
    );
    assert_eq!(
        cut_hash(generation, parent + one, last, &LEAVES),
        b256!("858a1c5ffa6402745e2d77ff233615bf9c6b10f031f31e1ad8a1be9a7353a468"),
    );
    assert_eq!(
        cut_hash(generation, parent, last + one, &LEAVES),
        b256!("36c5e1b7e88a7d8178db647e4ef79843a3b7d10afb6e1b72091295abbb6a0e00"),
    );
}
