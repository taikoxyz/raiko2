"""Independent decoded-byte modular oracle; no producer imports or count formulas.

The optional cache proves each decoded pure site on the actual complete stack once,
then checks that same stack at every body entry. Outer memory/counter/control flow
always executes directly. Grouped records preserve every target PC and its visits.
"""
from collections import Counter

MASK = (1 << 256) - 1
BASE_ADDRESS = 0x3000000000000000000000000000000000010000
OPCODES = {'addmod': 8, 'mulmod': 9}


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1; width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width


def operands(operation, modulus_class):
    assert type(operation) is str and operation in OPCODES, 'operation domain'
    assert type(modulus_class) is str and modulus_class in ('limb1', 'limb2', 'limb3', 'limb4'), 'modulus domain'
    return (MASK - 1, MASK - 2,
        {'limb1': 3, 'limb2': 2**64 + 1, 'limb3': 2**128 + 1, 'limb4': 2**192 + 1}[modulus_class])


def arithmetic(operation, a, b, modulus):
    assert operation in OPCODES and all(type(v) is int and 0 <= v <= MASK for v in (a, b, modulus)), 'arithmetic domain'
    if modulus == 0: return 0
    # Python integers preserve the full 257/512-bit intermediate before reduction.
    return (a + b if operation == 'addmod' else a * b) % modulus


def structure(instructions, operation, a, b, modulus, repetitions, tag):
    """Validate actual constant-width guards, pure body, loop and failure wiring."""
    index = 0; targets = []
    def take(opcode, value=0):
        nonlocal index
        assert index < len(instructions), 'missing instruction'
        at, op, immediate = instructions[index]; index += 1
        assert op == opcode and (immediate == value or value is None), 'byte intent/guard shape'
        return at, immediate
    def push(value, width=32): return take(0x5f + width, value)
    def guard(value):
        push(value); take(0x14); take(0x15); targets.append(take(0x61, None)[1]); take(0x57)
    for offset, value in ((0, 0), (32, tag)):
        push(value); push(offset, 3); take(0x52)
    push(modulus); push(b); push(a); take(0x82); take(0x82); take(0x82)
    warmup, _ = take(OPCODES[operation]); guard(arithmetic(operation, a, b, modulus))
    loop, _ = take(0x5b); push(0, 3); take(0x51); push(repetitions); take(0x14)
    _, done_target = take(0x61, None); take(0x57)
    body = instructions[index][0]; body_pcs = []
    for _ in range(64):
        take(0x82); take(0x82); take(0x82); at, _ = take(OPCODES[operation]); body_pcs.append(at); take(0x50)
    body_end = instructions[index][0]
    push(0, 3); take(0x51); push(1, 3); take(1); push(0, 3); take(0x52)
    take(0x61, loop); take(0x56); done, _ = take(0x5b); assert done_target == done, 'done target'
    for value in (a, b, modulus): take(0x80); guard(value); take(0x50)
    for offset, value in ((0, repetitions), (32, tag)):
        push(offset, 3); take(0x51); guard(value)
    take(0); failure, _ = take(0x5b); take(0xfe)
    assert index == len(instructions) and targets == [failure] * 6, 'failure/extra instructions'
    return warmup, loop, body, body_end, body_pcs


def execute(code, envelope, *, operation, modulus_class, repetitions, tag, owner=None, cache=True):
    assert type(envelope) is int and envelope >= 21000, 'gas envelope'
    assert type(repetitions) is int and 0 <= repetitions <= 1_000_000, 'repetitions domain'
    assert type(tag) is int and tag in (0, 1), 'tag domain'
    assert type(cache) is bool, 'cache domain'
    assert len(code) < 24576, 'code size'
    owner = f'0x{BASE_ADDRESS + tag:040x}' if owner is None else owner
    assert owner == f'0x{BASE_ADDRESS + tag:040x}', 'owner identity'
    a, b, modulus = operands(operation, modulus_class); expected = arithmetic(operation, a, b, modulus)
    instructions = list(decode(code)); by_pc = {at: (op, value) for at, op, value in instructions}
    warmup, loop, body, body_end, body_pcs = structure(instructions, operation, a, b, modulus, repetitions, tag)
    jumps = {at for at, op, _ in instructions if op == 0x5b}
    sites = {at: dict(pc=at, phase='warmup' if at == warmup else 'body', operand_a=a, operand_b=b, modulus=modulus,
        result=expected, visits=0) for at in (warmup, *body_pcs)}
    stack = []; memory = bytearray(); counts = Counter(); gas = 21000; pc = 0; progression = []
    stores = 0; iterations = 0
    # Derive pure-block counts and prove the full input/output stack from decoded bytes.
    pure_counts = Counter(); pure_cost = 0; proof_stack = [modulus, b, a]
    for at, op, _ in instructions:
        if not body <= at < body_end: continue
        pure_counts[f'opcode:0x{op:02x}'] += 1
        if op == 0x82: proof_stack.append(proof_stack[-3]); pure_cost += 3
        elif op == OPCODES[operation]:
            x, y, m = proof_stack.pop(), proof_stack.pop(), proof_stack.pop()
            assert (x, y, m) == (a, b, modulus), 'pure body ordered operands'
            value = arithmetic(operation, x, y, m); assert value == expected, 'pure body result'
            proof_stack.append(value); pure_cost += 8
        elif op == 0x50: assert proof_stack.pop() == expected, 'pure body POP'; pure_cost += 2
        else: raise AssertionError('stateful/non-pure body')
    assert proof_stack == [modulus, b, a], 'pure body complete stack invariant'
    for _ in range(30_000_000):
        assert pc in by_pc, 'invalid execution PC'
        if pc == loop:
            assert stack == [modulus, b, a], 'retained operands at loop'
            value = int.from_bytes(memory[:32], 'big')
            assert value == len(progression), 'counter progression'
            progression.append(value)
        if pc == body:
            assert stack == [modulus, b, a] and int.from_bytes(memory[:32], 'big') == iterations < repetitions, 'body entry invariant'
            iterations += 1
            if cache:
                counts.update(pure_counts); gas += pure_cost
                for at in body_pcs: sites[at]['visits'] += 1
                pc = body_end; assert gas <= envelope, 'out of gas'; continue
        at = pc; op, immediate = by_pc[pc]; pc += 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0)
        counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f: stack.append(immediate); gas += 3
        elif op in (0x80, 0x82):
            depth = op - 0x7f; assert len(stack) >= depth, 'stack underflow'; stack.append(stack[-depth]); gas += 3
        elif op == 0x50: stack.pop(); gas += 2
        elif op in (8, 9):
            x, y, m = stack.pop(), stack.pop(), stack.pop(); value = arithmetic(operation, x, y, m)
            assert (x, y, m, value) == (a, b, modulus, expected), 'ordered operands/results'
            sites[at]['visits'] += 1; stack.append(value); gas += 8
        elif op in (1, 0x14):
            x, y = stack.pop(), stack.pop(); stack.append((x + y) & MASK if op == 1 else int(x == y)); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); assert offset in (0, 32), 'memory domain'
            old = len(memory) // 32; new = max(old, offset // 32 + 1)
            gas += 3 + 3 * (new - old); memory.extend(bytes(32 * (new - old)))
            if op == 0x51: stack.append(int.from_bytes(memory[offset:offset + 32], 'big'))
            else:
                value = stack.pop()
                assert (offset, value) == ((0, 0) if stores == 0 else (32, tag) if stores == 1 else (0, stores - 1)), 'store schedule'
                stores += 1; memory[offset:offset + 32] = value.to_bytes(32, 'big')
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0; gas += 8 if op == 0x56 else 10
            if taken: assert target in jumps, 'invalid jump'; pc = target
        elif op == 0x5b: gas += 1
        elif op in (0, 0xfe):
            assert op == 0 and stack == [], 'failed guard/terminal/stack'
            assert int.from_bytes(memory[:32], 'big') == repetitions == iterations, 'counter mismatch'
            assert int.from_bytes(memory[32:], 'big') == tag and stores == repetitions + 2, 'protected tag/store count'
            assert progression == list(range(repetitions + 1)), 'counter visits'
            assert sites[warmup]['visits'] == 1 and all(sites[at]['visits'] == repetitions for at in body_pcs), 'target visits'
            return dict(terminal=op, stack=stack, gas=gas, counts=counts, counter=repetitions,
                counter_progression=progression, target_operations=sum(s['visits'] for s in sites.values()), sites=list(sites.values()))
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024 and gas <= envelope, 'out of gas'
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
    codes = [bytes.fromhex(c['code_hex'][2:]) for c in contracts]; assert len(set(codes)) == 2, 'distinct tagged code'
    assert 0 < tx['gas_limit'] <= 15_000_000 and tx['gas_limit'] <= 16_777_216, 'transaction caps'
    results = [execute(code, tx['gas_limit'], operation=p['operation'], modulus_class=p['modulus_class'], repetitions=p['repetitions'], tag=tag, owner=owners[tag]) for tag, code in enumerate(codes)]
    spent = 167355
    for result in results:
        assert 31_000_000 - spent >= tx['gas_limit'], 'header prefix cap'; spent += result['gas']
    assert spent <= 30_000_000, 'charged block cap'
    return results
