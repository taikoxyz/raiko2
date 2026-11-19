//! Public vectors emitted by the native AF1 `RawL1Header` decoder.
//!
//! Only mainnet block 26,155,639 is a real Ethereum header. The other rows
//! exercise the positional decoder or form synthetic chains for binding tests.

use alloy_primitives::{B256, Bytes, hex};
use raiko2_protocol_etna::l1::{
    L1AncestryContext, L1AncestryError, L1HeaderError, L1HeaderRef, MAX_L1_HEADER_BYTES,
    verify_l1_ancestry,
};

#[derive(Debug, PartialEq, Eq)]
struct Decoded {
    hash: B256,
    parent_hash: B256,
    state_root: B256,
    number: u64,
    timestamp: u64,
}

struct Case {
    label: &'static str,
    raw: Bytes,
    expected: Option<Decoded>,
}

impl Case {
    const fn fields(&self) -> &Decoded {
        self.expected.as_ref().expect("accepted native fixture")
    }
}

const CORPUS: &str = include_str!("fixtures/l1-native-cases.tsv");

fn parse_case(line: &'static str) -> Case {
    let mut columns = line.split('\t');
    let label = columns.next().unwrap();
    let raw = Bytes::from(hex::decode(columns.next().unwrap()).unwrap());
    let expected = match columns.next().unwrap() {
        "ok" => Some(Decoded {
            hash: columns.next().unwrap().parse().unwrap(),
            parent_hash: columns.next().unwrap().parse().unwrap(),
            state_root: columns.next().unwrap().parse().unwrap(),
            number: columns.next().unwrap().parse().unwrap(),
            timestamp: columns.next().unwrap().parse().unwrap(),
        }),
        "err" => {
            assert!(!columns.next().unwrap().is_empty());
            None
        }
        status => panic!("unknown native result {status} for {label}"),
    };
    assert!(columns.next().is_none(), "{label}");
    Case {
        label,
        raw,
        expected,
    }
}

fn fixture(label: &str) -> Case {
    parse_case(
        CORPUS
            .lines()
            .find(|line| line.split_once('\t').unwrap().0 == label)
            .unwrap_or_else(|| panic!("missing native vector {label}")),
    )
}

fn check_header(header: &L1HeaderRef<'_>, raw: &Bytes, expected: &Decoded) {
    assert_eq!(header.raw(), raw.as_ref());
    assert_eq!(header.raw().as_ptr(), raw.as_ptr());
    let actual = Decoded {
        hash: header.hash(),
        parent_hash: header.parent_hash(),
        state_root: header.state_root(),
        number: header.number(),
        timestamp: header.timestamp(),
    };
    assert_eq!(&actual, expected);
}

#[test]
fn decodes_the_exact_native_corpus() {
    assert_eq!(MAX_L1_HEADER_BYTES, 4096);
    let mut accepted = 0;
    let mut rejected = 0;
    for Case {
        label,
        raw,
        expected,
    } in CORPUS.lines().map(parse_case)
    {
        let decoded = L1HeaderRef::decode(&raw);
        if let Some(fields) = expected {
            let decoded = decoded.unwrap_or_else(|error| panic!("{label}: {error:?}"));
            check_header(&decoded, &raw, &fields);
            accepted += 1;
        } else {
            assert!(
                decoded.is_err(),
                "Raiko accepted rejected native row {label}"
            );
            rejected += 1;
        }
    }
    assert_eq!((accepted, rejected), (41, 118));
}

const fn context(reference: &Case) -> L1AncestryContext {
    L1AncestryContext {
        reference_hash: reference.fields().hash,
        reference_number: reference.fields().number,
        max_headers: 3,
        max_total_bytes: 3 * MAX_L1_HEADER_BYTES,
    }
}

#[test]
fn returns_only_the_supplied_authenticated_prefix_and_borrowed_fields() {
    let expected = [
        fixture("chain_tip"),
        fixture("chain_middle"),
        fixture("chain_oldest"),
    ];
    let raw = expected.each_ref().map(|case| case.raw.clone());
    for count in 1..=raw.len() {
        let context = L1AncestryContext {
            max_headers: count,
            max_total_bytes: raw[..count].iter().map(|header| header.len()).sum(),
            ..context(&expected[0])
        };
        let chain = verify_l1_ancestry(&raw[..count], &context).unwrap();
        assert_eq!(chain.headers().len(), count);
        for index in 0..count {
            let header = &chain.headers()[index];
            let fields = expected[index].fields();
            check_header(header, &raw[index], fields);
            assert!(std::ptr::eq(chain.header(fields.number).unwrap(), header));
        }
        assert!(
            chain
                .header(u64::MAX - u64::try_from(count).unwrap())
                .is_none()
        );
        assert!(chain.header(0).is_none());
    }

    let oldest = fixture("chain_zero");
    let reference = fixture("chain_one");
    let raw = [reference.raw.clone(), oldest.raw.clone()];
    let chain = verify_l1_ancestry(&raw, &context(&reference)).unwrap();
    check_header(chain.header(0).unwrap(), &raw[1], oldest.fields());
    assert!(chain.header(2).is_none());
    assert!(chain.header(u64::MAX).is_none());
}

#[test]
fn rejects_modified_context_or_disconnected_ancestry() {
    let tip = fixture("chain_tip");
    let parent = fixture("chain_middle");
    let oldest = fixture("chain_oldest");
    let alternate = fixture("chain_foreign_middle");
    let exact = [tip.raw.clone(), parent.raw.clone(), oldest.raw.clone()];
    let trusted = context(&tip);
    assert!(verify_l1_ancestry(&exact, &trusted).is_ok());
    for mismatched in [
        L1AncestryContext {
            reference_hash: parent.fields().hash,
            ..context(&tip)
        },
        L1AncestryContext {
            reference_number: parent.fields().number,
            ..context(&tip)
        },
    ] {
        assert!(verify_l1_ancestry(&exact, &mismatched).is_err());
    }
    for bad in [
        vec![parent.raw.clone(), tip.raw.clone(), oldest.raw.clone()],
        vec![tip.raw.clone(), oldest.raw.clone()],
        vec![tip.raw.clone(), parent.raw.clone(), parent.raw.clone()],
        vec![tip.raw.clone(), alternate.raw.clone(), oldest.raw.clone()],
        vec![
            tip.raw.clone(),
            parent.raw.clone(),
            fixture("outer_string").raw,
        ],
    ] {
        assert!(verify_l1_ancestry(&bad, &trusted).is_err());
    }

    let mainnet = fixture("mainnet_26155639");
    let appended = fixture("mainnet_shaped_future_synthetic");
    let real = mainnet.fields();
    let future = appended.fields();
    assert_eq!(
        (
            real.parent_hash,
            real.state_root,
            real.number,
            real.timestamp
        ),
        (
            future.parent_hash,
            future.state_root,
            future.number,
            future.timestamp
        )
    );
    assert_ne!(real.hash, future.hash);
    assert!(verify_l1_ancestry(std::slice::from_ref(&appended.raw), &context(&mainnet)).is_err());

    let gap = fixture("chain_hash_linked_height_gap");
    assert_eq!(gap.fields().parent_hash, oldest.fields().hash);
    let bad = [gap.raw.clone(), oldest.raw];
    assert!(verify_l1_ancestry(&bad, &context(&gap)).is_err());

    let zero = fixture("chain_wrap_zero");
    let wrap = fixture("chain_wrap_parent");
    assert_eq!(zero.fields().parent_hash, wrap.fields().hash);
    assert_eq!(wrap.fields().number, u64::MAX);
    let context = context(&zero);
    assert!(verify_l1_ancestry(std::slice::from_ref(&zero.raw), &context).is_ok());
    assert!(verify_l1_ancestry(&[zero.raw, wrap.raw], &context).is_err());
}

fn ancestry_error(headers: &[Bytes], context: &L1AncestryContext) -> L1AncestryError {
    verify_l1_ancestry(headers, context).expect_err("unbound ancestry")
}

#[test]
fn preflights_later_input_before_decoding_the_first_header() {
    let reference = fixture("chain_tip");
    let ctx = context(&reference);
    let valid = reference.raw.clone();
    let malformed = fixture("outer_string").raw;
    let capped = fixture("at_cap").raw;
    let over = fixture("over_cap").raw;
    assert!(matches!(
        ancestry_error(std::slice::from_ref(&malformed), &ctx),
        L1AncestryError::Header {
            index: 0,
            error: L1HeaderError::Rlp(_)
        }
    ));
    let count_ctx = L1AncestryContext {
        max_headers: 1,
        ..ctx
    };
    let total_ctx = L1AncestryContext {
        max_total_bytes: MAX_L1_HEADER_BYTES,
        ..ctx
    };
    assert_eq!(
        ancestry_error(&[malformed.clone(), capped.clone()], &count_ctx),
        L1AncestryError::CountLimit { actual: 2, max: 1 }
    );
    assert_eq!(
        ancestry_error(&[malformed.clone(), capped], &total_ctx),
        L1AncestryError::TotalBytesLimit {
            max: MAX_L1_HEADER_BYTES
        }
    );
    let cap_error = L1AncestryError::Header {
        index: 1,
        error: L1HeaderError::Oversize(MAX_L1_HEADER_BYTES + 1),
    };
    assert_eq!(ancestry_error(&[valid, over.clone()], &ctx), cap_error);
    assert_eq!(ancestry_error(&[malformed, over], &ctx), cap_error);
    assert_eq!(ancestry_error(&[], &ctx), L1AncestryError::Empty);
}
