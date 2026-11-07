"""Concrete byte oracle for Address20 probes; no producer/runtime/gas helper imports.

This is separately implemented arithmetic, not independent authorship or authenticated
state evidence. Native witness authentication remains a distinct qualification gate.
"""
from collections import Counter
import hashlib
import json

QUERY = int('1122334400000000000000000000000055667788', 16)
SENTINEL = 0xa55a
MASK = (1 << 256) - 1
PRIMARY = '0x' + '66' * 20
RESERVED = {int(x, 16) for x in [PRIMARY, '0x' + '88' * 20, '0x' + '99' * 20,
    '0x' + '33' * 20, '0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a',
    '0x1670000000000000000000000000000000010001',
    '0x0000777735367b36bc9b61c50022d9d0700db4ec']}


def decode(code):
    assert type(code) is bytes, 'code domain'
    pc = 0
    while pc < len(code):
        at = pc; op = code[pc]; pc += 1
        size = op - 0x5f if 0x60 <= op <= 0x7f else 0
        assert pc + size <= len(code), 'truncated PUSH'
        value = int.from_bytes(code[pc:pc + size], 'big') if size else 0
        yield at, op, value
        pc += size


def structure(instructions, operation, repetitions, *, key_count=512, query=QUERY):
    opcode = {'balance': 0x31, 'extcodesize': 0x3b}[operation]; index = 0; failures = []
    def take(expected, value=0):
        nonlocal index
        assert index < len(instructions), 'truncated byte shape'
        at, actual, immediate = instructions[index]; index += 1
        assert actual == expected and (value is None or value == immediate), 'byte intent/guard shape'
        return at, immediate
    def push(value, width=3): return take(0x5f + width, value)
    def load(offset): push(offset); return take(0x51)[0]
    def guard(value):
        push(value, 32); take(0x14); take(0x15); failures.append(take(0x61, None)[1]); take(0x57)
    take(0x36); guard(32 * key_count)
    for offset, value in [(0, 0), (64, 0), (96, SENTINEL)]: push(value); push(offset); take(0x52)
    seed, _ = take(0x5b); load(0); push(key_count); take(0x14); _, seed_target = take(0x61, None); take(0x57)
    load(0); push(5); take(0x1b); seed_load = take(0x35)[0]; seed_read = take(opcode)[0]; guard(0)
    load(0); push(1); take(1); push(0); take(0x52); take(0x61, seed); take(0x56)
    seed_done, _ = take(0x5b); assert seed_done == seed_target, 'seed branch'
    push(32 * (key_count - 1)); query_load = take(0x35)[0]; take(0x80); warmup = take(opcode)[0]; guard(0)
    loop, _ = take(0x5b); load(64); push(repetitions, 32); take(0x14); _, done_target = take(0x61, None); take(0x57)
    body = instructions[index][0]; sites = []
    for _ in range(64): take(0x80); sites.append(take(opcode)[0]); take(0x50)
    load(64); push(1); take(1); push(64); take(0x52); take(0x61, loop); take(0x56)
    done, _ = take(0x5b); assert done_target == done, 'done branch'
    take(0x80); guard(query); take(0x50)
    for offset, value in [(0, key_count), (64, repetitions), (96, SENTINEL)]: load(offset); guard(value)
    take(0); failure, _ = take(0x5b); take(0xfe)
    assert index == len(instructions) and failures == [failure] * 7, 'failure targets/extra bytes'
    return dict(seed=seed, seed_load=seed_load, seed_read=seed_read, query_load=query_load,
        warmup=warmup, loop=loop, body=body, body_sites=frozenset(sites))


def addresses(data, *, key_count=512, query=QUERY, strict_keys=True):
    assert type(key_count) is int and 1 <= key_count <= 1024, 'key count domain'
    assert type(query) is int and 0 <= query < 1 << 160 and type(strict_keys) is bool, 'query/strict domain'
    assert not strict_keys or (key_count == 512 and query == QUERY), 'legacy catalog parameters'
    assert type(data) is bytes and len(data) == 32 * key_count, 'calldata domain'
    result = []
    for offset in range(0, len(data), 32):
        word = data[offset:offset + 32]
        assert word[:12] == bytes(12), 'ABI address left padding'
        value = int.from_bytes(word, 'big')
        assert (not strict_keys or sum(b != 0 for b in word[12:]) == 8) and value > 0xffff and value not in RESERVED, 'address byte cost/reserved domain'
        result.append(value)
    assert len(set(result)) == key_count and result[-1] == query, 'unique addresses/query last'
    return result


def execute(code, data, envelope, *, operation, repetitions, account_values=None,
            key_count=512, query=QUERY, access_list=False, strict_keys=True):
    assert type(operation) is str and operation in ('balance', 'extcodesize'), 'operation domain'
    assert type(repetitions) is int and 0 <= repetitions <= 1000000, 'R domain'
    assert type(access_list) is bool and (not strict_keys or not access_list), 'access-list mode'
    keys = addresses(data, key_count=key_count, query=query, strict_keys=strict_keys)
    intrinsic = 21000 + data.count(0) * 4 + (len(data) - data.count(0)) * 16 + (2400 * key_count if access_list else 0)
    assert type(code) is bytes and 0 < len(code) <= 24576 and type(envelope) is int and envelope >= intrinsic, 'code/gas domain'
    values = {} if account_values is None else account_values
    assert type(values) is dict and all(type(k) is int and type(v) is int and 0 <= v <= MASK for k, v in values.items()), 'account value domain'
    instructions = list(decode(code)); shape = structure(instructions, operation, repetitions, key_count=key_count, query=query)
    by_pc = {at: (op, immediate) for at, op, immediate in instructions}; jumps = {at for at, op, _ in instructions if op == 0x5b}
    opcode = 0x31 if operation == 'balance' else 0x3b
    stack = []; memory = bytearray(); warm = set(keys) if access_list else set(); counts = Counter(); pc_counts = Counter(); pc = 0
    gas = intrinsic
    cold = warm_reads = expansion = iterations = visits = 0; seed = []
    def word(offset): return int.from_bytes(memory[offset:offset + 32], 'big')
    for _ in range(2000000):
        assert pc in by_pc, 'invalid PC'
        if pc == shape['seed']:
            assert stack == [] and word(0) == len(seed), 'seed counter/stack'
        if pc == shape['loop']:
            assert stack == [query] and word(0) == key_count and word(64) == visits and word(96) == SENTINEL, 'loop guard'
            assert warm == set(keys), 'complete warmset'
            visits += 1
        if pc == shape['body']:
            assert word(64) == iterations < repetitions, 'body counter'
            iterations += 1
        at = pc; op, immediate = by_pc[pc]
        pc += 1 + (op - 0x5f if 0x60 <= op <= 0x7f else 0)
        counts[f'opcode:0x{op:02x}'] += 1; pc_counts[at] += 1
        if 0x60 <= op <= 0x7f: stack.append(immediate); gas += 3
        elif op == 0x80: stack.append(stack[-1]); gas += 3
        elif op == 0x50: stack.pop(); gas += 2
        elif op in (1, 0x14, 0x1b):
            a, b = stack.pop(), stack.pop()
            stack.append((a + b) & MASK if op == 1 else int(a == b) if op == 0x14 else (b << a) & MASK); gas += 3
        elif op == 0x15: stack.append(int(stack.pop() == 0)); gas += 3
        elif op == 0x36: stack.append(len(data)); gas += 2
        elif op == 0x35:
            offset = stack.pop(); expected = 32 * len(seed) if at == shape['seed_load'] else 32 * (key_count - 1)
            assert offset == expected and offset <= 32 * (key_count - 1), 'calldata offset/order'
            stack.append(int.from_bytes(data[offset:offset + 32], 'big')); gas += 3
        elif op in (0x51, 0x52):
            offset = stack.pop(); assert offset in (0, 64, 96), 'memory domain'
            old = len(memory) // 32; new = max(old, (offset + 63) // 32)
            delta = 3 * new + new * new // 512 - 3 * old - old * old // 512
            expansion += delta; gas += 3 + delta; memory.extend(bytes(32 * (new - old)))
            if op == 0x51: stack.append(word(offset))
            else: memory[offset:offset + 32] = stack.pop().to_bytes(32, 'big')
        elif op == opcode:
            key = stack.pop(); assert 0 <= key < 1 << 160, 'actual Address20 operand'
            if at == shape['seed_read']:
                assert len(seed) < key_count and key == keys[len(seed)] and (key in warm) == access_list, 'seed order/first touch'
                seed.append(key)
                if access_list: warm_reads += 1; gas += 100
                else: cold += 1; gas += 2600
            else:
                assert at == shape['warmup'] or at in shape['body_sites'], 'unexpected account read'
                assert key == query and key in warm and len(warm) == key_count, 'warm query intent'
                warm_reads += 1; gas += 100
            warm.add(key); stack.append(values.get(key, 0))
        elif op in (0x56, 0x57):
            target = stack.pop(); taken = op == 0x56 or stack.pop() != 0; gas += 8 if op == 0x56 else 10
            if taken: assert target in jumps, 'jump target'; pc = target
        elif op == 0x5b: gas += 1
        elif op in (0, 0xfe):
            assert op == 0 and stack == [], 'guard failure/terminal stack'
            assert (word(0), word(64), word(96), len(memory)) == (key_count, repetitions, SENTINEL, 128), 'final counters/memory'
            assert iterations == repetitions and visits == repetitions + 1, 'loop cardinality'
            assert cold == (0 if access_list else key_count) and warm_reads == 64 * repetitions + 1 + (key_count if access_list else 0) and seed == keys, 'account footprint'
            floor = 21000 + (data.count(0) + 4 * (len(data) - data.count(0))) * 10
            intent = dict(addresses=[f'0x{k:040x}' for k in seed], query=f'0x{query:040x}', pc_counts=dict(sorted(pc_counts.items())))
            return dict(gas=max(gas, floor), gross_gas=gas, eip7623_floor_gas=floor, counts=counts,
                stack=stack, counter=repetitions, peak_memory_bytes=len(memory), memory_expansion_gas=expansion,
                seed_addresses=[f'0x{k:040x}' for k in seed], cold_reads=cold, warm_reads=warm_reads,
                read_events=cold + warm_reads, terminal='STOP', account_writes=0, persistent_storage_reads=0,
                read_intent_sha256=hashlib.sha256(json.dumps(intent, sort_keys=True, separators=(',', ':')).encode()).hexdigest())
        else: raise AssertionError(f'unsupported opcode {op:02x}')
        assert len(stack) <= 1024 and gas <= envelope, 'stack bound/out of gas'
    raise AssertionError('instruction bound')


def run_users(row):
    p = row['parameters']; request = row['request']; program = request['program']; tx = program['transactions']
    sized = 'sized_account_catalog' in p
    if sized:
        assert p['sized_account_catalog'] is True and type(p['key_count']) is int and 1 <= p['key_count'] <= 1024, 'sized metadata domain'
        assert type(p['access_list']) is bool, 'sized access-list boolean'
        n = p['key_count']; use_al = p['access_list']
        assert all(type(p[name]) is int for name in ('transaction_gas_limit', 'header_gas_limit',
            'charged_gas_budget', 'unroll_factor', 'bulk_operations', 'all_user_transactions',
            'all_transactions_including_anchor', 'block_count')), 'sized schedule types'
        assert (p['all_user_transactions'], p['all_transactions_including_anchor'], p['block_count']) == (1, 2, 1), 'sized one-user metadata'
        assert p['header_gas_limit'] == 31000000 and p['charged_gas_budget'] == 30000000, 'sized header/budget metadata'
        assert p['header_prefix_remaining_gas'] == [30832645] and type(p['header_prefix_remaining_gas'][0]) is int, 'sized header prefix'
        assert p['unroll_factor'] == 64 and p['bulk_operations'] == 64 * p['repetitions'], 'sized loop metadata'
        assert (p['seed_counter_memory_offset'], p['counter_memory_offset'], p['protected_memory_offset']) == (0, 64, 96), 'sized memory metadata'
        assert type(p['query_address']) is str and len(p['query_address']) == 42 and p['query_address'].startswith('0x') and all(x in '0123456789abcdef' for x in p['query_address'][2:]), 'query address domain'
        query = int(p['query_address'], 16)
    else: n = 512; use_al = False; query = QUERY
    assert set(request) == {'schema_version', 'expected_user_success', 'program'} and type(request['schema_version']) is int and request['schema_version'] == 1 and request['expected_user_success'] is True, 'request domain'
    assert set(program) == {'code_hex', 'input_hex', 'block_count', 'expected_route_counts', 'transactions'} | ({'access_list'} if use_al else set()), 'absent-account/no-extra-prestate domain'
    assert tx == dict(kind='call', count_per_block=1, gas_limit=p['transaction_gas_limit']) and type(tx['count_per_block']) is int and type(program['block_count']) is int and program['block_count'] == 1, 'one-user schedule'
    assert p['primary_code_owner'] == PRIMARY and p['transaction_nonce'] == 0 and p['recipient_schedule'] == [PRIMARY], 'primary/nonce'
    assert type(tx['gas_limit']) is int and 21000 <= tx['gas_limit'] <= 15000000 and tx['gas_limit'] <= 16777216, 'transaction caps'
    if sized: assert tx['gas_limit'] == 15000000, 'sized fixed15M envelope'
    data = bytes.fromhex(program['input_hex'][2:]); code = bytes.fromhex(program['code_hex'][2:]); keys = addresses(data, key_count=n, query=query, strict_keys=not sized)
    assert [f'0x{k:040x}' for k in keys] == p['ordered_addresses'], 'original catalog order'
    if use_al:
        al = program['access_list']
        assert al == dict(pattern='raw_addresses', count=n, addresses=p['ordered_addresses']) and type(al['count']) is int, 'raw access-list order/count/empty-keys'
    assert hashlib.sha256(data).hexdigest() == p['calldata_sha256'] == p['address_words_sha256'], 'calldata hash'
    assert hashlib.sha256(code).hexdigest() == p['runtime_sha256'] and len(code) == p['runtime_bytes'], 'runtime hash/length'
    assert p['query_address'] == f'0x{query:040x}' and p['key_kind'] == 'address20', 'query/account domain'
    assert p['target_balance'] == p['target_code_bytes'] == p['expected_persistent_storage_reads'] == p['expected_persistent_storage_writes'] == 0, 'absent target/zero storage metadata'
    assert type(p['repetitions']) is int and 0 <= p['repetitions'] <= 1000000, 'R metadata domain'
    assert all(type(p[name]) is int for name in ('accessed_unique_addresses', 'seed_reads', 'query_reads', 'read_events')), 'account counter types'
    assert (p['accessed_unique_addresses'], p['seed_reads'], p['query_reads'], p['read_events']) == (n, n, 64 * p['repetitions'] + 1, 64 * p['repetitions'] + n + 1), 'declared account cardinality'
    assert (p['outer_calldata_bytes'], p['calldata_zero_bytes'], p['calldata_nonzero_bytes']) == (len(data), data.count(0), len(data) - data.count(0)), 'declared calldata dimensions'
    assert all(type(v) is int and v >= 0 for v in program['expected_route_counts'].values()), 'opcode count domain'
    result = execute(code, data, tx['gas_limit'], operation=p['operation'], repetitions=p['repetitions'], key_count=n, query=query, access_list=use_al, strict_keys=not sized)
    assert result['counts'] == +Counter(program['expected_route_counts']), 'declared opcode counts'
    assert result['gas'] == p['expected_user_charged_gas'] and result['gross_gas'] == p['expected_user_gross_gas'], 'declared gas'
    assert p['expected_block_charged_gas'] == result['gas'] + 167355 and p['expected_anchor_gas'] == 167355, 'declared charged block gas'
    assert p['peak_memory_bytes'] == result['peak_memory_bytes'] and p['sentinel'] == SENTINEL and p['expected_counter'] == p['repetitions'], 'declared memory guards'
    intrinsic = 21000 + data.count(0) * 4 + (len(data) - data.count(0)) * 16 + (2400 * n if use_al else 0)
    accounting = p['gas_accounting_per_user']
    fields = [('intrinsic_gas', intrinsic), ('cold_account_gas', 2600 * result['cold_reads']),
        ('warm_account_gas', 100 * result['warm_reads']), ('gross_gas', result['gross_gas']),
        ('charged_gas', result['gas']), ('eip7623_floor_gas', result['eip7623_floor_gas']),
        ('memory_expansion_gas', result['memory_expansion_gas']), ('refund_counter', 0), ('refund_applied', 0)]
    if sized:
        ordinary = result['gross_gas'] - intrinsic - result['memory_expansion_gas'] - 2600 * result['cold_reads'] - 100 * result['warm_reads']
        fields += [('access_list_intrinsic_gas', 2400 * n if use_al else 0), ('cold_account_count', result['cold_reads']),
            ('warm_account_count', result['warm_reads']), ('ordinary_opcode_gas', ordinary), ('runtime_bytes', len(code))]
        assert accounting['eip7623_floor_active'] is (result['gross_gas'] < result['eip7623_floor_gas']), 'floor activation'
    for name, value in fields:
        assert type(accounting[name]) is int and accounting[name] == value, 'declared gas component ' + name
    assert 31000000 - 167355 >= tx['gas_limit'] and 167355 + result['gas'] <= 30000000, 'header/block caps'
    return [result]
