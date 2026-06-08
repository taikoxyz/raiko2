use alloy_primitives::keccak256;

pub fn execute_bytecode(bytecode: &[u8]) -> u64 {
    let mut pc = 0usize;
    let mut stack = Vec::<u64>::new();
    let mut memory = Vec::<u8>::new();

    while pc < bytecode.len() {
        let opcode = bytecode[pc];
        pc += 1;
        match opcode {
            0x00 => break,
            0x01 => binary(&mut stack, |left, right| left.wrapping_add(right)),
            0x02 => binary(&mut stack, |left, right| left.wrapping_mul(right)),
            0x03 => binary(&mut stack, |left, right| left.wrapping_sub(right)),
            0x04 => binary(&mut stack, |left, right| if right == 0 { 0 } else { left / right }),
            0x05 => binary(&mut stack, signed_div),
            0x06 => binary(&mut stack, |left, right| if right == 0 { 0 } else { left % right }),
            0x07 => binary(&mut stack, signed_mod),
            0x08 => ternary(&mut stack, |left, right, modulus| {
                if modulus == 0 {
                    0
                } else {
                    left.wrapping_add(right) % modulus
                }
            }),
            0x09 => ternary(&mut stack, |left, right, modulus| {
                if modulus == 0 {
                    0
                } else {
                    left.wrapping_mul(right) % modulus
                }
            }),
            0x0A => binary(&mut stack, |left, right| {
                let exponent = u32::try_from(right).unwrap_or(u32::MAX);
                left.wrapping_pow(exponent)
            }),
            0x0B => binary(&mut stack, signextend),
            0x10 => binary(&mut stack, |left, right| u64::from(left < right)),
            0x11 => binary(&mut stack, |left, right| u64::from(left > right)),
            0x12 => binary(&mut stack, |left, right| u64::from(as_i64(left) < as_i64(right))),
            0x13 => binary(&mut stack, |left, right| u64::from(as_i64(left) > as_i64(right))),
            0x14 => binary(&mut stack, |left, right| u64::from(left == right)),
            0x15 => {
                let value = pop(&mut stack);
                stack.push(u64::from(value == 0));
            }
            0x16 => binary(&mut stack, |left, right| left & right),
            0x17 => binary(&mut stack, |left, right| left | right),
            0x18 => binary(&mut stack, |left, right| left ^ right),
            0x19 => {
                let value = pop(&mut stack);
                stack.push(!value);
            }
            0x1A => binary(&mut stack, byte_at),
            0x1B => binary(&mut stack, shift_left),
            0x1C => binary(&mut stack, shift_right),
            0x1D => binary(&mut stack, shift_arithmetic_right),
            0x20 => {
                let offset = to_usize(pop(&mut stack));
                let size = to_usize(pop(&mut stack));
                ensure_memory(&mut memory, offset.saturating_add(size));
                let digest = keccak256(&memory[offset..offset + size]);
                let folded = u64::from_be_bytes(digest[..8].try_into().expect("hash prefix"));
                stack.push(folded);
            }
            0x50 => {
                pop(&mut stack);
            }
            0x51 => {
                let offset = to_usize(pop(&mut stack));
                ensure_memory(&mut memory, offset.saturating_add(32));
                stack.push(read_word(&memory[offset..offset + 32]));
            }
            0x52 => {
                let offset = to_usize(pop(&mut stack));
                let value = pop(&mut stack);
                ensure_memory(&mut memory, offset.saturating_add(32));
                memory[offset + 24..offset + 32].copy_from_slice(&value.to_be_bytes());
            }
            0x53 => {
                let offset = to_usize(pop(&mut stack));
                let value = pop(&mut stack);
                ensure_memory(&mut memory, offset.saturating_add(1));
                memory[offset] = value as u8;
            }
            0x56 => {
                pc = checked_jump_destination(pop(&mut stack), bytecode);
            }
            0x57 => {
                let destination = pop(&mut stack);
                let condition = pop(&mut stack);
                if condition != 0 {
                    pc = checked_jump_destination(destination, bytecode);
                }
            }
            0x58 => stack.push((pc - 1) as u64),
            0x59 => stack.push(memory.len() as u64),
            0x5A => stack.push(u64::MAX.saturating_sub(pc as u64)),
            0x5B => {}
            0x5E => {
                let destination = to_usize(pop(&mut stack));
                let source = to_usize(pop(&mut stack));
                let len = to_usize(pop(&mut stack));
                let end = source
                    .saturating_add(len)
                    .max(destination.saturating_add(len));
                ensure_memory(&mut memory, end);
                let copied = memory[source..source + len].to_vec();
                memory[destination..destination + len].copy_from_slice(&copied);
            }
            0x5f => stack.push(0),
            0x60..=0x7f => {
                let len = usize::from(opcode - 0x5f);
                if pc + len > bytecode.len() {
                    panic!("truncated PUSH{len}");
                }
                let value = bytecode[pc..pc + len]
                    .iter()
                    .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
                pc += len;
                stack.push(value);
            }
            0x80..=0x8F => {
                let depth = usize::from(opcode - 0x7F);
                let value = *stack
                    .get(stack.len().checked_sub(depth).expect("DUP depth underflow"))
                    .unwrap_or_else(|| panic!("DUP{depth} requires {depth} stack item(s)"));
                stack.push(value);
            }
            0x90..=0x9F => {
                let depth = usize::from(opcode - 0x8F);
                let len = stack.len();
                if len <= depth {
                    panic!("SWAP{depth} requires {} stack items", depth + 1);
                }
                stack.swap(len - 1, len - 1 - depth);
            }
            _ => panic!("unsupported opcode 0x{opcode:02x}"),
        }
    }

    stack.iter().fold(memory.len() as u64, |acc, value| {
        acc.wrapping_mul(31).wrapping_add(*value)
    })
}

fn binary(stack: &mut Vec<u64>, op: impl FnOnce(u64, u64) -> u64) {
    let right = pop(stack);
    let left = pop(stack);
    stack.push(op(left, right));
}

fn ternary(stack: &mut Vec<u64>, op: impl FnOnce(u64, u64, u64) -> u64) {
    let third = pop(stack);
    let second = pop(stack);
    let first = pop(stack);
    stack.push(op(first, second, third));
}

fn pop(stack: &mut Vec<u64>) -> u64 {
    stack.pop().expect("stack underflow")
}

fn ensure_memory(memory: &mut Vec<u8>, len: usize) {
    if memory.len() < len {
        memory.resize(len, 0);
    }
}

fn read_word(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes[24..32].try_into().expect("word suffix"))
}

fn to_usize(value: u64) -> usize {
    usize::try_from(value).expect("memory index too large")
}

fn checked_jump_destination(destination: u64, bytecode: &[u8]) -> usize {
    let destination = to_usize(destination);
    if bytecode.get(destination) != Some(&0x5B) {
        panic!("invalid JUMP destination {destination}");
    }
    destination
}

fn as_i64(value: u64) -> i64 {
    value as i64
}

fn signed_div(left: u64, right: u64) -> u64 {
    let left = as_i64(left);
    let right = as_i64(right);
    if right == 0 {
        0
    } else if left == i64::MIN && right == -1 {
        i64::MIN as u64
    } else {
        (left / right) as u64
    }
}

fn signed_mod(left: u64, right: u64) -> u64 {
    let left = as_i64(left);
    let right = as_i64(right);
    if right == 0 {
        0
    } else {
        (left % right) as u64
    }
}

fn signextend(value: u64, byte_index: u64) -> u64 {
    if byte_index >= 8 {
        return value;
    }
    let bit_index = byte_index * 8 + 7;
    let mask = (1u64 << bit_index) - 1;
    let sign_bit = 1u64 << bit_index;
    if value & sign_bit == 0 {
        value & (mask | sign_bit)
    } else {
        value | !mask
    }
}

fn byte_at(index: u64, value: u64) -> u64 {
    if index >= 8 {
        0
    } else {
        (value >> ((7 - index) * 8)) & 0xff
    }
}

fn shift_left(value: u64, shift: u64) -> u64 {
    if shift >= 64 {
        0
    } else {
        value.wrapping_shl(shift as u32)
    }
}

fn shift_right(value: u64, shift: u64) -> u64 {
    if shift >= 64 {
        0
    } else {
        value >> shift
    }
}

fn shift_arithmetic_right(value: u64, shift: u64) -> u64 {
    if shift >= 64 {
        if as_i64(value) < 0 {
            u64::MAX
        } else {
            0
        }
    } else {
        (as_i64(value) >> shift) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::execute_bytecode;

    #[test]
    fn executes_stack_binary_template() {
        let bytecode = [0x60, 0x01, 0x60, 0x02, 0x01, 0x60, 0x01, 0x01, 0x50, 0x00];
        assert_eq!(execute_bytecode(&bytecode), 0);
    }

    #[test]
    fn executes_extended_pure_opcode_templates() {
        let bytecode = [
            0x60, 0x08, 0x60, 0x02, 0x05, 0x50, // SDIV
            0x60, 0x08, 0x60, 0x03, 0x07, 0x50, // SMOD
            0x60, 0x01, 0x60, 0x02, 0x60, 0x05, 0x08, 0x50, // ADDMOD
            0x60, 0x02, 0x60, 0x03, 0x60, 0x05, 0x09, 0x50, // MULMOD
            0x60, 0x02, 0x60, 0x03, 0x0a, 0x50, // EXP
            0x60, 0xff, 0x60, 0x00, 0x0b, 0x50, // SIGNEXTEND
            0x60, 0x01, 0x60, 0x02, 0x12, 0x50, // SLT
            0x60, 0x02, 0x60, 0x01, 0x13, 0x50, // SGT
            0x60, 0x01, 0x19, 0x50, // NOT
            0x60, 0x00, 0x60, 0xff, 0x1a, 0x50, // BYTE
            0x60, 0x01, 0x60, 0x02, 0x1b, 0x50, // SHL
            0x60, 0x08, 0x60, 0x01, 0x1c, 0x50, // SHR
            0x60, 0x08, 0x60, 0x01, 0x1d, 0x50, // SAR
            0x00,
        ];

        assert_eq!(execute_bytecode(&bytecode), 0);
    }

    #[test]
    fn executes_keccak_32_template() {
        let bytecode = [
            0x60, 0x00, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0x20, 0x50, 0x00,
        ];
        assert_eq!(execute_bytecode(&bytecode), 32);
    }

    #[test]
    fn executes_remaining_pure_opcode_templates() {
        let bytecode = [
            0x5f, 0x50, // PUSH0; POP
            0x7f, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x50, // PUSH32; POP
            0x60, 0x01, 0x60, 0x02, 0x60, 0x03, 0x60, 0x04, 0x60, 0x05, 0x60, 0x06, 0x60,
            0x07, 0x60, 0x08, 0x60, 0x09, 0x60, 0x0a, 0x60, 0x0b, 0x60, 0x0c, 0x60, 0x0d,
            0x60, 0x0e, 0x60, 0x0f, 0x60, 0x10, 0x8f, 0x50, // DUP16; POP
            0x60, 0x11,
            0x9f, // SWAP16
            0x58, 0x50, // PC; POP
            0x59, 0x50, // MSIZE; POP
            0x5a, 0x50, // GAS; POP
            0x60, 0x01, 0x60, 0x00, 0x52, // MSTORE
            0x60, 0x20, 0x60, 0x00, 0x60, 0x20, 0x5e, // MCOPY
            0x60, 0x5f, 0x56, // JUMP to the next JUMPDEST
            0x00, 0x5b, // skipped STOP; JUMPDEST
            0x60, 0x01, 0x60, 0x66, 0x57, // JUMPI to the final JUMPDEST
            0x00, 0x5b, 0x00,
        ];

        assert_ne!(execute_bytecode(&bytecode), 0);
    }
}
