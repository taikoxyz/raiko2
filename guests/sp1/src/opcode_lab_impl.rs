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
            0x06 => binary(&mut stack, |left, right| if right == 0 { 0 } else { left % right }),
            0x10 => binary(&mut stack, |left, right| u64::from(left < right)),
            0x11 => binary(&mut stack, |left, right| u64::from(left > right)),
            0x14 => binary(&mut stack, |left, right| u64::from(left == right)),
            0x15 => {
                let value = pop(&mut stack);
                stack.push(u64::from(value == 0));
            }
            0x16 => binary(&mut stack, |left, right| left & right),
            0x17 => binary(&mut stack, |left, right| left | right),
            0x18 => binary(&mut stack, |left, right| left ^ right),
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
            0x80 => {
                let value = *stack.last().expect("DUP1 requires stack item");
                stack.push(value);
            }
            0x90 => {
                let len = stack.len();
                if len < 2 {
                    panic!("SWAP1 requires two stack items");
                }
                stack.swap(len - 1, len - 2);
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

#[cfg(test)]
mod tests {
    use super::execute_bytecode;

    #[test]
    fn executes_stack_binary_template() {
        let bytecode = [0x60, 0x01, 0x60, 0x02, 0x01, 0x60, 0x01, 0x01, 0x50, 0x00];
        assert_eq!(execute_bytecode(&bytecode), 0);
    }

    #[test]
    fn executes_keccak_32_template() {
        let bytecode = [
            0x60, 0x00, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0x20, 0x50, 0x00,
        ];
        assert_eq!(execute_bytecode(&bytecode), 32);
    }
}
