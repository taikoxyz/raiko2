"""Bounded, independent actual-byte SHA256 wrapper oracle, not a PGU model.

No producer imports or declared route multiplicities drive execution. Each call's
ordered bytes and stdlib digest are checked before counter/XOR qualification.
"""
from collections import Counter
import hashlib

MASK = (1 << 256) - 1


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width
    assert pc == len(code), 'decode range'


def execute(code, envelope, *, pattern, length, tag, repetitions, override=None):
    assert type(envelope) is int and envelope >= 21000, 'invalid gas envelope'
    assert type(pattern) is str and pattern in ('zero', 'a5'), 'pattern domain'
    assert type(length) is int and length in (64, 4096, 1048576, 1269664, 1269665, 2097152), 'length domain'
    assert type(tag) is int and tag in (0, 1), 'tag domain'
    assert type(repetitions) is int and 0 <= repetitions <= 1_000_000, 'repetition domain'
    assert len(code) <= 24576, 'code size'
    instructions = list(decode(code)); jumps = {at for at, op, _ in instructions if op == 0x5b}
    assert instructions[-1][1] == 0xfe, 'missing failure terminal'
    override = {} if override is None else override
    stack = []; memory = bytearray(); counts = Counter(); gas = 21000; pc = 0; calls = []
    output = 64 + length; max_memory = output + 32; copied = 32; copies = []; copy_gas = 0
    expected_tail = bytes([0 if pattern == 'zero' else 0xa5]) * (length - 32)
    returndata_size = 0; attempted = completed = 0; xor = 0

    def expand(offset, size):
        nonlocal gas
        assert 0 <= offset <= max_memory and 0 <= size <= length, 'memory range'
        assert offset + size <= max_memory, 'memory range'
        old = len(memory) // 32; new = max(old, (offset + size + 31) // 32) if size else old
        gas += 3 * new + new * new // 512 - 3 * old - old * old // 512
        memory.extend(bytes(32 * (new - old)))

    def word(offset): return int.from_bytes(memory[offset:offset + 32], 'big')

    def finish(op):
        assert gas <= envelope, 'out of gas'
        if op == 0:
            assert copied == length and copies, 'missing pattern preparation'
            assert bytes(memory[96:output]) == expected_tail, 'prepared tail input bytes'
            assert len(calls) == attempted == completed == repetitions == word(0), 'counter/call mismatch'
            assert word(32) == xor, 'digest XOR mismatch'
            assert word(output) == (2 if not calls else int.from_bytes(calls[-1]['digest'], 'big')), 'output digest'
            assert not any(memory[max_memory:]), 'memory padding modified'
        return dict(terminal=op, gas=gas, counts=counts, stack=stack, memory=memory,
            calls=calls, attempted_calls=attempted, completed_calls=completed, counter=word(0),
            accumulator=word(32), output=word(output), copies=copies, mcopy_gas=copy_gas)

    for _ in range(10_000_000):
        assert 0 <= pc < len(code), 'execution range'
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; assert pc + width <= len(code), 'truncated executed PUSH'
            stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif op in (1, 0x14, 0x17, 0x18):
            a, b = stack.pop(), stack.pop()
            stack.append(((a + b) & MASK) if op == 1 else int(a == b) if op == 0x14 else a | b if op == 0x17 else a ^ b); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); expand(offset, 32); gas += 3
            if op == 0x51: stack.append(word(offset))
            else: memory[offset:offset + 32] = stack.pop().to_bytes(32, 'big')
        elif op == 0x5e:
            dest, source, size = stack.pop(), stack.pop(), stack.pop()
            assert (dest, source, size) == (64 + copied, 64, min(copied, length - copied)), 'MCOPY schedule'
            assert size > 0 and not attempted, 'MCOPY phase'
            assert len(memory) == 32 * ((max_memory + 31) // 32), 'output not preallocated'
            if not copies:
                assert memory[64:96] == bytes([0 if pattern == 'zero' else 0xa5]) * 32, 'seed input bytes'
                assert word(0) == word(32) == 0 and word(output) == 2, 'initial words'
            expand(source, size); expand(dest, size)
            cost = 3 + 3 * ((size + 31) // 32); gas += cost; copy_gas += cost
            image = bytes(memory[source:source + size]); memory[dest:dest + size] = image
            copied += size; copies.append(dict(source=source, destination=dest, length=size, gas=cost))
        elif op == 0x5a:
            gas += 2; assert gas <= envelope, 'out of gas'; stack.append(envelope - gas)
        elif op == 0xfa:
            requested, target, start, size, out, out_size = (stack.pop() for _ in range(6))
            assert (target, start, size, out, out_size) == (2, 64, length, output, 32), 'STATICCALL operands'
            assert copied == length, 'incomplete input copy'
            expand(start, size); expand(out, out_size); gas += 100
            assert gas <= envelope, 'out of gas'
            available = envelope - gas; forwarded = min(requested, available - available // 64)
            actual = bytes(memory[start:start + size]); index = attempted
            expected = ((tag << 128) | index).to_bytes(32, 'big') + expected_tail
            assert actual == expected, 'literal ordered input bytes'
            assert word(0) == index, 'input counter order'
            sentinel = word(out); assert sentinel == 2, 'output sentinel not reset'
            digest = hashlib.sha256(actual).digest(); native = 60 + 12 * ((size + 31) // 32)
            assert forwarded >= native, 'insufficient forwarded gas'
            gas += native; attempted += 1
            status = override.get('status', 1); returndata_size = override.get('return_size', 32)
            returned = (int.from_bytes(digest, 'big') ^ int(override.get('wrong_digest', False))).to_bytes(32, 'big')
            assert returned == digest, 'per-call returned digest'
            write_size = min(out_size, returndata_size, override.get('write_size', 32))
            memory[out:out + write_size] = returned[:write_size]; stack.append(status)
            if status == 1: completed += 1
            xor ^= int.from_bytes(digest, 'big')
            calls.append(dict(tag=tag, index=index, input_sha256=digest.hex(), digest=digest,
                actual_return=returned, sentinel=sentinel, available_gas=available, requested_gas=requested,
                forwarded_gas=forwarded, returned_gas=forwarded - native, native_gas=native,
                status=status, return_size=returndata_size, gas_after_call=envelope - gas))
        elif op == 0x3d: stack.append(returndata_size); gas += 2
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0
            gas += 8 if op == 0x56 else 10
            if taken:
                assert target in jumps, 'invalid jump destination'; pc = target
        elif op == 0x5b: gas += 1
        elif op in (0, 0xfe): return finish(op)
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024, 'stack limit exceeded'
        assert gas <= envelope, 'out of gas'
    raise AssertionError('instruction bound exceeded')


def run_users(row):
    p = row['parameters']; program = row['request']['program']; tx = program['transactions']; contracts = program['prestate_contracts']
    assert len(contracts) == 2 and all(c['initial_storage'] == {} for c in contracts), 'persistent domain'
    owners = [c['address'] for c in contracts]
    assert len(set(owners)) == 2, 'distinct owners'
    assert tx['count_per_block'] == len(tx['recipients']) == 2, 'two user schedule'
    assert tx['recipients'] == owners == p['recipient_schedule'], 'owner call schedule'
    assert p['owner_tags'] == [0, 1], 'owner tags'
    assert program['input_hex'] == '0x' and program['block_count'] == 1, 'outer transaction domain'
    codes = [bytes.fromhex(c['code_hex'][2:]) for c in contracts]
    assert len(set(codes)) == 2, 'distinct tagged code'
    shapes = [[op for _, op, _ in decode(code)] for code in codes]
    assert shapes[0] == shapes[1], 'instruction shape'
    results = [execute(code, tx['gas_limit'], pattern=p['pattern'], length=p['input_bytes'], tag=tag,
        repetitions=p['repetitions']) for tag, code in enumerate(codes)]
    assert all(r['terminal'] == 0 for r in results), 'user execution failed'
    identities = [c['input_sha256'] for r in results for c in r['calls']]
    assert len(set(identities)) == len(identities), 'duplicate call input'
    return results
