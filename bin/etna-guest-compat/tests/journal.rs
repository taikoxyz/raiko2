#[test]
fn native_suite_matches_pinned_stock_journal() {
    assert_eq!(
        etna_guest_compat_fixture::run().as_slice(),
        include_bytes!("../../../crates/etna-guest-compat/fixtures/journal.bin"),
    );
}
