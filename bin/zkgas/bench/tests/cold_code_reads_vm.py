"""Bounded decoded-byte cold-account oracle, independent of the producer.

Address/value commitments are debug intent, not Ethereum roots or native per-read
exports. Execution stores at most512 cold addresses and eight samples, no trace.
"""
from collections import Counter
import hashlib

BASE = 0x4000000000000000000000000000000000010000
PRIMARY = '0x' + '66' * 20
MASK = (1 << 256) - 1
OPS = {'balance': 0x31, 'size': 0x3b}


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width


def structure(code, operation, visits):
    """Check every instruction, immediate width and branch, including unvisited body."""
    instructions = iter(decode(code)); targets = []
    def take(opcode, value=0):
        at, op, immediate = next(instructions, (-1, -1, -1))
        assert op == opcode and (value is None or immediate == value), 'byte intent/guard shape'
        return at, immediate
    def push(value, width=32): return take(0x5f + width, value)
    def load(): push(0, 3); take(0x51)
    def require(value):
        push(value); take(0x14); take(0x15); targets.append(take(0x61, None)[1]); take(0x57)
    push(0); push(0, 3); take(0x52)
    loop, _ = take(0x5b); load(); push(visits); take(0x14)
    _, done_target = take(0x61, None); take(0x57)
    load(); push(BASE, 20); take(1); take(OPS[operation]); require(0 if operation == 'balance' else 24576)
    load(); push(1, 3); take(1); push(0, 3); take(0x52); take(0x61, loop); take(0x56)
    done, _ = take(0x5b); assert done_target == done, 'done target'
    load(); require(visits); take(0); failure, _ = take(0x5b); take(0xfe)
    assert next(instructions, None) is None and targets == [failure, failure], 'failure/extra instructions'
    return loop


def execute(code, envelope, *, operation, footprint, visits, contracts):
    assert type(operation) is str and operation in OPS, 'operation domain'
    assert type(footprint) is int and footprint in (16, 128, 512), 'footprint domain'
    assert type(visits) is int and 0 <= visits <= footprint, 'visits domain'
    assert type(envelope) is int and envelope >= 21000, 'gas envelope'
    assert 0 < len(code) < 24576, 'runtime size'
    assert len(contracts) == footprint, 'target count'
    world = {}
    for i, contract in enumerate(contracts):
        address = BASE + i
        assert contract['address'] == f'0x{address:040x}', 'target address/order'
        assert contract.get('initial_storage') == {}, 'target empty persistent storage'
        body = bytes.fromhex(contract['code_hex'][2:])
        assert len(body) == 24576 and body[:34] == b'\x7f' + (i + 1).to_bytes(32, 'big') + b'\0', 'target length/tag/prefix'
        assert body[34:] == b'\x5b' * 24542, 'target tail content intent'
        world[address] = len(body)
    loop = structure(code, operation, visits)
    instructions = {pc: (op, value) for pc, op, value in decode(code)}
    jumps = {pc for pc, (op, _) in instructions.items() if op == 0x5b}
    stack = []; memory = bytearray(); gas = 21000; pc = 0; counts = Counter()
    cold = set(); samples = []; sequence = hashlib.sha256(); counter_sequence = hashlib.sha256()
    stores = reads = loop_visits = 0
    for _ in range(100_000):
        assert pc in instructions, 'invalid PC'
        if pc == loop:
            counter = int.from_bytes(memory, 'big')
            assert stack == [] and counter == loop_visits <= visits, 'counter/empty loop stack'
            counter_sequence.update(counter.to_bytes(32, 'big')); loop_visits += 1
        op, immediate = instructions[pc]; pc += 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0)
        counts[f'opcode:0x{op:02x}'] += 1
        if 0x60 <= op <= 0x7f: stack.append(immediate); gas += 3
        elif op in (1, 0x14):
            a, b = stack.pop(), stack.pop(); stack.append((a + b) & MASK if op == 1 else int(a == b)); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op in (0x51, 0x52):
            assert stack.pop() == 0, 'memory offset'
            if not memory: memory.extend(bytes(32)); gas += 3
            gas += 3
            if op == 0x51: stack.append(int.from_bytes(memory, 'big'))
            else:
                value = stack.pop(); assert value == stores, 'counter store progression'
                memory[:] = value.to_bytes(32, 'big'); stores += 1
        elif op in (0x31, 0x3b):
            address = stack.pop(); assert op == OPS[operation], 'operation intent'
            assert address == BASE + reads and address in world, 'ordered account read'
            assert address not in cold, 'target must be cold once'
            cold.add(address); value = 0 if op == 0x31 else world[address]; gas += 2600
            stack.append(value); sequence.update(address.to_bytes(20, 'big') + value.to_bytes(32, 'big'))
            if len(samples) < 8: samples.append(dict(index=reads, address=f'0x{address:040x}', value=value, cold=True))
            reads += 1
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0; gas += 8 if op == 0x56 else 10
            if taken: assert target in jumps, 'invalid jump'; pc = target
        elif op == 0x5b: gas += 1
        elif op in (0, 0xfe):
            assert op == 0 and stack == [], 'failed guard/terminal/empty stack'
            assert int.from_bytes(memory, 'big') == reads == visits, 'final counter/read count'
            assert stores == visits + 1 and loop_visits == visits + 1, 'counter progression count'
            return dict(gas=gas, counts=counts, terminal=op, stack=stack, counter=reads,
                cold_count=len(cold), warm_count=0, samples=samples,
                ordered_address_value_sha256=sequence.hexdigest(), counter_sequence_sha256=counter_sequence.hexdigest())
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert gas <= envelope and len(stack) <= 1024, 'out of gas/stack'
    raise AssertionError('instruction bound')


def run(row):
    p = row['parameters']; request = row['request']; program = request['program']; tx = program['transactions']
    assert request['schema_version'] == 1 and request['expected_user_success'] is True, 'request domain'
    assert p['primary_code_owner_address'] == PRIMARY and p['primary_code_owner_count'] == 1, 'primary owner'
    assert p['all_user_transactions'] == 1 and p['outer_calldata_bytes'] == 0, 'one user'
    assert tx == dict(kind='call', count_per_block=1, gas_limit=p['transaction_gas_limit']), 'transaction schedule'
    assert program['input_hex'] == '0x' and program['block_count'] == 1, 'outer domain'
    assert set(program) == {'code_hex', 'input_hex', 'block_count', 'transactions', 'prestate_contracts', 'expected_route_counts'}, 'program domain'
    contracts = program['prestate_contracts']; footprint = p['footprint']; visits = p['visits']
    assert p['workload_level'] in ('zero', 'selected') and visits == (0 if p['workload_level'] == 'zero' else footprint), 'level visits'
    assert p['prestate_target_count'] == footprint and p['visited_target_count'] == visits, 'target metadata'
    assert p['target_base_address'] == f'0x{BASE:040x}', 'target base metadata'
    assert p['target_code_bytes'] == 24576 and p['target_code_pattern'] == 'push-tag-stop-jumpdest', 'code metadata'
    assert p['target_code_sha256'] == [hashlib.sha256(bytes.fromhex(c['code_hex'][2:])).hexdigest() for c in contracts], 'code SHA256 intent'
    code = bytes.fromhex(program['code_hex'][2:])
    assert p['runtime_bytes'] == len(code) and p['runtime_sha256'] == hashlib.sha256(code).hexdigest(), 'runtime commitment'
    assert 0 < tx['gas_limit'] <= min(15_000_000, 16_777_216), 'transaction caps'
    assert 31_000_000 - 167355 >= tx['gas_limit'], 'header prefix cap'
    result = execute(code, tx['gas_limit'], operation=p['operation'], footprint=footprint, visits=visits, contracts=contracts)
    assert 167355 + result['gas'] <= 30_000_000, 'block charged cap'
    return result
