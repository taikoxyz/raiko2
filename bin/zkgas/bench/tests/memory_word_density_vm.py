"""Independent actual-byte stack/memory oracle; no producer imports.

A bounded bytearray models EVM memory. An optional memoized stateful body
transition is proved from every decoded instruction, including every read/write
address and alternating value. Its complete stack and target memory precondition
are rechecked on every use. Counter, tag, expansion, outer flow and guards execute
directly. This host optimization does not apply to actual EVM/SP1 execution.
"""
from collections import Counter

MASK = (1 << 256) - 1
BASE_ADDRESS = 0x3000000000000000000000000000000000010000
A = int.from_bytes(b'\xa5' * 32, 'big')
B = int.from_bytes(b'\x5a' * 32, 'big')


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1; width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width


def structure(instructions, operation, arena_bytes, residue, repetitions, tag, expected_a=A):
    """Bind complete byte intent, including non-executed R0 body and guards."""
    index = 0; targets = []; warmup = []
    def take(opcode, value=0):
        nonlocal index
        assert index < len(instructions), 'missing instruction'
        at, op, immediate = instructions[index]; index += 1
        assert op == opcode and (value is None or immediate == value), 'byte intent/guard shape'
        return at, immediate
    def push(value, width=32): return take(0x5f + width, value)
    def guard(value):
        push(value); take(0x14); take(0x15); targets.append(take(0x61, None)[1]); take(0x57)
    def load(offset): push(offset, 3); return take(0x51)[0]
    push(0, 1); push(arena_bytes - 1, 3); take(0x53)
    for offset, value in ((64, 0), (96, tag)):
        push(value); push(offset, 3); take(0x52)
    if operation == 'store':
        push(expected_a); push(B); push(residue, 3)
        for dup, value in ((0x82, expected_a), (0x81, B)):
            take(dup); take(0x81); warmup.append(take(0x52)[0]); load(residue); guard(value)
    else:
        push(B); push(residue, 3); take(0x52); push(residue, 3)
        take(0x80); warmup.append(take(0x51)[0]); guard(B)
    loop, _ = take(0x5b); load(64); push(repetitions); take(0x14)
    _, done_target = take(0x61, None); take(0x57)
    body = instructions[index][0]; body_pcs = []
    for site in range(64):
        if operation == 'store':
            take(0x82 if site % 2 == 0 else 0x81); take(0x81); body_pcs.append(take(0x52)[0])
        else: take(0x80); body_pcs.append(take(0x51)[0]); take(0x50)
    body_end = instructions[index][0]
    load(64); push(1, 3); take(1); push(64, 3); take(0x52); take(0x61, loop); take(0x56)
    done, _ = take(0x5b); assert done_target == done, 'done target'
    load(residue); guard(B); load(arena_bytes - 32); guard(0)
    for value in ((residue, B, expected_a) if operation == 'store' else (residue,)):
        take(0x80); guard(value); take(0x50)
    for offset, value in ((64, repetitions), (96, tag)): load(offset); guard(value)
    take(0); failure, _ = take(0x5b); take(0xfe)
    assert index == len(instructions) and targets == [failure] * (9 if operation == 'store' else 6), 'failure/extra instructions'
    return warmup, loop, body, body_end, body_pcs


def prove_body(instructions, entry_stack, entry_word, *, operation, offset, expected_a=A):
    """Prove COMPLETE stack plus ordered32-byte read/write footprint transition.

    Only the target32-byte region is writable. A decoded address outside this
    region fails before any transition can be cached. No full memory snapshots
    are taken. The proven transition preserves every byte outside that region.
    """
    expected_stack = [expected_a, B, offset] if operation == 'store' else [offset]
    assert list(entry_stack) == expected_stack and bytes(entry_word) == B.to_bytes(32, 'big'), 'body entry stack/memory'
    stack = list(entry_stack); word = bytes(entry_word); effects = []; counts = Counter(); gas = 0
    for at, op, immediate in instructions:
        assert immediate == 0, 'body immediate'
        counts[f'opcode:0x{op:02x}'] += 1
        if 0x80 <= op <= 0x82:
            depth = op - 0x7f; assert len(stack) >= depth, 'body stack underflow'
            stack.append(stack[-depth]); gas += 3
        elif op == 0x52:
            address, value = stack.pop(), stack.pop()
            assert operation == 'store' and address == offset, 'body write footprint'
            expected = expected_a if len(effects) % 2 == 0 else B
            assert value == expected and int.from_bytes(word, 'big') == (B if len(effects) % 2 == 0 else expected_a), 'alternating body writes'
            effects.append(dict(pc=at, offset=address, value=value, previous=int.from_bytes(word, 'big')))
            word = value.to_bytes(32, 'big'); gas += 3
        elif op == 0x51:
            address = stack.pop(); assert operation == 'load' and address == offset, 'body read footprint'
            value = int.from_bytes(word, 'big'); assert value == B, 'body read B'
            effects.append(dict(pc=at, offset=address, value=value, previous=value)); stack.append(value); gas += 3
        elif op == 0x50:
            assert operation == 'load' and stack.pop() == B, 'body result POP'; gas += 2
        else: raise AssertionError('unsupported body instruction')
    assert stack == expected_stack and word == B.to_bytes(32, 'big'), 'body complete stack/memory transition'
    assert len(effects) == 64, 'body target count'
    return dict(entry_stack=tuple(entry_stack), entry_word=bytes(entry_word), final_word=word, offset=offset,
        effects=effects, counts=counts, gas=gas, writes=64 if operation == 'store' else 0,
        reads=64 if operation == 'load' else 0, memory_growth=0)


def apply_body(proof, stack, memory):
    """Apply the proven stateful transition after checking its live preconditions."""
    offset = proof['offset']
    assert tuple(stack) == proof['entry_stack'], 'cached complete stack precondition'
    assert offset + 32 <= len(memory), 'cached in-bounds precondition'
    assert bytes(memory[offset:offset + 32]) == proof['entry_word'], 'cached memory feedback/precondition'
    # The proof enumerated every write at this one exact address. Composing those
    #64 writes leaves final_word; the footprint proof preserves all other bytes.
    if proof['writes']: memory[offset:offset + 32] = proof['final_word']


def execute(code, envelope, *, operation, arena_bytes, residue, repetitions, tag, owner=None, cache=True, expected_a=A):
    assert type(operation) is str and operation in ('store', 'load'), 'operation domain'
    assert type(arena_bytes) is int and arena_bytes in (256, 1048576), 'arena domain'
    assert type(residue) is int and residue in ((0, 1) if operation == 'store' else (1,)), 'offset domain'
    assert type(repetitions) is int and 0 <= repetitions <= 1_000_000, 'repetitions domain'
    assert type(tag) is int and tag in (0, 1) and type(cache) is bool, 'tag/cache domain'
    assert type(envelope) is int and envelope >= 21000 and len(code) < 24576, 'gas/runtime domain'
    owner = f'0x{BASE_ADDRESS + tag:040x}' if owner is None else owner
    assert owner == f'0x{BASE_ADDRESS + tag:040x}', 'owner identity'
    instructions = list(decode(code)); by_pc = {at: (op, value) for at, op, value in instructions}
    warmup, loop, body, body_end, body_pcs = structure(instructions, operation, arena_bytes, residue, repetitions, tag, expected_a)
    retained = [expected_a, B, residue] if operation == 'store' else [residue]
    proof = prove_body([i for i in instructions if body <= i[0] < body_end], retained, B.to_bytes(32, 'big'), operation=operation, offset=residue, expected_a=expected_a)
    jumps = {at for at, op, _ in instructions if op == 0x5b}; sites = {}
    for index, at in enumerate((*warmup, *body_pcs)):
        phase = 'warmup' if at in warmup else 'body'; site_index = index if phase == 'warmup' else index - len(warmup)
        value = expected_a if operation == 'store' and site_index % 2 == 0 else B
        sites[at] = dict(pc=at, phase=phase, offset=residue, value=value, visits=0)
    stack = []; memory = bytearray(); counts = Counter(); gas = 21000; expansion = 0; pc = 0; progression = []; iterations = 0
    target_sequence = 0; target_reads = 0; target_writes = 0
    for _ in range(30_000_000):
        assert pc in by_pc, 'invalid PC'
        if pc == loop:
            assert stack == retained, 'retained loop stack'
            value = int.from_bytes(memory[64:96], 'big'); assert value == len(progression), 'counter progression'
            assert int.from_bytes(memory[96:128], 'big') == tag, 'tag feedback'
            progression.append(value)
        if pc == body:
            assert int.from_bytes(memory[64:96], 'big') == iterations < repetitions, 'body counter'
            iterations += 1
            # Both cached and direct runs check the same actual memory entry.
            assert stack == retained and bytes(memory[residue:residue + 32]) == proof['entry_word'], 'body entry memory'
            if cache:
                apply_body(proof, stack, memory); counts.update(proof['counts']); gas += proof['gas']
                target_writes += proof['writes']; target_reads += proof['reads']; target_sequence += len(proof['effects'])
                for effect in proof['effects']: sites[effect['pc']]['visits'] += 1
                pc = body_end; assert gas <= envelope, 'out of gas'; continue
        at = pc; op, immediate = by_pc[pc]; pc += 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0)
        counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f: stack.append(immediate); gas += 3
        elif 0x80 <= op <= 0x82:
            depth = op - 0x7f; assert len(stack) >= depth, 'stack underflow'; stack.append(stack[-depth]); gas += 3
        elif op == 0x50: stack.pop(); gas += 2
        elif op in (1, 0x14):
            x, y = stack.pop(), stack.pop(); stack.append((x + y) & MASK if op == 1 else int(x == y)); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52, 0x53):
            offset = stack.pop(); length = 1 if op == 0x53 else 32
            assert offset in (residue, 64, 96, arena_bytes - 32, arena_bytes - 1), 'memory address domain'
            old = len(memory) // 32; new = max(old, (offset + length + 31) // 32)
            assert 32 * new <= arena_bytes, 'exact arena bound'
            delta = (3 * new + new * new // 512) - (3 * old + old * old // 512)
            expansion += delta; gas += 3 + delta; memory.extend(bytes(32 * (new - old)))
            if op == 0x51: value = int.from_bytes(memory[offset:offset + 32], 'big'); stack.append(value)
            else:
                value = stack.pop()
                if op == 0x53: memory[offset] = value & 255
                else: memory[offset:offset + 32] = value.to_bytes(32, 'big')
            if at in sites:
                site = sites[at]; assert (offset, value) == (site['offset'], site['value']), 'actual target address/value'
                if operation == 'store':
                    assert value == (expected_a if target_sequence % 2 == 0 else B), 'target alternation'
                    target_writes += 1
                else: assert value == B, 'target read B'; target_reads += 1
                target_sequence += 1; site['visits'] += 1
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0; gas += 8 if op == 0x56 else 10
            if taken: assert target in jumps, 'invalid jump'; pc = target
        elif op == 0x5b: gas += 1
        elif op in (0, 0xfe):
            assert op == 0 and stack == [], 'failed guard/terminal/stack'
            assert len(memory) == arena_bytes and int.from_bytes(memory[64:96], 'big') == repetitions == iterations, 'peak/counter'
            assert int.from_bytes(memory[96:128], 'big') == tag, 'final tag'
            assert bytes(memory[residue:residue + 32]) == B.to_bytes(32, 'big') and memory[-32:] == bytes(32), 'final memory'
            assert progression == list(range(repetitions + 1)), 'counter visits'
            assert all(site['visits'] == (1 if site['phase'] == 'warmup' else repetitions) for site in sites.values()), 'target visits'
            return dict(terminal=op, stack=stack, gas=gas, counts=counts, counter=repetitions, counter_progression=progression,
                target_operations=target_sequence, target_reads=target_reads, target_writes=target_writes,
                sites=list(sites.values()), peak_memory_bytes=len(memory), memory_expansion_gas=expansion,
                target_word_hex='0x' + memory[residue:residue + 32].hex(), last_word_hex='0x' + memory[-32:].hex())
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024 and gas <= envelope, 'stack bound/out of gas'
    raise AssertionError('instruction bound')


def run_users(row):
    p = row['parameters']; request = row['request']; program = request['program']; tx = program['transactions']; contracts = program['prestate_contracts']
    assert request['schema_version'] == 1 and request['expected_user_success'] is True, 'request contract'
    assert len(contracts) == 2 and all(c['initial_storage'] == {} for c in contracts), 'persistent domain'
    owners = [c['address'] for c in contracts]
    assert owners == [f'0x{BASE_ADDRESS + tag:040x}' for tag in range(2)], 'owner identities'
    assert tx == dict(kind='call', count_per_block=2, gas_limit=p['transaction_gas_limit'], recipients=owners), 'two user schedule'
    assert p['recipient_schedule'] == owners and p['owner_tags'] == [0, 1], 'owner metadata'
    assert (program['code_hex'], program['input_hex'], program['block_count']) == ('0x00', '0x', 1), 'outer domain'
    assert (p['value_a_hex'], p['value_b_hex']) == ('0x' + 'a5' * 32, '0x' + '5a' * 32), 'original value intent'
    codes = [bytes.fromhex(c['code_hex'][2:]) for c in contracts]; assert len(set(codes)) == 2, 'distinct tagged code'
    assert 0 < tx['gas_limit'] <= 15_000_000 and tx['gas_limit'] <= 16_777_216, 'transaction caps'
    results = [execute(code, tx['gas_limit'], operation=p['operation'], arena_bytes=p['arena_bytes'], residue=p['residue'], repetitions=p['repetitions'], tag=tag, owner=owners[tag]) for tag, code in enumerate(codes)]
    spent = 167355
    for result in results:
        assert 31_000_000 - spent >= tx['gas_limit'], 'header prefix cap'; spent += result['gas']
    assert spent <= 30_000_000, 'charged block cap'
    return results
