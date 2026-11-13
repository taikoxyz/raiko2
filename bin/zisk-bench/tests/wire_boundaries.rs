//! Independent boundary checks for the experimental CPU lane.
use bincode::Options;
use raiko2_primitives_shasta::{
    GuestInput,
    zisk::{frame_stdin, unframe_stdin},
};

#[test]
fn complete_proposal_roundtrips_through_exact_guest_codec() {
    let input: GuestInput = serde_json::from_str(include_str!(
        "../../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json"
    )).unwrap();
    assert_eq!(input.witnesses.len(), 192);
    let payload = bincode::serialize(&input).unwrap();
    let frame = frame_stdin(&payload);
    let decoded: GuestInput = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(unframe_stdin(&frame).unwrap())
        .unwrap();
    assert_eq!(bincode::serialize(&decoded).unwrap(), payload);

    let mut suffixed = payload;
    suffixed.push(0);
    assert!(
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .reject_trailing_bytes()
            .deserialize::<GuestInput>(unframe_stdin(&frame_stdin(&suffixed)).unwrap())
            .is_err()
    );
}

#[test]
fn empty_aligned_and_unaligned_frames_have_exact_boundaries() {
    for length in [0, 1, 7, 8, 9, 63, 64, 65] {
        let payload = vec![37; length];
        let frame = frame_stdin(&payload);
        assert_eq!(frame.len() % 8, 0);
        assert_eq!(unframe_stdin(&frame).unwrap(), payload);
        assert!(unframe_stdin(&frame[..frame.len() - 1]).is_err());
        let mut extra = frame;
        extra.extend_from_slice(&[0; 8]);
        assert!(unframe_stdin(&extra).is_err());
    }
}

#[test]
fn cost_parser_rejects_overflow_zero_steps_and_duplicate_steps() {
    let stats = "STEPS,10\nCOST,MAIN,20,\nCOST,OPCODES,30,\nCOST,PRECOMPILES,40,\nCOST,MEMORY,50,\nCOST,BASE,60,\nCOST,TOTAL,200,\n";
    assert!(zisk_bench::parse_stats(&stats.replace("STEPS,10", "STEPS,0"), 100).is_err());
    assert!(zisk_bench::parse_stats(&format!("{stats}STEPS,10\n"), 100).is_err());
    let overflow = stats.replace("COST,MAIN,20", &format!("COST,MAIN,{}", u64::MAX));
    assert!(zisk_bench::parse_stats(&overflow, 100).is_err());
    assert!(
        zisk_bench::parse_stats(
            &stats.replace("STEPS,10", "STEPS,18446744073709551616"),
            u64::MAX
        )
        .is_err()
    );
}
