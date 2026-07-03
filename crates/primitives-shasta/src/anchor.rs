//! Shared Shasta anchor validation helpers.

const ANCHOR_MAX_OFFSET: u64 = 128;
const MAINNET_ANCHOR_MAX_OFFSET: u64 = 512;
const TAIKO_MAINNET_CHAIN_ID: u64 = 167_000;

/// Returns the maximum permitted gap between the proposal origin block and any anchor block.
///
/// Only mainnet uses the larger window; every other chain uses the default. The guest fails closed
/// before proving any chain absent from the compiled-in spec list, so a special-cased-but-unlisted
/// chain ID can no longer slip through (F-1).
#[must_use]
pub const fn anchor_max_offset_for_chain(chain_id: u64) -> u64 {
    if chain_id == TAIKO_MAINNET_CHAIN_ID {
        MAINNET_ANCHOR_MAX_OFFSET
    } else {
        ANCHOR_MAX_OFFSET
    }
}

/// A contiguous run of L2 blocks derived from one Shasta derivation source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorSourceSpan {
    pub is_forced_inclusion: bool,
    pub block_count: usize,
}

/// Result of source-aware anchor validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceAwareAnchorPlan {
    /// First anchor checkpoint index that must be proven through `l1_ancestor_headers`.
    ///
    /// Checkpoints before this index belong to forced-inclusion sources and must match the parent
    /// Anchor checkpoint instead.
    pub l1_header_anchor_start_index: usize,
    pub requires_parent_checkpoint: bool,
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

/// Validate Shasta anchor linkage while respecting derivation source boundaries.
///
/// Forced-inclusion sources inherit the parent anchor and therefore must not advance it. The final
/// normal source is then checked with the regular anchor progression/window rules.
///
/// # Errors
///
/// Returns a descriptive error when the source layout is not the canonical forced-prefix plus final
/// normal source, when source spans do not cover every block, or when an anchor violates the source
/// rule.
pub fn validate_source_aware_anchor_progression(
    anchor_block_numbers: &[u64],
    source_spans: &[AnchorSourceSpan],
    last_anchor_block_number: u64,
    origin_block_number: u64,
    chain_id: u64,
) -> Result<SourceAwareAnchorPlan, String> {
    if anchor_block_numbers.is_empty() {
        return Err("anchor_block_numbers must not be empty".to_string());
    }
    let Some((normal_source, forced_sources)) = source_spans.split_last() else {
        return Err("source_spans must not be empty".to_string());
    };
    if normal_source.is_forced_inclusion {
        return Err("last Shasta derivation source must be a normal source".to_string());
    }
    if normal_source.block_count == 0 {
        return Err("normal Shasta derivation source must contain at least one block".to_string());
    }

    let mut cursor = 0usize;
    for (source_index, span) in forced_sources.iter().enumerate() {
        if !span.is_forced_inclusion {
            return Err(format!(
                "Shasta derivation source {source_index} must be forced inclusion; only the final source may be normal"
            ));
        }
        if span.block_count == 0 {
            return Err(format!(
                "forced inclusion source {source_index} must contain at least one block"
            ));
        }
        let end = cursor.checked_add(span.block_count).ok_or_else(|| {
            format!("forced inclusion source {source_index} block count overflow")
        })?;
        let anchors = anchor_block_numbers.get(cursor..end).ok_or_else(|| {
            format!(
                "source spans cover more blocks than anchor_block_numbers at source {source_index}"
            )
        })?;
        for &anchor_block_number in anchors {
            if anchor_block_number != last_anchor_block_number {
                return Err(format!(
                    "forced inclusion source {source_index} anchor {anchor_block_number} must equal parent anchor {last_anchor_block_number}"
                ));
            }
        }
        cursor = end;
    }

    let end = cursor
        .checked_add(normal_source.block_count)
        .ok_or_else(|| "normal Shasta derivation source block count overflow".to_string())?;
    if end != anchor_block_numbers.len() {
        return Err(format!(
            "source spans cover {end} blocks but anchor_block_numbers has {}",
            anchor_block_numbers.len()
        ));
    }

    validate_anchor_progression(
        &anchor_block_numbers[cursor..end],
        last_anchor_block_number,
        origin_block_number,
        chain_id,
    )?;

    Ok(SourceAwareAnchorPlan {
        l1_header_anchor_start_index: cursor,
        requires_parent_checkpoint: cursor > 0,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AnchorSourceSpan, anchor_max_offset_for_chain, should_bypass_stalled_anchor_linkage,
        validate_anchor_progression, validate_source_aware_anchor_progression,
    };

    #[test]
    fn mainnet_uses_large_anchor_window() {
        assert_eq!(anchor_max_offset_for_chain(167_000), 512);
    }

    #[test]
    fn non_mainnet_chain_uses_default_window() {
        assert_eq!(anchor_max_offset_for_chain(167_001), 128);
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

    // --- progression_* : window + monotonicity boundaries ---

    #[test]
    fn progression_accepts_anchor_at_lower_window_edge() {
        // non-mainnet offset 128; origin 1000 => min_anchor 872
        validate_anchor_progression(&[872], 0, 1000, 167_001).expect("lower edge is inclusive");
    }

    #[test]
    fn progression_rejects_anchor_below_lower_window_edge() {
        let err = validate_anchor_progression(&[871], 0, 1000, 167_001).expect_err("below window");
        assert!(err.contains("outside valid range"));
    }

    #[test]
    fn progression_accepts_anchor_at_origin() {
        validate_anchor_progression(&[1000], 0, 1000, 167_001).expect("origin is inclusive");
    }

    #[test]
    fn progression_rejects_anchor_above_origin() {
        let err = validate_anchor_progression(&[1001], 0, 1000, 167_001).expect_err("above origin");
        assert!(err.contains("outside valid range"));
    }

    #[test]
    fn progression_rejects_regression_below_previous_in_batch() {
        // all in window [872,1000]; third regresses below the second
        let err =
            validate_anchor_progression(&[900, 905, 902], 900, 1000, 167_001).expect_err("regress");
        assert!(err.contains("regressed below previous anchor"));
    }

    #[test]
    fn progression_rejects_empty_anchor_numbers() {
        let err = validate_anchor_progression(&[], 0, 1000, 167_001).expect_err("empty");
        assert!(err.contains("must not be empty"));
    }

    #[test]
    fn progression_handles_origin_below_offset_via_saturating_min() {
        // origin 50 < offset 128 => min_anchor saturates to 0; anchor 0 is valid
        validate_anchor_progression(&[0], 0, 50, 167_001).expect("saturating min admits anchor 0");
    }

    // --- progression_* : stalled-bypass boundary (strict greater-than) ---

    #[test]
    fn bypass_is_exclusive_at_offset_boundary_non_mainnet() {
        // origin - anchor == 128 (== offset) must NOT bypass (code uses strict `>`)
        assert!(!should_bypass_stalled_anchor_linkage(
            &[42, 42],
            42,
            170,
            167_001
        ));
        // origin - anchor == 129 (> offset) bypasses
        assert!(should_bypass_stalled_anchor_linkage(
            &[42, 42],
            42,
            171,
            167_001
        ));
    }

    #[test]
    fn bypass_is_exclusive_at_offset_boundary_mainnet() {
        // mainnet offset 512
        assert!(!should_bypass_stalled_anchor_linkage(
            &[42, 42],
            42,
            554,
            167_000
        ));
        assert!(should_bypass_stalled_anchor_linkage(
            &[42, 42],
            42,
            555,
            167_000
        ));
    }

    #[test]
    fn bypass_requires_all_anchors_equal_last() {
        // first==last but a later anchor differs => not a stall => no bypass
        assert!(!should_bypass_stalled_anchor_linkage(
            &[42, 42, 43],
            42,
            1000,
            167_001
        ));
    }

    #[test]
    fn source_aware_progression_accepts_forced_prefix_then_normal_catchup() {
        let plan = validate_source_aware_anchor_progression(
            &[13_414, 13_414, 13_414, 13_414, 14_188],
            &[
                AnchorSourceSpan {
                    is_forced_inclusion: true,
                    block_count: 1,
                },
                AnchorSourceSpan {
                    is_forced_inclusion: true,
                    block_count: 1,
                },
                AnchorSourceSpan {
                    is_forced_inclusion: true,
                    block_count: 1,
                },
                AnchorSourceSpan {
                    is_forced_inclusion: true,
                    block_count: 1,
                },
                AnchorSourceSpan {
                    is_forced_inclusion: false,
                    block_count: 1,
                },
            ],
            13_414,
            14_190,
            167_001,
        )
        .expect("forced prefix should be accepted");

        assert_eq!(plan.l1_header_anchor_start_index, 4);
        assert!(plan.requires_parent_checkpoint);
    }

    #[test]
    fn source_aware_progression_rejects_forced_segment_that_bumps_anchor() {
        let err = validate_source_aware_anchor_progression(
            &[13_414, 13_500, 14_188],
            &[
                AnchorSourceSpan {
                    is_forced_inclusion: true,
                    block_count: 2,
                },
                AnchorSourceSpan {
                    is_forced_inclusion: false,
                    block_count: 1,
                },
            ],
            13_414,
            14_190,
            167_001,
        )
        .expect_err("forced segment must not bump anchor");

        assert!(err.contains("forced inclusion source"));
    }
}
