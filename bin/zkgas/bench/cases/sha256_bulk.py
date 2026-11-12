#!/usr/bin/env python3
"""Generate 18 compiler-free, guarded unique-input SHA-256 bulk requests."""
import argparse
import hashlib
import inspect
import json
from pathlib import Path
import re

import initcode_preparation
from initcode_preparation import _Factory
import spawn_memory
from spawn_memory import digest, encoded, write
import storage_fanout
from storage_fanout import BASE_ADDRESS, MODEL
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-sha256-bulk-20261104'
PATTERNS = ('zero', 'a5')
LENGTHS = (64, 4096, 1048576)
LINEAR_LENGTHS = (1269664, 1269665, 2097152)
LINEAR_CORPUS = 'unzen-sha256-linear-regime-20261104'
LEVELS = ('zero', 'single', 'full')
MAX_REPETITIONS = 1_000_000
USERS = 2
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
OWNERS = tuple(f'0x{BASE_ADDRESS + tag:040x}' for tag in range(USERS))


def _validate(pattern, length):
    if type(pattern) is not str or pattern not in PATTERNS: raise ValueError('pattern must be zero or a5')
    if type(length) is not int or length not in LENGTHS + LINEAR_LENGTHS: raise ValueError('input length must be64,4096,1048576,1269664,1269665 or2097152')


def expected_xor(pattern, length, repetitions, tag=0):
    """Compute only real emitted guards, never gas-search candidates."""
    _validate(pattern, length); original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    original.integer(tag, 0, USERS - 1, 'owner tag')
    tail = bytes([0 if pattern == 'zero' else 0xa5]) * (length - 32); value = 0
    for index in range(repetitions):
        value ^= int.from_bytes(hashlib.sha256(((tag << 128) | index).to_bytes(32, 'big') + tail).digest(), 'big')
    return value


def _assemble(pattern, length, repetitions, tag, xor):
    """Fixed-width assembly; xor=0 is exclusively an internal gas-sizing input."""
    _validate(pattern, length); original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    original.integer(tag, 0, USERS - 1, 'owner tag'); original.integer(xor, 0, (1 << 256) - 1, 'expected XOR')
    asm = _Factory(); output = 64 + length

    def store(offset, value, times=1, width=3):
        asm.push(value, width, times); asm.push(offset, times=times); asm.op(0x52, times)

    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)

    # Preallocate full input/output once. Even the zero control seeds and copies.
    store(output, 2); store(0, 0); store(32, 0)
    store(64, int.from_bytes(bytes([0 if pattern == 'zero' else 0xa5]) * 32, 'big'), width=32)
    filled = 32
    while filled < length:
        size = min(filled, length - filled)
        asm.push(size); asm.push(64); asm.push(64 + filled); asm.op(0x5e)
        filled += size
    visits = repetitions + 1; asm.label('loop', visits)
    load(0, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits)
    asm.target('done', visits); asm.op(0x57, visits)
    load(0, repetitions); asm.push(tag << 128, 32, repetitions); asm.op(0x17, repetitions)
    asm.push(64, times=repetitions); asm.op(0x52, repetitions)
    store(output, 2, repetitions)
    asm.push(32, times=repetitions); asm.push(output, times=repetitions)
    asm.push(length, times=repetitions); asm.push(64, times=repetitions)
    asm.push(2, times=repetitions); asm.op(0x5a, repetitions); asm.op(0xfa, repetitions)
    mutation._check(asm, 1, repetitions)
    asm.op(0x3d, repetitions); mutation._check(asm, 32, repetitions)
    load(32, repetitions); load(output, repetitions); asm.op(0x18, repetitions)
    asm.push(32, times=repetitions); asm.op(0x52, repetitions)
    load(0, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions)
    asm.push(0, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); load(0); asm.push(repetitions, 32); asm.op(0x14); mutation._require(asm)
    load(32); asm.push(xor, 32); asm.op(0x14); mutation._require(asm); asm.op(0)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _runtime(pattern, length, repetitions, tag=0):
    return _assemble(pattern, length, repetitions, tag, expected_xor(pattern, length, repetitions, tag))


def gas_accounting(pattern, length, repetitions):
    code, counts = _assemble(pattern, length, repetitions, 0, 0)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x17: 3, 0x18: 3, 0x3d: 2,
             0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10, 0x5a: 2, 0x5b: 1, 0x5e: 0, 0xfa: 0, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)]) for op, n in counts.items())
    words = 3 + (length + 31) // 32; memory = 3 * words + words * words // 512
    filled = 32; copy_gas = 0
    while filled < length:
        size = min(filled, length - filled); copy_gas += 3 + 3 * ((size + 31) // 32); filled += size
    native = 60 + 12 * ((length + 31) // 32); precompile = repetitions * (100 + native)
    gross = 21000 + ordinary + memory + copy_gas + precompile
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=memory,
        mcopy_gas=copy_gas, native_gas_per_call=native, warm_access_gas_per_call=100,
        precompile_gas=precompile, gross_gas=gross, refund_counter=0, refund_applied=0, charged_gas=gross,
        runtime_bytes=len(code))


def _envelope(gas):
    # Fixture continuation policy, not a protocol requirement or SSTORE sentry.
    return gas['gross_gas'] + (gas['native_gas_per_call'] + 62) // 63 + 2301


def fits(pattern, length, repetitions):
    gas = gas_accounting(pattern, length, repetitions); envelope = _envelope(gas)
    return (gas['runtime_bytes'] <= 24576 and envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_repetitions(pattern, length):
    _validate(pattern, length)
    if not fits(pattern, length, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, MAX_REPETITIONS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(pattern, length, middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(pattern, length, low) and not fits(pattern, length, low + 1)
    return low


def make_case(pattern='a5', length=64, level='full', *, repetitions=None):
    _validate(pattern, length)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero, single or full')
    maximum = full_repetitions(pattern, length)
    if repetitions is None:
        repetitions = {'zero': 0, 'single': 1, 'full': maximum}[level]
    else:
        if level != 'full': raise ValueError('explicit repetitions require level=full')
        original.integer(repetitions, 0, maximum, 'repetitions')
        level = 'explicit'
    gas = gas_accounting(pattern, length, repetitions); full_gas = gas_accounting(pattern, length, maximum); envelope = _envelope(full_gas)
    runtimes = [_runtime(pattern, length, repetitions, tag) for tag in range(USERS)]
    codes = [code for code, _ in runtimes]; counts = runtimes[0][1]
    assert all(other == counts for _, other in runtimes)
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']
    program = request['program']; routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = list(OWNERS)
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(), initial_storage={}) for owner, code in zip(OWNERS, codes)]
    prefix = f'sha256-bulk-{pattern}-l{length}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    return dict(sample_id=f'{prefix}-n{repetitions}', family='sha256-bulk', corpus_id=LINEAR_CORPUS if length in LINEAR_LENGTHS else CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_sha256_bulk_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent source review and all18 native/signature/receipt/code/account/empty-root/witness/unique-input/original-real-quote gates precede freeze and actuals.',
        scope='Finite whole-proposal actual/node/complete-quote comparison. No per-op PGU attribution, global/cache bound, price fit, proof, GPU or production claim.',
        parameters=dict(pattern=pattern, input_bytes=length, workload_level=level, repetitions=repetitions, full_repetitions=maximum,
            precompile_address='0x02', expected_precompile_success=True, expected_precompile_calls=USERS * repetitions,
            expected_precompile_calls_per_user=repetitions, owner_tags=list(range(USERS)),
            expected_xor_hex=[f'0x{expected_xor(pattern, length, repetitions, tag):064x}' for tag in range(USERS)],
            input_schedule='BE256((owner_tag<<128)|index) || pattern_byte*(input_bytes-32), index0..R-1',
            tail_pattern_byte=0 if pattern == 'zero' else 0xa5, outer_calldata_bytes=0,
            counter_memory_offset=0, accumulator_memory_offset=32, input_memory_offset=64, output_memory_offset=64 + length,
            memory_words=3 + (length + 31) // 32, output_sentinel=2, expected_counter=repetitions,
            account_count=USERS, aggregate_state_size=0, state_size_per_account=0,
            expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0,
            expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            user_transactions_per_block=USERS, all_user_transactions=USERS, all_transactions_including_anchor=USERS + 1,
            block_count=1, visits_per_account=1, transaction_kind='call', transaction_gas_limit=envelope,
            transaction_envelope_reserve=envelope - full_gas['gross_gas'], header_gas_limit=HEADER_GAS_LIMIT,
            charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS, gas_accounting_per_user=gas,
            expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter=0, expected_user_refund_applied=0,
            expected_user_charged_gas=gas['charged_gas'], expected_block_charged_gas=block_gas,
            near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            recipient_schedule=list(OWNERS), expected_terminal='STOP', expected_user_success=True,
            expected_user_receipt_statuses=[True] * USERS, expected_anchor_success=True, expected_exceptional_instructions=0,
            execution_profile='evm-gas-only', preload_created_accounts_or_code=False, shared_runtime=False,
            runtime_bytes=len(codes[0]), runtime_sha256=[digest(code) for code in codes], unused_primary_code_hex='0x00',
            initial_storage_rule='Two distinct authenticated code accounts with empty persistent storage; whole-world fees, nonces and Anchor still change.',
            range_rule='Preallocate sentinel2; seed and logarithmically MCOPY both patterns even R0. Require status1,returndata32,exact XOR/counter and STOP.'), request=request)


def source_modules():
    return {'initcode_preparation.py': initcode_preparation, 'spawn_memory.py': spawn_memory,
            'storage_fanout.py': storage_fanout, 'storage_mutation.py': mutation, 'top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._check': mutation._check, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'sha256_bulk.{fn.__name__}': fn for fn in (_validate, expected_xor, _assemble, _runtime, gas_accounting, _envelope, fits, full_repetitions, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL, linear_regime=False):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    if type(linear_regime) is not bool: raise ValueError('linear_regime must be bool')
    lengths = LINEAR_LENGTHS if linear_regime else LENGTHS
    corpus = LINEAR_CORPUS if linear_regime else CORPUS
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for pattern in PATTERNS:
        for length in lengths:
            for level in LEVELS:
                row = make_case(pattern, length, level); raw = encoded(row.pop('request'))
                path = f"requests/{row['sample_id']}.json"; write(output / path, raw)
                rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 18
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=corpus, frozen_model_id=model_id,
        purpose=f"18 finite unique-input SHA256 requests: zero/a5 tails,{'/'.join(map(str, lengths))} bytes, own-zero/single/maximal-uniform-full.",
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        patterns=list(PATTERNS), lengths=list(lengths), levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False,
        primary_sources=['https://ethereum.github.io/execution-specs/src/ethereum/forks/berlin/vm/precompiled_contracts/sha256.py.html', 'https://eips.ethereum.org/EIPS/eip-5656'],
        native_gas_source='Frozen revm-precompile41.0.0 src/hash.rs:60+12ceil(input_bytes/32)',
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'], cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    parser.add_argument('--linear-regime', action='store_true', help='select 1269664/1269665/2097152-byte input lengths')
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id, linear_regime=args.linear_regime)}))
