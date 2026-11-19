"""Independent actual-byte MCOPY interpreter; imports no producer or route formulas.

Copies retain source/destination/content hashes and an ordered sequence hash,
never repeated64KiB records. Final memory is Python byte intent, not native export.
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


def domain(layout):
    if layout == 'same-residue-disjoint': return 64, 65600, 131136
    if layout == 'different-residue-disjoint': return 64, 65601, 131137
    if layout == 'overlap-right': return 64, 65, 65601
    if layout == 'overlap-left': return 65, 64, 65601
    raise AssertionError('layout domain')


def execute(code, envelope, *, layout, tag, repetitions, owner=None, copy_mode='snapshot'):
    assert type(envelope) is int and envelope >= 21000, 'gas envelope'
    assert type(layout) is str, 'layout domain'
    src, dst, end = domain(layout); length = 65536
    assert type(tag) is int and tag in (0, 1), 'tag domain'
    assert type(repetitions) is int and 0 <= repetitions <= 1_000_000, 'repetitions domain'
    assert copy_mode in ('snapshot', 'forward', 'backward'), 'copy semantics domain'
    assert len(code) < 24576, 'code size'
    owner = f'0x{BASE_ADDRESS + tag:040x}' if owner is None else owner
    assert owner == f'0x{BASE_ADDRESS + tag:040x}', 'owner identity'
    instructions = list(decode(code)); jumps = {at for at, op, _ in instructions if op == 0x5b}
    assert instructions[-1][1] == 0xfe, 'failure terminal'
    size = ((end + 31) // 32) * 32
    expected = bytearray(size); expected[-32:] = bytes([238]) * 32; expected[32:64] = bytes([123]) * 32
    pattern = bytes(range(128 * tag, 128 * tag + 32))
    expected[64:65600] = pattern * 2048
    if dst > 65536:
        expected[dst:dst + length] = bytes(255 - value for value in pattern) * 2048
        if dst == 65601: expected[65600] = 205
    else: expected[65600] = pattern[0]
    writes = [(0x52, size - 32, int.from_bytes(bytes([238]) * 32, 'big')), (0x52, 0, 0),
        (0x52, 32, int.from_bytes(bytes([123]) * 32, 'big')), (0x52, 64, int.from_bytes(pattern, 'big'))]
    if dst > 65536: writes.append((0x52, dst, int.from_bytes(bytes(255 - v for v in pattern), 'big')))
    if dst == 65601: writes.append((0x53, 65600, 205))
    elif dst < 65536: writes.append((0x53, 65600, pattern[0]))
    stores = 0; prep = []
    for start in ([64, dst] if dst > 65536 else [64]):
        filled = 32
        while filled < length:
            chunk = min(filled, length - filled); prep.append((start, start + filled, chunk)); filled += chunk
    offsets = sorted({32, src, src + 32768, src + 65504, dst, dst + 32768, dst + 65504, size - 32} | ({65600} if dst == 65601 else set()))
    stack = []; memory = bytearray(); counts = Counter(); gas = 21000; pc = 0
    copies = []; sequence = hashlib.sha256(); loop_copies = 0; prepared = False; copy_gas = 0; guards_seen = []

    def expand(offset, count):
        nonlocal gas
        assert 0 <= offset <= size and 0 <= count <= size and offset + count <= size, 'memory range'
        old = len(memory) // 32; new = max(old, (offset + count + 31) // 32) if count else old
        gas += (3 * new + new * new // 512) - (3 * old + old * old // 512)
        memory.extend(bytes(32 * (new - old)))

    def word(offset): return int.from_bytes(memory[offset:offset + 32], 'big')

    for _ in range(1_000_000):
        assert 0 <= pc < len(code), 'execution range'
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; assert pc + width <= len(code), 'truncated PUSH'
            stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif op in (1, 0x14):
            assert len(stack) >= 2, 'stack underflow'; a, b = stack.pop(), stack.pop()
            stack.append((a + b) & MASK if op == 1 else int(a == b)); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52, 0x53):
            offset = stack.pop(); expand(offset, 1 if op == 0x53 else 32); gas += 3
            if op == 0x51:
                stack.append(word(offset))
                if prepared and loop_copies == repetitions and offset != 0: guards_seen.append(offset)
            else:
                value = stack.pop()
                if not prepared:
                    assert stores < len(writes) and (op, offset, value) == writes[stores], 'prepared memory store schedule'
                    stores += 1
                if op == 0x52: memory[offset:offset + 32] = value.to_bytes(32, 'big')
                else: memory[offset] = value & 255
        elif op == 0x5e:
            destination, source, count = stack.pop(), stack.pop(), stack.pop()
            if not prepared:
                index = len(copies); assert index < len(prep), 'extra preparation copy'
                assert (source, destination, count) == prep[index], 'preparation copy schedule'
                assert len(memory) == size, 'preallocation'
            else:
                assert (source, destination, count) == (src, dst, length), 'loop copy schedule'
                assert loop_copies < repetitions and word(0) == loop_copies, 'copy index/count'
            expand(source, count); expand(destination, count)
            content = bytes(memory[source:source + count]); cost = 3 + 3 * ((count + 31) // 32); gas += cost; copy_gas += cost
            # Alternative test-only implementations expose non-snapshot overlap bugs.
            if prepared and copy_mode != 'snapshot':
                indices = range(count) if copy_mode == 'forward' else reversed(range(count))
                for index in indices: memory[destination + index] = memory[source + index]
            else: memory[destination:destination + count] = content
            if prepared:
                intended = bytes(expected[src:src + length]); expected[dst:dst + length] = intended
                assert content == intended and bytes(memory[destination:destination + count]) == intended, 'copy content differs from snapshot intent'
                loop_copies += 1
            content_hash = hashlib.sha256(content).digest()
            record = dict(phase='loop' if prepared else 'preparation', index=loop_copies - 1 if prepared else len(copies), source=source, destination=destination, length=count, content_sha256=content_hash.hex(), gas=cost)
            sequence.update(bytes([int(prepared)]) + source.to_bytes(8, 'big') + destination.to_bytes(8, 'big') + count.to_bytes(8, 'big') + content_hash)
            copies.append(record)
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0; gas += 8 if op == 0x56 else 10
            if taken: assert target in jumps, 'invalid jump destination'; pc = target
        elif op == 0x5b:
            gas += 1
            if not prepared:
                assert len(copies) == len(prep) and stores == len(writes), 'incomplete preparation copies/stores'
                assert memory == expected, 'prepared memory bytes'
                prepared = True
        elif op in (0, 0xfe):
            assert op == 0, 'failed final guard/terminal'
            assert prepared and loop_copies == repetitions == word(0), 'counter/copy mismatch'
            expected[:32] = repetitions.to_bytes(32, 'big'); assert memory == expected, 'final memory intent'
            assert guards_seen == offsets, 'final sampled guard offsets/order'
            assert stack == [], 'terminal stack'
            return dict(terminal=op, gas=gas, counts=counts, stack=stack, memory=memory,
                counter=word(0), loop_copies=loop_copies, copies=copies, mcopy_gas=copy_gas,
                sequence_sha256=sequence.hexdigest(), memory_sha256=hashlib.sha256(memory).hexdigest(),
                guards=[f'0x{word(offset):064x}' for offset in offsets])
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024, 'stack limit'; assert gas <= envelope, 'out of gas'
    raise AssertionError('instruction bound')


def run_users(row):
    p = row['parameters']; program = row['request']['program']; tx = program['transactions']; contracts = program['prestate_contracts']
    assert len(contracts) == 2 and all(c['initial_storage'] == {} for c in contracts), 'persistent domain'
    owners = [c['address'] for c in contracts]
    assert len(set(owners)) == 2 and owners == [f'0x{BASE_ADDRESS + tag:040x}' for tag in range(2)], 'distinct owner identities'
    assert tx['kind'] == 'call' and tx['count_per_block'] == len(tx['recipients']) == 2, 'two user schedule'
    assert tx['recipients'] == owners == p['recipient_schedule'], 'owner call schedule'
    assert p['owner_tags'] == [0, 1] and program['input_hex'] == '0x' and program['block_count'] == 1, 'outer domain'
    codes = [bytes.fromhex(c['code_hex'][2:]) for c in contracts]; assert len(set(codes)) == 2, 'distinct tagged code'
    assert [op for _, op, _ in decode(codes[0])] == [op for _, op, _ in decode(codes[1])], 'code shape'
    return [execute(code, tx['gas_limit'], layout=p['layout'], tag=tag, repetitions=p['repetitions'], owner=owners[tag]) for tag, code in enumerate(codes)]
