#!/usr/bin/env python3
"""Generate thirty compiler-free, two-owner LOG data/topic intent probes."""
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

CORPUS = 'unzen-log-receipt-bulk-20261104'
TOPICS = (1, 4)
LENGTHS = (0, 64, 262144)
PATTERNS = ('zero', 'a5')
DOMAINS = ((0, 'zero'), (64, 'zero'), (64, 'a5'), (262144, 'zero'), (262144, 'a5'))
LEVELS = ('zero', 'single', 'full')
MAX_REPETITIONS = 1_000_000
USERS = 2
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
OWNERS = tuple(f'0x{BASE_ADDRESS + tag:040x}' for tag in range(USERS))


def _validate(topics, length, pattern, repetitions=0, tag=0):
    if type(topics) is not int or topics not in TOPICS: raise ValueError('topics must be 1 or 4')
    if type(length) is not int or length not in LENGTHS: raise ValueError('input length must be 0,64 or262144')
    if type(pattern) is not str or pattern not in PATTERNS or (length == 0 and pattern != 'zero'): raise ValueError('pattern must be zero/a5; empty input requires zero')
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions'); original.integer(tag, 0, USERS - 1, 'owner tag')


def expected_sequence(topics, length, pattern, repetitions, tag=0):
    """Debug intent commitment, never a protocol receipt root or sizing input.

    SHA256 over ordered address20 || topic_count1 || topics32*N || length8 ||
    SHA256(data)32 records. Empty sequences commit the empty byte string.
    """
    _validate(topics, length, pattern, repetitions, tag)
    tail = bytes([0 if pattern == 'zero' else 0xa5]) * max(0, length - 32)
    sequence = hashlib.sha256(); owner = bytes.fromhex(OWNERS[tag][2:])
    for index in range(repetitions):
        data = ((tag << 128) | index).to_bytes(32, 'big') + tail if length else b''
        ordered = b''.join(((tag << 128) | (j << 64) | index).to_bytes(32, 'big') for j in range(topics))
        sequence.update(owner + bytes([topics]) + ordered + length.to_bytes(8, 'big') + hashlib.sha256(data).digest())
    return sequence.hexdigest()


def _runtime(topics, length, pattern, repetitions, tag=0):
    """Fixed immediate widths preserve shape across count/tag/pattern selections."""
    _validate(topics, length, pattern, repetitions, tag); asm = _Factory(); extent = max(32, length)
    def store(offset, value, times=1, width=3):
        asm.push(value, width, times); asm.push(offset, times=times); asm.op(0x52, times)
    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)
    # Preallocate once. R0 prepares exactly the same payload as its active peers.
    store(64 + extent - 32, 0); store(0, 0); store(32, 0)
    store(64, int.from_bytes(bytes([0 if pattern == 'zero' else 0xa5]) * 32, 'big'), width=32)
    filled = 32
    while filled < extent:
        size = min(filled, extent - filled)
        asm.push(size); asm.push(64); asm.push(64 + filled); asm.op(0x5e); filled += size
    visits = repetitions + 1; asm.label('loop', visits)
    load(0, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    load(0, repetitions); asm.push(tag << 128, 32, repetitions); asm.op(0x17, repetitions)
    asm.push(64, times=repetitions); asm.op(0x52, repetitions)
    load(64, repetitions); asm.push(32, times=repetitions); asm.op(0x52, repetitions)
    # LOG pops topic0 first, so push topic N-1 through topic0.
    for j in reversed(range(topics)):
        load(0, repetitions); asm.push((tag << 128) | (j << 64), 32, repetitions); asm.op(0x17, repetitions)
    asm.push(length, times=repetitions); asm.push(64, times=repetitions); asm.op(0xa0 + topics, repetitions)
    load(0, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(0, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); load(0); asm.push(repetitions, 32); asm.op(0x14); mutation._require(asm)
    load(32); asm.push(((tag << 128) | (repetitions - 1)) if repetitions else 0, 32); asm.op(0x14); mutation._require(asm); asm.op(0)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(topics, length, pattern, repetitions):
    code, counts = _runtime(topics, length, pattern, repetitions)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x17: 3, 0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10, 0x5b: 1, 0x5e: 0, 0xa1: 0, 0xa4: 0, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)]) for op, n in counts.items())
    words = 2 + max(32, length) // 32; memory = 3 * words + words * words // 512
    filled = 32; copy_gas = 0
    while filled < max(32, length):
        size = min(filled, max(32, length) - filled); copy_gas += 3 + 3 * ((size + 31) // 32); filled += size
    event = 375 + 375 * topics + 8 * length; log_gas = repetitions * event
    gross = 21000 + ordinary + memory + copy_gas + log_gas
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=memory,
        mcopy_gas=copy_gas, log_gas_per_event=event, log_gas=log_gas, gross_gas=gross,
        refund_counter=0, refund_applied=0, charged_gas=gross, runtime_bytes=len(code))


def _envelope(gas):
    # Fixture continuation margin only: no CALL/EIP150 and no SSTORE sentry.
    return gas['gross_gas'] + 1000


def fits(topics, length, pattern, repetitions):
    gas = gas_accounting(topics, length, pattern, repetitions); envelope = _envelope(gas)
    return (gas['runtime_bytes'] < 24576 and envelope <= TRANSACTION_GAS_CAP and envelope <= EIP7825_GAS_CAP
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_repetitions(topics, length, pattern):
    _validate(topics, length, pattern)
    if not fits(topics, length, pattern, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, MAX_REPETITIONS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(topics, length, pattern, middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(topics, length, pattern, low) and not fits(topics, length, pattern, low + 1)
    return low


def make_case(topics=1, length=64, pattern='a5', level='full'):
    _validate(topics, length, pattern)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero, single or full')
    maximum = full_repetitions(topics, length, pattern); repetitions = {'zero': 0, 'single': 1, 'full': maximum}[level]
    gas = gas_accounting(topics, length, pattern, repetitions); full_gas = gas_accounting(topics, length, pattern, maximum); envelope = _envelope(full_gas)
    runtimes = [_runtime(topics, length, pattern, repetitions, tag) for tag in range(USERS)]; codes = [code for code, _ in runtimes]; counts = runtimes[0][1]
    assert all(other == counts for _, other in runtimes)
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = list(OWNERS)
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(), initial_storage={}) for owner, code in zip(OWNERS, codes)]
    prefix = f'log-receipt-bulk-log{topics}-{pattern}-l{length}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    return dict(sample_id=f'{prefix}-n{repetitions}', family='log-receipt-bulk', corpus_id=CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_log_receipt_bulk_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent byte intent, source, all30 native/signature/receipt/code/account/empty-root/witness/original quote gates precede freeze and actuals.',
        scope='Finite whole-proposal comparison. Native exports gas/status, not log data/topics. Intent digests are not receipt roots; no per-event PGU, bloom noncollision, price fit or production claim.',
        parameters=dict(topics=topics, input_bytes=length, pattern=pattern, workload_level=level, repetitions=repetitions, full_repetitions=maximum,
            expected_logs=USERS * repetitions, expected_logs_per_user=repetitions, owner_tags=list(range(USERS)),
            expected_last_input_hex=[f'0x{((tag << 128) | (repetitions - 1)) if repetitions else 0:064x}' for tag in range(USERS)],
            expected_log_sequence_sha256=[expected_sequence(topics, length, pattern, repetitions, tag) for tag in range(USERS)],
            topic_schedule='BE256((owner_tag<<128)|(topic_index<<64)|index), topic_index0..N-1, index0..R-1',
            data_schedule='empty if L0 else BE256((owner_tag<<128)|index) || pattern_byte*(L-32)',
            intent_commitment='SHA256(concat(address20||topic_count1||ordered_topics32*N||length8||SHA256(data)32)); debug intent only, not receipt root',
            tail_pattern_byte=0 if pattern == 'zero' else 0xa5, outer_calldata_bytes=0,
            counter_memory_offset=0, last_input_memory_offset=32, input_memory_offset=64, memory_words=2 + max(32, length) // 32,
            expected_counter=repetitions, account_count=USERS, aggregate_state_size=0, state_size_per_account=0,
            expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0, expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            user_transactions_per_block=USERS, all_user_transactions=USERS, all_transactions_including_anchor=USERS + 1,
            block_count=1, visits_per_account=1, transaction_kind='call', transaction_gas_limit=envelope,
            transaction_envelope_reserve=1000, header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET,
            expected_anchor_gas=ANCHOR_GAS, gas_accounting_per_user=gas, expected_user_gross_gas=gas['gross_gas'],
            expected_user_refund_counter=0, expected_user_refund_applied=0, expected_user_charged_gas=gas['charged_gas'],
            expected_block_charged_gas=block_gas, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            recipient_schedule=list(OWNERS), expected_terminal='STOP', expected_user_success=True,
            expected_user_receipt_statuses=[True] * USERS, expected_anchor_success=True, expected_exceptional_instructions=0,
            execution_profile='evm-gas-only', preload_created_accounts_or_code=False, shared_runtime=False,
            runtime_bytes=len(codes[0]), runtime_sha256=[digest(code) for code in codes], unused_primary_code_hex='0x00',
            initial_storage_rule='Two distinct authenticated code accounts with empty persistent storage; fees, nonces and Anchor still change.',
            range_rule='Preallocate max(32,L), seed and logarithmically MCOPY even R0; exact ordered LOG bytes are external intent, counter/last-input guards terminate STOP.'), request=request)


def source_modules():
    return {'initcode_preparation.py': initcode_preparation, 'spawn_memory.py': spawn_memory,
            'storage_fanout.py': storage_fanout, 'storage_mutation.py': mutation, 'top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'log_receipt_bulk.{fn.__name__}': fn for fn in (_validate, expected_sequence, _runtime, gas_accounting, _envelope, fits, full_repetitions, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for topics in TOPICS:
        for length, pattern in DOMAINS:
            for level in LEVELS:
                row = make_case(topics, length, pattern, level); raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 30
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='Thirty finite LOG1/LOG4 data/topic cases: L0/64/262144, zero/a5 (L0 zero only), own-zero/single/maximal uniform full.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        topics=list(TOPICS), domains=[dict(input_bytes=length, pattern=pattern) for length, pattern in DOMAINS], levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False, primary_sources=['https://ethereum.github.io/yellowpaper/paper.pdf', 'https://eips.ethereum.org/EIPS/eip-5656'],
        log_gas_source='Frozen revm-interpreter41 host::log and revm-context-interface41:375 static+375*N+8*L, base charged exactly once',
        native_observability='Native ControlledNativeReceipt exports gas/status, not log data/topics; full-vs-minimized execution_result replay is compared by frozen constructor.',
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'], cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
