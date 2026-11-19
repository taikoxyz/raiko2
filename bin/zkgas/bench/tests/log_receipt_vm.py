"""Bounded independent byte EVM for LOG intent; imports no producer/route counts.

Events retain compact byte digests and ordered topics; this is not an independent
receipt-root implementation or a native per-log export.
"""
from collections import Counter
import hashlib

MASK = (1 << 256) - 1
BASE_ADDRESS = 0x3000000000000000000000000000000000010000


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1; width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width


def execute(code, envelope, *, topics, length, pattern, tag, repetitions, owner=None):
    assert type(envelope) is int and envelope >= 21000, 'gas envelope'
    assert type(topics) is int and topics in (1, 4), 'topics domain'
    assert type(length) is int and length in (0, 64, 262144), 'length domain'
    assert type(pattern) is str and pattern in ('zero', 'a5') and (length or pattern == 'zero'), 'pattern domain'
    assert type(tag) is int and tag in (0, 1), 'tag domain'
    assert type(repetitions) is int and 0 <= repetitions <= 1_000_000, 'repetitions domain'
    assert len(code) < 24576, 'code size'
    owner = f'0x{BASE_ADDRESS + tag:040x}' if owner is None else owner
    assert owner == f'0x{BASE_ADDRESS + tag:040x}', 'owner identity'
    instructions = list(decode(code)); jumps = {at for at, op, _ in instructions if op == 0x5b}
    assert instructions[-1][1] == 0xfe, 'failure terminal'
    stack = []; memory = bytearray(); counts = Counter(); gas = 21000; pc = 0
    logs = []; sequence = hashlib.sha256(); extent = max(32, length); max_memory = 64 + extent
    copied = 32; copies = []; copy_gas = log_gas = 0; expected_seed = bytes([0 if pattern == 'zero' else 0xa5]) * 32
    expected_tail = bytes([0 if pattern == 'zero' else 0xa5]) * max(0, length - 32)
    prepared = False

    def expand(offset, size):
        nonlocal gas
        assert 0 <= offset <= max_memory and 0 <= size <= extent and offset + size <= max_memory, 'memory range'
        old = len(memory) // 32; new = max(old, (offset + size + 31) // 32) if size else old
        gas += 3 * new + new * new // 512 - 3 * old - old * old // 512
        memory.extend(bytes(32 * (new - old)))

    def word(offset): return int.from_bytes(memory[offset:offset + 32], 'big')

    def check_prepared():
        nonlocal prepared
        if prepared: return
        assert len(memory) == max_memory and copied == extent, 'incomplete preallocation/copy'
        assert bytes(memory[64:]) == expected_seed + expected_tail, 'prepared payload bytes'
        assert word(0) == word(32) == 0, 'initial counters'
        prepared = True

    def finish(op):
        assert gas <= envelope, 'out of gas'
        if op == 0:
            check_prepared()
            assert len(logs) == repetitions == word(0), 'counter/log mismatch'
            assert word(32) == (((tag << 128) | (repetitions - 1)) if repetitions else 0), 'last input guard'
            assert bytes(memory[96:]) == expected_tail, 'payload tail changed'
            assert stack == [], 'terminal stack'
        return dict(terminal=op, gas=gas, counts=counts, stack=stack, memory=memory, logs=logs,
            log_count=len(logs), sequence_sha256=sequence.hexdigest(), counter=word(0), last_input=word(32),
            copies=copies, mcopy_gas=copy_gas, log_gas=log_gas)

    for _ in range(10_000_000):
        assert 0 <= pc < len(code), 'execution range'
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; assert pc + width <= len(code), 'truncated PUSH'
            stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif op in (1, 0x14, 0x17):
            assert len(stack) >= 2, 'stack underflow'; a, b = stack.pop(), stack.pop()
            stack.append(((a + b) & MASK) if op == 1 else int(a == b) if op == 0x14 else a | b); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); expand(offset, 32); gas += 3
            if op == 0x51: stack.append(word(offset))
            else: memory[offset:offset + 32] = stack.pop().to_bytes(32, 'big')
        elif op == 0x5e:
            dest, source, size = stack.pop(), stack.pop(), stack.pop()
            assert (dest, source, size) == (64 + copied, 64, min(copied, extent - copied)), 'MCOPY schedule'
            assert size > 0 and not logs and not prepared, 'MCOPY phase'
            assert len(memory) == max_memory and memory[64:96] == expected_seed, 'preallocation/seed'
            expand(source, size); expand(dest, size); cost = 3 + 3 * ((size + 31) // 32); gas += cost; copy_gas += cost
            memory[dest:dest + size] = bytes(memory[source:source + size]); copied += size
            copies.append(dict(source=source, destination=dest, length=size, gas=cost))
        elif op in (0xa1, 0xa4):
            start, size = stack.pop(), stack.pop(); n = op - 0xa0
            actual_topics = [stack.pop() for _ in range(n)]
            assert (n, start, size) == (topics, 64, length), 'LOG operands'
            assert prepared, 'LOG before payload preparation'
            expand(start, size); index = len(logs)
            assert index < repetitions and word(0) == index, 'LOG index/count'
            expected_topics = [(tag << 128) | (j << 64) | index for j in range(topics)]
            assert actual_topics == expected_topics, 'literal ordered topics'
            actual = bytes(memory[start:start + size]); expected = ((tag << 128) | index).to_bytes(32, 'big') + expected_tail if length else b''
            assert actual == expected, 'literal ordered log data bytes'
            assert word(32) == word(64) == ((tag << 128) | index), 'last-input counter order'
            cost = 375 + 375 * n + 8 * size; gas += cost; log_gas += cost; data_digest = hashlib.sha256(actual).digest()
            sequence.update(bytes.fromhex(owner[2:]) + bytes([n]) + b''.join(v.to_bytes(32, 'big') for v in actual_topics) + size.to_bytes(8, 'big') + data_digest)
            logs.append(dict(address=owner, index=index, topics=[f'0x{v:064x}' for v in actual_topics],
                offset=start, length=size, data_sha256=data_digest.hex(), gas=cost))
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0; gas += 8 if op == 0x56 else 10
            if taken:
                assert target in jumps, 'invalid jump destination'; pc = target
        elif op == 0x5b:
            gas += 1
            if not prepared: check_prepared()
        elif op in (0, 0xfe): return finish(op)
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024, 'stack limit'; assert gas <= envelope, 'out of gas'
    raise AssertionError('instruction bound')


def run_users(row):
    p = row['parameters']; program = row['request']['program']; tx = program['transactions']; contracts = program['prestate_contracts']
    assert len(contracts) == 2 and all(c['initial_storage'] == {} for c in contracts), 'persistent domain'
    owners = [c['address'] for c in contracts]
    assert len(set(owners)) == 2 and owners == [f'0x{BASE_ADDRESS + tag:040x}' for tag in range(2)], 'distinct owner identities'
    assert tx['count_per_block'] == len(tx['recipients']) == 2, 'two user schedule'
    assert tx['recipients'] == owners == p['recipient_schedule'], 'owner call schedule'
    assert p['owner_tags'] == [0, 1] and program['input_hex'] == '0x' and program['block_count'] == 1, 'outer domain'
    codes = [bytes.fromhex(c['code_hex'][2:]) for c in contracts]; assert len(set(codes)) == 2, 'distinct tagged code'
    assert [op for _, op, _ in decode(codes[0])] == [op for _, op, _ in decode(codes[1])], 'code shape'
    results = [execute(code, tx['gas_limit'], topics=p['topics'], length=p['input_bytes'], pattern=p['pattern'], tag=tag, repetitions=p['repetitions'], owner=owners[tag]) for tag, code in enumerate(codes)]
    assert all(r['terminal'] == 0 for r in results), 'user execution failed'
    identities = [(log['address'], tuple(log['topics']), log['data_sha256']) for r in results for log in r['logs']]
    assert len(set(identities)) == len(identities), 'duplicate event identity'
    return results
