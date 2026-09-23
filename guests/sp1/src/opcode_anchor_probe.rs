use raiko2_primitives::OpcodeLabInput;
use revm::{interpreter::Stack, primitives::U256};

const TARGET_SCENARIO: &str = "anchor_target_";
const CONTROL_SCENARIO: &str = "anchor_control";
const MAX_COUNT: u64 = 131_072;
const PUBLIC_INPUT_LEN: usize = 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorProbeLane {
    Target,
    Control,
}

fn expected_case(opcode: u8) -> Option<&'static str> {
    match opcode {
        0x50 => Some("synthetic_anchor_probe_pop"),
        0x5f => Some("synthetic_anchor_probe_push0"),
        0x80 => Some("synthetic_anchor_probe_dup1"),
        0x90 => Some("synthetic_anchor_probe_swap1"),
        _ => None,
    }
}

pub fn anchor_probe_lane(input: &OpcodeLabInput) -> Option<AnchorProbeLane> {
    if expected_case(input.opcode)? != input.case {
        return None;
    }
    match input.scenario.as_str() {
        TARGET_SCENARIO => Some(AnchorProbeLane::Target),
        CONTROL_SCENARIO => Some(AnchorProbeLane::Control),
        _ => None,
    }
}

fn validate_anchor_probe(input: &OpcodeLabInput) -> Result<AnchorProbeLane, &'static str> {
    let lane = anchor_probe_lane(input).ok_or("anchor probe declaration is missing")?;
    let expected_raw_gas = match input.opcode {
        0x50 | 0x5f => 2,
        0x80 | 0x90 => 3,
        _ => return Err("anchor probe opcode is unsupported"),
    };
    if input.target_raw_gas != expected_raw_gas {
        return Err("anchor probe raw gas differs from the frozen anchor");
    }
    if input.target_count > MAX_COUNT {
        return Err("anchor probe count exceeds the frozen checkpoint");
    }
    if input.tx_gas_limit != Some(100_000)
        || input.generator_max_count != Some(MAX_COUNT)
        || input.fixed_bytecode_len != Some(1)
        || input.bytecode != [0x00]
    {
        return Err("anchor probe envelope differs from the frozen declaration");
    }
    Ok(lane)
}

fn mix_anchor_probe(accumulator: u64, value: u64) -> u64 {
    accumulator
        .rotate_left(13)
        .wrapping_mul(0x9e37_79b1_85eb_ca87)
        .wrapping_add(value)
}

type AnchorStackStep = fn(&mut Stack) -> Result<(), &'static str>;

#[inline(never)]
fn anchor_control_step(_stack: &mut Stack) -> Result<(), &'static str> {
    Ok(())
}

#[inline(never)]
fn anchor_pop_step(stack: &mut Stack) -> Result<(), &'static str> {
    stack
        .pop()
        .map(|_| ())
        .map_err(|_| "anchor probe stack operation failed")
}

#[inline(never)]
fn anchor_push0_step(stack: &mut Stack) -> Result<(), &'static str> {
    stack
        .push(U256::ZERO)
        .then_some(())
        .ok_or("anchor probe stack operation failed")
}

#[inline(never)]
fn anchor_dup1_step(stack: &mut Stack) -> Result<(), &'static str> {
    stack
        .dup(1)
        .then_some(())
        .ok_or("anchor probe stack operation failed")
}

#[inline(never)]
fn anchor_swap1_step(stack: &mut Stack) -> Result<(), &'static str> {
    stack
        .swap(1)
        .then_some(())
        .ok_or("anchor probe stack operation failed")
}

fn select_anchor_stack_step(
    input: &OpcodeLabInput,
) -> Result<AnchorStackStep, &'static str> {
    if validate_anchor_probe(input)? == AnchorProbeLane::Control {
        return Ok(anchor_control_step);
    }
    match input.opcode {
        0x50 => Ok(anchor_pop_step),
        0x5f => Ok(anchor_push0_step),
        0x80 => Ok(anchor_dup1_step),
        0x90 => Ok(anchor_swap1_step),
        _ => Err("anchor probe opcode is unsupported"),
    }
}

pub fn execute_anchor_probe(input: &OpcodeLabInput) -> Result<u64, &'static str> {
    let stack_step = select_anchor_stack_step(input)?;
    let mut accumulator = 0x243f_6a88_85a3_08d3 ^ u64::from(input.opcode);
    let mut stack = Stack::new();
    for iteration in 0..input.target_count {
        stack.data_mut().clear();
        let first = iteration ^ accumulator.rotate_left(7);
        let second = iteration
            .wrapping_mul(0x1000_0000_01b3)
            .wrapping_add(accumulator.rotate_right(11));
        if !stack.push(U256::from(first)) || !stack.push(U256::from(second)) {
            return Err("anchor probe stack seed failed");
        }
        accumulator = mix_anchor_probe(accumulator, iteration);
        stack_step(&mut stack)?;
        let _ = core::hint::black_box(&stack);
        accumulator = mix_anchor_probe(accumulator, iteration.rotate_left(17));
    }
    Ok(accumulator)
}

pub fn prepare_anchor_probe_public_input(
    input: &OpcodeLabInput,
    accumulator: u64,
) -> Result<[u8; PUBLIC_INPUT_LEN], &'static str> {
    let lane = match validate_anchor_probe(input)? {
        AnchorProbeLane::Target => 0,
        AnchorProbeLane::Control => 1,
    };
    let mut output = [0u8; PUBLIC_INPUT_LEN];
    output[0] = 1;
    output[1] = input.opcode;
    output[2] = lane;
    output[4..12].copy_from_slice(&input.target_count.to_le_bytes());
    output[12..20].copy_from_slice(&input.target_raw_gas.to_le_bytes());
    output[20..28].copy_from_slice(&accumulator.to_le_bytes());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{
        AnchorProbeLane, CONTROL_SCENARIO, TARGET_SCENARIO, anchor_probe_lane,
        execute_anchor_probe, prepare_anchor_probe_public_input,
    };
    use raiko2_primitives::OpcodeLabInput;

    fn input(opcode: u8, lane: AnchorProbeLane, count: u64) -> OpcodeLabInput {
        OpcodeLabInput {
            case: match opcode {
                0x50 => "synthetic_anchor_probe_pop",
                0x5f => "synthetic_anchor_probe_push0",
                0x80 => "synthetic_anchor_probe_dup1",
                0x90 => "synthetic_anchor_probe_swap1",
                _ => "synthetic_anchor_probe_unsupported",
            }
            .into(),
            scenario: match lane {
                AnchorProbeLane::Target => TARGET_SCENARIO,
                AnchorProbeLane::Control => CONTROL_SCENARIO,
            }
            .into(),
            opcode,
            target_count: count,
            target_raw_gas: if matches!(opcode, 0x50 | 0x5f) { 2 } else { 3 },
            tx_gas_limit: Some(100_000),
            bytecode: vec![0x00],
            generator_max_count: Some(131_072),
            fixed_bytecode_len: Some(1),
        }
    }

    #[test]
    fn probe_lane_requires_the_exact_case_opcode_and_scenario_identity() {
        let target = input(0x50, AnchorProbeLane::Target, 8);
        assert_eq!(anchor_probe_lane(&target), Some(AnchorProbeLane::Target));

        let mut ordinary = target.clone();
        ordinary.case = "ordinary_pop".into();
        assert_eq!(anchor_probe_lane(&ordinary), None);

        let mut mismatched = target;
        mismatched.opcode = 0x80;
        assert_eq!(anchor_probe_lane(&mismatched), None);
    }

    #[test]
    fn target_and_control_inputs_have_equal_wire_length() {
        let target = input(0x50, AnchorProbeLane::Target, 131_072);
        let control = input(0x50, AnchorProbeLane::Control, 131_072);

        assert_eq!(
            bincode::serialize(&target).unwrap().len(),
            bincode::serialize(&control).unwrap().len()
        );
    }

    #[test]
    fn probe_executes_all_four_stack_primitives_against_a_matched_control() {
        for opcode in [0x50, 0x5f, 0x80, 0x90] {
            let target = input(opcode, AnchorProbeLane::Target, 8);
            let control = input(opcode, AnchorProbeLane::Control, 8);
            let target_result = execute_anchor_probe(&target).unwrap();
            let control_result = execute_anchor_probe(&control).unwrap();

            assert_eq!(target_result, control_result, "opcode 0x{opcode:02x}");
            assert_ne!(
                prepare_anchor_probe_public_input(&target, target_result).unwrap(),
                prepare_anchor_probe_public_input(&control, control_result).unwrap(),
                "opcode 0x{opcode:02x}",
            );
        }
    }

    #[test]
    fn probe_rejects_an_unfrozen_envelope() {
        let mut probe = input(0x50, AnchorProbeLane::Target, 8);
        probe.tx_gas_limit = None;
        assert_eq!(
            execute_anchor_probe(&probe),
            Err("anchor probe envelope differs from the frozen declaration")
        );
    }
}
