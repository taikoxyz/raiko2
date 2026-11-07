"""Independent decoded-byte calldata/storage oracle; no producer imports.

The optional pure-read body cache proves all64 decoded read sites and the complete
stack/warmset/zero-value invariant first. Live preconditions are checked on every
use. Seed accesses, counters, guards and outer flow always execute directly.
Actual EVM/SP1 execution performs every SLOAD; this cache is oracle-only.
"""
from collections import Counter
import hashlib
import json

QUERY = 0x1122334455667788 << 192
SENTINEL = 0xa55a
MASK = (1 << 256) - 1
PRIMARY = '0x' + '66' * 20


def decode(code):
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1; width = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + width <= len(code), 'truncated PUSH'
        yield at, op, int.from_bytes(code[pc:pc + width], 'big'); pc += width


def structure(instructions, repetitions, *, key_count=512, query=QUERY):
    """Authenticate complete byte intent, including guards and the R0 body."""
    index = 0; failures = []
    def take(opcode, value=0):
        nonlocal index
        assert index < len(instructions), 'missing instruction'
        at, op, immediate = instructions[index]; index += 1
        assert op == opcode and (value is None or immediate == value), 'byte intent/guard shape'
        return at, immediate
    def push(value, width=3): return take(0x5f + width, value)
    def load(offset): push(offset); return take(0x51)[0]
    def guard(value):
        push(value, 32); take(0x14); take(0x15); failures.append(take(0x61, None)[1]); take(0x57)
    take(0x36); guard(32 * key_count)
    for offset, value in ((0, 0), (64, 0), (96, SENTINEL)): push(value); push(offset); take(0x52)
    seed, _ = take(0x5b); load(0); push(key_count); take(0x14); _, seed_done_target = take(0x61, None); take(0x57)
    load(0); push(5); take(0x1b); seed_load = take(0x35)[0]; seed_read = take(0x54)[0]; guard(0)
    load(0); push(1); take(1); push(0); take(0x52); take(0x61, seed); take(0x56)
    seed_done, _ = take(0x5b); assert seed_done == seed_done_target, 'seed target'
    push(32 * (key_count - 1)); query_load = take(0x35)[0]; take(0x80); warmup = take(0x54)[0]; guard(0)
    loop, _ = take(0x5b); load(64); push(repetitions, 32); take(0x14); _, done_target = take(0x61, None); take(0x57)
    body = instructions[index][0]; body_pcs = []
    for _ in range(64): take(0x80); body_pcs.append(take(0x54)[0]); take(0x50)
    body_end = instructions[index][0]
    load(64); push(1); take(1); push(64); take(0x52); take(0x61, loop); take(0x56)
    done, _ = take(0x5b); assert done_target == done, 'done target'
    take(0x80); guard(query); take(0x50)
    for offset, value in ((0, key_count), (64, repetitions), (96, SENTINEL)): load(offset); guard(value)
    take(0); failure, _ = take(0x5b); take(0xfe)
    assert index == len(instructions) and failures == [failure] * 7, 'failure targets/extra bytes'
    return dict(seed=seed, seed_load=seed_load, seed_read=seed_read, query_load=query_load,
        warmup=warmup, loop=loop, body=body, body_end=body_end, body_pcs=body_pcs)


def _entry(stack, warm, storage, query, expected_warm, *, key_count=512):
    assert type(query) is int and 0 <= query <= MASK and list(stack) == [query], 'complete query stack precondition'
    assert frozenset(warm) == expected_warm and query in warm and len(warm) == key_count, 'warmset precondition'
    assert all(value == 0 for value in storage.values()) and storage.get(query, 0) == 0, 'zero storage feedback/precondition'


def prove_body(instructions, entry_stack, warm, storage, query, *, key_count=512):
    expected_warm = frozenset(warm); _entry(entry_stack, warm, storage, query, expected_warm, key_count=key_count)
    assert len(instructions) == 192, 'body exact64-site length'
    stack = list(entry_stack); counts = Counter(); sites = []; gas = 0
    for index, (at, op, immediate) in enumerate(instructions):
        assert immediate == 0 and op == (0x80, 0x54, 0x50)[index % 3], 'decoded body read intent'
        counts[f'opcode:0x{op:02x}'] += 1
        if op == 0x80: stack.append(stack[-1]); gas += 3
        elif op == 0x54:
            key = stack.pop(); assert key == query and key in warm, 'body key/warmness'
            value = storage.get(key, 0); assert value == 0, 'body zero value'; stack.append(value); gas += 100; sites.append(at)
        else: assert stack.pop() == 0, 'body discarded value'; gas += 2
    assert stack == list(entry_stack) == [query] and len(sites) == 64, 'complete pure-read transition'
    return dict(key_count=key_count, query=query, warm=expected_warm, counts=counts, gas=gas, sites=tuple(sites), reads=64, writes=0)


def apply_body(proof, stack, warm, storage):
    _entry(stack, warm, storage, proof['query'], proof['warm'], key_count=proof['key_count'])
    # Proven effect:64 reads of a warm zero slot, identical complete stack and
    # warmset, no writes and no memory footprint. Nothing mutable is skipped.


def _keys(data, *, key_count=512, query=QUERY, structured=False):
    assert type(structured) is bool and type(key_count) is int and 1 <= key_count <= (4096 if structured else 1024), 'key_count domain'
    assert type(data) is bytes and len(data) == 32 * key_count, 'calldata length'
    keys = [int.from_bytes(data[i:i + 32], 'big') for i in range(0, 32 * key_count, 32)]
    assert len(set(keys)) == key_count and keys[-1] == query, 'unique catalog/query last'
    if not structured:
        assert query == QUERY and all(sum(byte != 0 for byte in k.to_bytes(32, 'big')) == 8 for k in keys), 'fixed calldata byte cost'
    return keys


def execute(code, data, envelope, *, repetitions, cache=True, key_count=512, query=QUERY, structured=False):
    assert type(structured) is bool and type(key_count) is int and 1 <= key_count <= (4096 if structured else 1024), 'key_count domain'
    assert type(repetitions) is int and 0 <= repetitions <= 1000000 and type(cache) is bool, 'R/cache domain'
    assert type(query) is int and 0 <= query <= MASK and (structured or query == QUERY), 'query domain'
    assert type(data) is bytes
    assert type(code) is bytes and len(code) < 24576 and type(envelope) is int and envelope >= 21000 + 4*data.count(0) + 16*(len(data)-data.count(0)), 'code/gas domain'
    keys = _keys(data, key_count=key_count, query=query, structured=structured); instructions = list(decode(code)); shape = structure(instructions, repetitions, key_count=key_count, query=query)
    by_pc = {at: (op, value) for at, op, value in instructions}; jumps = {at for at, op, _ in instructions if op == 0x5b}
    proof = prove_body([i for i in instructions if shape['body'] <= i[0] < shape['body_end']], [query], set(keys), {}, query, key_count=key_count)
    stack = []; memory = bytearray(); storage = {}; warm = set(); counts = Counter(); pc_counts = Counter()
    gas = 21000 + data.count(0) * 4 + (len(data) - data.count(0)) * 16; pc = 0; expansion = 0
    cold = 0; warm_reads = 0; seed_keys = []; iterations = 0; loop_visits = 0
    def word(offset): return int.from_bytes(memory[offset:offset + 32], 'big')
    for _ in range(2000000):
        assert pc in by_pc, 'invalid PC'
        if pc == shape['seed']:
            assert stack == [] and word(0) == len(seed_keys), 'seed counter/stack'
        if pc == shape['loop']:
            assert stack == [query] and word(0) == key_count and word(64) == loop_visits and word(96) == SENTINEL, 'loop counter/sentinel/stack'
            loop_visits += 1
        if pc == shape['body']:
            assert word(64) == iterations < repetitions, 'body counter'
            _entry(stack, warm, storage, query, proof['warm'], key_count=key_count); iterations += 1
            if cache:
                apply_body(proof, stack, warm, storage); gas += proof['gas']; counts.update(proof['counts']); warm_reads += 64
                pc_counts.update({at: 1 for at, _, _ in instructions if shape['body'] <= at < shape['body_end']})
                pc = shape['body_end']; assert gas <= envelope, 'out of gas'; continue
        at = pc; op, immediate = by_pc[pc]; pc += 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0)
        counts[f'opcode:0x{op:02x}'] += 1; pc_counts[at] += 1
        if 0x60 <= op <= 0x7f: stack.append(immediate); gas += 3
        elif op == 0x80: stack.append(stack[-1]); gas += 3
        elif op == 0x50: stack.pop(); gas += 2
        elif op in (1, 0x14, 0x1b):
            x, y = stack.pop(), stack.pop(); stack.append((x + y) & MASK if op == 1 else int(x == y) if op == 0x14 else (y << x) & MASK); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op == 0x36: stack.append(len(data)); gas += 2
        elif op == 0x35:
            offset = stack.pop(); expected = 32 * len(seed_keys) if at == shape['seed_load'] else 32 * (key_count - 1)
            assert offset == expected and offset <= 32 * (key_count - 1), 'calldata offset/order'
            stack.append(int.from_bytes(data[offset:offset + 32], 'big')); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); assert offset in (0, 64, 96), 'memory domain'
            old = len(memory) // 32; new = max(old, (offset + 63) // 32)
            delta = 3 * new + new * new // 512 - 3 * old - old * old // 512
            expansion += delta; gas += 3 + delta; memory.extend(bytes(32 * (new - old)))
            if op == 0x51: stack.append(word(offset))
            else: memory[offset:offset + 32] = stack.pop().to_bytes(32, 'big')
        elif op == 0x54:
            key = stack.pop(); value = storage.get(key, 0); assert value == 0, 'read zero invariant'
            if at == shape['seed_read']:
                assert len(seed_keys) < key_count and key == keys[len(seed_keys)] and key not in warm, 'seed order/unique first touch'
                seed_keys.append(key); cold += 1; gas += 2100
            else:
                assert key == query and key in warm and len(warm) == key_count, 'query warm read'
                warm_reads += 1; gas += 100
            warm.add(key); stack.append(value)
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0; gas += 8 if op == 0x56 else 10
            if taken: assert target in jumps, 'jump target'; pc = target
        elif op == 0x5b: gas += 1
        elif op in (0, 0xfe):
            assert op == 0 and stack == [], 'guard failure/terminal stack'
            assert (word(0), word(64), word(96), len(memory)) == (key_count, repetitions, SENTINEL, 128), 'final counters/sentinel/memory'
            assert iterations == repetitions and loop_visits == repetitions + 1, 'outer loop visits'
            assert cold == key_count and warm_reads == 64 * repetitions + 1 and seed_keys == keys and storage == {}, 'storage/read footprint'
            grouped = {str(k): v for k, v in sorted(pc_counts.items())}
            commitment = hashlib.sha256(json.dumps(dict(keys=[f'0x{k:064x}' for k in seed_keys], query=f'0x{query:064x}', pc_counts=grouped), sort_keys=True, separators=(',', ':')).encode()).hexdigest()
            return dict(terminal=op, stack=stack, counter=repetitions, gas=gas, counts=counts, cold_reads=cold, warm_reads=warm_reads,
                seed_keys=[f'0x{k:064x}' for k in seed_keys], read_events=cold + warm_reads, peak_memory_bytes=len(memory),
                memory_expansion_gas=expansion, grouped_pc_counts=grouped, read_intent_sha256=commitment)
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024 and gas <= envelope, 'stack bound/out of gas'
    raise AssertionError('instruction bound')


def run_users(row, *, structured=False):
    p = row['parameters']; request = row['request']; program = request['program']; tx = program['transactions']
    key_count = p.get('key_count', 512)
    users = p['all_user_transactions'] if structured else 1
    assert type(users) is int and users in (1, 2), 'structured user count domain'
    assert type(structured) is bool and type(key_count) is int and 1 <= key_count <= (4096 if structured else 1024), 'key_count metadata domain'
    assert set(request) == {'schema_version', 'expected_user_success', 'program'} and request['schema_version'] == 1 and request['expected_user_success'] is True, 'request domain'
    assert set(program) == {'code_hex', 'input_hex', 'block_count', 'expected_route_counts', 'transactions'}, 'primary empty storage/domain'
    expected_tx = dict(kind='call', counts_by_block=[0, users], gas_limit=p['transaction_gas_limit']) if structured else dict(kind='call', count_per_block=1, gas_limit=p['transaction_gas_limit'])
    assert tx == expected_tx and program['block_count'] == (2 if structured else 1), 'user schedule'
    assert p['primary_code_owner'] == PRIMARY and type(p['transaction_nonce']) is int and p['transaction_nonce'] == 0 and p['recipient_schedule'] == [PRIMARY]*users, 'primary/nonce'
    assert 0 < tx['gas_limit'] <= 15000000 and tx['gas_limit'] <= 16777216, 'transaction caps'
    data = bytes.fromhex(program['input_hex'][2:]); code = bytes.fromhex(program['code_hex'][2:])
    query = int.from_bytes(data[-32:], 'big') if structured else QUERY
    keys = _keys(data, key_count=key_count, query=query, structured=structured)
    if structured:
        layout = p['raw_key_layout']
        assert type(layout) is dict and set(layout) == {'L1', 'L3'} and type(layout['L1']) is int and 0 <= layout['L1'] < 1 << 64 and type(layout['L3']) is int and layout['L3'] == 0, 'structured layout'
        assert p['ordinal_shift'] == 24 and type(p['ordinal_shift']) is int and keys == [(i << 24) | (layout['L1'] << 64) for i in range(1, key_count + 1)], 'affine ordered calldata'
        assert p['mode'] == 'structured-warm-tail', 'explicit structured mode'
        assert all(type(x) is int for x in tx['counts_by_block']), 'typed block schedule'
        assert p['execution_profile'] == 'evm-gas-only' and p['preload_created_accounts_or_code'] is False, 'execution scope'
        assert p['expected_terminal'] == 'STOP' and p['expected_user_success'] is True, 'terminal metadata'
    assert [f'0x{k:064x}' for k in keys] == p['ordered_keys'], 'original catalog/order intent'
    assert hashlib.sha256(data).hexdigest() == p['calldata_sha256'] == p['raw_key_sha256'], 'catalog hashes'
    assert hashlib.sha256(code).hexdigest() == p['runtime_sha256'], 'runtime hash'
    assert p['query_key'] == f'0x{query:064x}' and p['initial_storage_entries'] == 0, 'query/zero prestate'
    results = []; routes = Counter()
    for _ in range(users):
        result = execute(code, data, tx['gas_limit'], repetitions=p['repetitions'], key_count=key_count, query=query, structured=structured)
        assert (result['cold_reads'], result['warm_reads'], result['read_events']) == (key_count, 64*p['repetitions']+1, key_count+64*p['repetitions']+1), 'per-transaction cold/warm intent'
        if not structured:
            assert 31000000 - 167355 >= tx['gas_limit'] and 167355 + result['gas'] <= 30000000, 'header/block caps'
        r = p['repetitions']; z = data.count(0); nz = len(data) - z
        intrinsic = 21000 + 4*z + 16*nz; floor = 21000 + 10*(z + 4*nz)
        cold = 2100 * key_count; warm = 100 * (64 * r + 1)
        gross = result['gas']; charged = max(gross, floor)
        expected = dict(seed_reads=key_count, accessed_unique=key_count, accessed_unique_keys=key_count,
            expected_persistent_storage_reads=key_count + 1 + 64*r, read_events=key_count + 1 + 64*r,
            expected_persistent_storage_writes=0, initial_storage_entries=0, initial_state_size=0,
            aggregate_state_size=0, state_size_per_account=0, expected_initial_storage_entry_count=0,
            expected_final_storage_entry_count=0, seed_counter_memory_offset=0, counter_memory_offset=64,
            protected_memory_offset=96, sentinel=SENTINEL, peak_memory_bytes=128, memory_words=4, runtime_bytes=len(code),
            outer_calldata_bytes=32*key_count,
            calldata_zero_bytes=z, calldata_nonzero_bytes=nz,
            target_query_reads=64*r+1, target_operations_per_user=64*r+1,
            warmup_target_operations_per_user=1, body_target_operations_per_user=64*r,
            bulk_operations=64*r, unroll_factor=64, expected_counter=r,
            expected_user_gross_gas=gross, expected_user_charged_gas=charged,
            expected_user_refund_counter=0, expected_user_refund_applied=0)
        if structured:
            assert 'expected_anchor_gas' not in p and 'expected_block_charged_gas' not in p, 'no fake actual Anchor gas'
            assert type(p['anchor_gas_limit_upper_bound']) is int and p['anchor_gas_limit_upper_bound'] == 1000000
            assert p['user_transactions_by_block'] == [0, users] and all(type(x) is int for x in p['user_transactions_by_block'])
            remaining = [30000000-before*charged for before in range(users)]
            assert p['block_charged_gas_upper_bounds'] == [1000000, 1000000+users*charged] and p['header_prefix_remaining_gas'] == remaining, 'source per-block bounds'
            assert all(prefix >= tx['gas_limit'] for prefix in remaining), 'each header admission prefix'
            assert p['expected_user_receipt_statuses'] == [True]*users and all(v is True for v in p['expected_user_receipt_statuses']), 'receipt status intent'
            expected.update(block_count=2, all_user_transactions=users, all_transactions_including_anchor=users+2, visits_per_account=users,
                header_gas_limit=31000000, charged_gas_budget=30000000,
                source_proposal_charged_gas_upper_bound=2000000+users*charged, source_proposal_envelope_upper_bound=2000000+users*(gross+1000))
            assert tx['gas_limit'] == gross + 1000 and 2000000+users*tx['gas_limit'] <= 30000000 and 2000000+users*charged <= 30000000, 'source proposal envelope/budget'
            assert len(data) <= 131072
        else:
            expected['expected_block_charged_gas'] = 167355+charged
        for name, value in expected.items():
            assert type(p[name]) is int and p[name] == value, 'count/gas metadata: ' + name
        gas = dict(intrinsic_gas=intrinsic, eip7623_floor_gas=floor, eip7623_floor_active=gross < floor,
            ordinary_opcode_gas=gross-intrinsic-12-cold-warm, memory_expansion_gas=12,
            cold_sload_gas=cold, warm_sload_gas=warm, cold_sload_count=key_count,
            warm_sload_count=64*r+1, body_gas_per_repetition=64*105, gross_gas=gross,
            charged_gas=charged, refund_counter=0, refund_applied=0, runtime_bytes=len(code))
        actual_gas = p['gas_accounting_per_user']
        assert set(actual_gas) == set(gas), 'gas accounting field domain'
        for name, value in gas.items():
            assert type(actual_gas[name]) is type(value) and actual_gas[name] == value, 'literal gas accounting metadata: ' + name
        results.append(result); routes.update(result['counts'])
    assert routes == +Counter(program['expected_route_counts']), 'literal aggregate opcode route counts'
    return results
