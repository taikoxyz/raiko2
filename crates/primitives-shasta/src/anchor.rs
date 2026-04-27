//! Shared Shasta anchor validation helpers.

const ANCHOR_MAX_OFFSET: u64 = 128;
const MAINNET_ANCHOR_MAX_OFFSET: u64 = 512;
const TAIKO_MAINNET_CHAIN_ID: u64 = 167_000;
const TAIKO_TRANSITION_CHAIN_ID: u64 = 167_014;

/// Returns the maximum permitted gap between the proposal origin block and any anchor block.
#[must_use]
pub const fn anchor_max_offset_for_chain(chain_id: u64) -> u64 {
    if chain_id == TAIKO_MAINNET_CHAIN_ID || chain_id == TAIKO_TRANSITION_CHAIN_ID {
        MAINNET_ANCHOR_MAX_OFFSET
    } else {
        ANCHOR_MAX_OFFSET
    }
}

/// Return true when old raiko's stalled-anchor linkage bypass applies.
#[must_use]
pub fn should_bypass_stalled_anchor_linkage(
    anchor_block_numbers: &[u64],
    last_anchor_block_number: u64,
    origin_block_number: u64,
    chain_id: u64,
) -> bool {
    let Some(&first_anchor_block_number) = anchor_block_numbers.first() else {
        return false;
    };
    first_anchor_block_number == last_anchor_block_number
        && origin_block_number.saturating_sub(first_anchor_block_number)
            > anchor_max_offset_for_chain(chain_id)
        && anchor_block_numbers
            .iter()
            .all(|&anchor_block_number| anchor_block_number == first_anchor_block_number)
}

/// Validate Shasta anchor linkage for a materialized proposal batch.
///
/// # Errors
///
/// Returns a descriptive error string when anchors regress below the parent anchor or fall outside
/// the permitted origin window.
pub fn validate_anchor_progression(
    anchor_block_numbers: &[u64],
    last_anchor_block_number: u64,
    origin_block_number: u64,
    chain_id: u64,
) -> Result<(), String> {
    if anchor_block_numbers.is_empty() {
        return Err("anchor_block_numbers must not be empty".to_string());
    }

    let min_anchor_block_number =
        origin_block_number.saturating_sub(anchor_max_offset_for_chain(chain_id));
    let mut previous_anchor_block_number = None;

    for &anchor_block_number in anchor_block_numbers {
        if anchor_block_number < last_anchor_block_number {
            return Err(format!(
                "anchor {anchor_block_number} is below last_anchor_block_number {last_anchor_block_number}"
            ));
        }
        if anchor_block_number < min_anchor_block_number
            || anchor_block_number > origin_block_number
        {
            return Err(format!(
                "anchor {anchor_block_number} is outside valid range [{min_anchor_block_number}, {origin_block_number}]"
            ));
        }
        if let Some(previous_anchor_block_number) = previous_anchor_block_number
            && anchor_block_number < previous_anchor_block_number
        {
            return Err(format!(
                "anchor {anchor_block_number} regressed below previous anchor {previous_anchor_block_number}"
            ));
        }

        previous_anchor_block_number = Some(anchor_block_number);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        anchor_max_offset_for_chain, should_bypass_stalled_anchor_linkage,
        validate_anchor_progression,
    };

    #[test]
    fn uses_mainnet_window_for_transition_chain() {
        assert_eq!(anchor_max_offset_for_chain(167_014), 512);
    }

    #[test]
    fn rejects_anchor_regression_below_last_anchor() {
        let err = validate_anchor_progression(&[41, 42], 42, 50, 167_000).expect_err("reject");
        assert!(err.contains("below last_anchor_block_number"));
    }

    #[test]
    fn accepts_anchor_sequences_that_do_not_grow() {
        validate_anchor_progression(&[42, 42], 42, 50, 167_000).expect("valid anchors");
    }

    #[test]
    fn rejects_anchor_outside_valid_window() {
        let err = validate_anchor_progression(&[1], 0, 200, 167_001).expect_err("reject");
        assert!(err.contains("outside valid range"));
    }

    #[test]
    fn accepts_non_decreasing_sequence_with_growth() {
        validate_anchor_progression(&[42, 42, 43], 41, 100, 167_000).expect("valid anchors");
    }

    #[test]
    fn bypasses_stalled_anchor_linkage_when_all_anchors_match_last_anchor() {
        assert!(should_bypass_stalled_anchor_linkage(
            &[42, 42],
            42,
            200,
            167_001
        ));
    }

    #[test]
    fn does_not_bypass_stalled_anchor_linkage_when_anchors_change() {
        assert!(!should_bypass_stalled_anchor_linkage(
            &[42, 43],
            42,
            200,
            167_001
        ));
    }

    #[test]
    fn does_not_bypass_stalled_anchor_linkage_inside_anchor_window() {
        assert!(!should_bypass_stalled_anchor_linkage(
            &[42, 42],
            42,
            100,
            167_001
        ));
    }
}
