"""Bounded independent byte interpreter for root transient-storage fixtures.

This oracle models attempted transient operations and frame rollback, without
turning ephemeral slots into persistent-storage proof or MPT resources.
"""
from collections import Counter

MASK = (1 << 256) - 1


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width


def execute(code, envelope, *, owner, transient=None):
    """Execute one frame; caller finalizes the transaction by clearing its map."""
    state = {} if transient is None else transient
    checkpoint = state.copy()
    try:
        return _execute(code, envelope, owner=owner, state=state)
    except (AssertionError, IndexError):
        state.clear(); state.update(checkpoint)
        raise


def _execute(code, envelope, *, owner, state):
    assert type(envelope) is int and envelope >= 21000, 'invalid gas envelope'
    assert isinstance(owner, str) and owner, 'invalid owner'
    initial = state.copy(); stack = []; memory = bytearray(); counts = Counter()
    reads = []; writes = []; journal = []; gas = 21000; pc = 0
    boundaries = {at for at, op, _ in decode(code) if op == 0x5b}

    def expand(offset):
        nonlocal gas
        assert 0 <= offset < 1024, 'unexpected memory address'
        old = len(memory) // 32; new = max(old, (offset + 63) // 32)
        gas += 3 * new + new * new // 512 - 3 * old - old * old // 512
        memory.extend(bytes(32 * (new - old)))

    def finish(op):
        nonlocal gas
        assert gas <= envelope, 'out of gas'
        if op in (0xfd, 0xfe):
            state.clear(); state.update(initial)
        if op == 0xfe: gas = envelope
        return dict(terminal=op, gas=gas, refund=0, applied=0, charged=gas,
            reads=reads, writes=writes, journal=journal, counts=counts, memory=memory, stack=stack,
            initial_transient=initial, frame_transient=state.copy())

    for _ in range(5_000_000):
        assert 0 <= pc < len(code), 'program counter outside code'
        op = code[pc]; pc += 1; counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f:
            width = op - 0x5f; stack.append(int.from_bytes(code[pc:pc + width], 'big')); pc += width; gas += 3
        elif op == 1:
            stack.append((stack.pop() + stack.pop()) & MASK); gas += 3
        elif op == 0x14:
            stack.append(int(stack.pop() == stack.pop())); gas += 3
        elif op == 0x15:
            stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); expand(offset); gas += 3
            if op == 0x51: stack.append(int.from_bytes(memory[offset:offset + 32], 'big'))
            else: memory[offset:offset + 32] = stack.pop().to_bytes(32, 'big')
        elif op == 0x5c:
            assert envelope - gas >= 100, 'out of gas'
            key = stack.pop(); value = state.get((owner, key), 0)
            reads.append((key, value)); stack.append(value); gas += 100
        elif op == 0x5d:
            # EIP-1153 charges exactly100 gas and has no EIP-2200 SSTORE sentry.
            assert envelope - gas >= 100, 'out of gas'
            key, new = stack.pop(), stack.pop(); old = state.get((owner, key), 0)
            writes.append((key, old, new)); journal.append((owner, key, old, new)); gas += 100
            if new: state[(owner, key)] = new
            else: state.pop((owner, key), None)
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0
            gas += 8 if op == 0x56 else 10
            if taken:
                assert target in boundaries, 'invalid jump destination'
                pc = target
        elif op == 0x5b: gas += 1
        elif op == 0xfd:
            offset, length = stack.pop(), stack.pop()
            assert offset == length == 0, 'unsupported nonzero REVERT range'
            return finish(op)
        elif op in (0, 0xfe): return finish(op)
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024, 'stack limit exceeded'
        assert gas <= envelope, 'out of gas'
    raise AssertionError('instruction bound exceeded')


def run_users(row):
    """Four transaction boundaries; frame traces survive root REVERT and clear."""
    program = row['request']['program']; transactions = program['transactions']
    contracts = {c['address']: c for c in program['prestate_contracts']}
    state = {}; records = []
    assert len(transactions['recipients']) == transactions['count_per_block'], 'recipient count'
    for recipient in transactions['recipients']:
        assert recipient in contracts, 'recipient lacks authenticated code account'
        code = bytes.fromhex(contracts[recipient]['code_hex'][2:])
        result = execute(code, transactions['gas_limit'], owner=recipient, transient=state)
        attempted = {(recipient, key) for key, _ in result['reads']} | {(recipient, key) for key, _, _ in result['writes']}
        state.clear()  # Transaction finalization, independent of STOP/REVERT.
        records.append(dict(recipient=recipient, owner=recipient, result=result,
            attempted_pairs=attempted, after_finalize=state.copy()))
    return records
