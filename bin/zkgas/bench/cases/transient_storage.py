#!/usr/bin/env python3
"""Generate eight guarded transient-storage inputs with their own R0 controls."""
import argparse
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

CORPUS = 'unzen-transient-storage-20261103'
MODES = ('stop', 'revert')
KEY_COUNTS = (16, 4096)
LEVELS = ('zero', 'full')
USERS = 4
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
ENVELOPE_RESERVE = 1000  # Test envelope margin; TSTORE has no SSTORE gas sentry.
OWNER = f'0x{BASE_ADDRESS:040x}'


def _validate(mode, keys):
    if type(mode) is not str or mode not in MODES:
        raise ValueError('mode must be stop or revert')
    if type(keys) is not int or keys not in KEY_COUNTS:
        raise ValueError('keys must be 16 or 4096')


def _runtime(mode, keys, rounds):
    """Guard three traversals, counting actual executed 64-pair batches and tail."""
    _validate(mode, keys); original.integer(rounds, 0, 65536, 'rounds per key')
    asm = _Factory(); batches, tail = divmod(rounds, 64)

    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)

    def store(offset, value, times=1):
        asm.push(value, times=times); asm.push(offset, times=times); asm.op(0x52, times)

    def add(offset, value, times):
        load(offset, times); asm.push(value, times=times); asm.op(1, times)
        asm.push(offset, times=times); asm.op(0x52, times)

    def read(expected, times):
        mutation._key(asm, False, times); asm.op(0x5c, times); mutation._check(asm, expected, times)

    def write_value(value, times):
        asm.push(value, times=times); mutation._key(asm, False, times); asm.op(0x5d, times)

    def begin(name):
        store(0, 0); asm.label(name, keys + 1)
        load(0, keys + 1); asm.push(keys, times=keys + 1); asm.op(0x14, keys + 1)
        asm.target(name + '-done', keys + 1); asm.op(0x57, keys + 1)
        load(64, keys); load(0, keys); asm.op(1, keys)
        asm.push(96, times=keys); asm.op(0x52, keys)

    def end(name):
        add(0, 1, keys); asm.target(name, keys); asm.op(0x56, keys)
        asm.label(name + '-done'); load(0); mutation._check(asm, keys)

    asm.push(0); asm.op(0x5c); mutation._check(asm, 0)
    asm.push(1); asm.push(0); asm.op(0x5d)
    store(64, 1); store(160, 0)
    begin('seed'); read(0, keys); write_value(5, keys); read(5, keys); end('seed')
    begin('dirty'); read(5, keys); store(128, 0, keys)
    visits = keys * (batches + 1); asm.label('batch', visits)
    load(128, visits); asm.push(rounds - tail, times=visits); asm.op(0x14, visits)
    asm.target('batch-done', visits); asm.op(0x57, visits)
    for _ in range(64):
        write_value(7, keys * batches); write_value(5, keys * batches)
    add(128, 64, keys * batches); asm.target('batch', keys * batches); asm.op(0x56, keys * batches)
    asm.label('batch-done', keys)
    for _ in range(tail): write_value(7, keys); write_value(5, keys)
    add(128, tail, keys); load(128, keys); mutation._check(asm, rounds, keys)
    load(160, keys); load(128, keys); asm.op(1, keys); asm.push(160, times=keys); asm.op(0x52, keys)
    read(5, keys); end('dirty')
    begin('cleanup'); read(5, keys); write_value(0, keys); read(0, keys); end('cleanup')
    asm.push(0); asm.op(0x5c); mutation._check(asm, 1)
    load(160); asm.push(keys * rounds, width=4); asm.op(0x14); mutation._require(asm)
    if mode == 'stop': asm.op(0)
    else: asm.push(0); asm.push(0); asm.op(0xfd)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(mode, keys, rounds):
    _validate(mode, keys); original.integer(rounds, 0, 65536, 'rounds per key')
    _, counts = _runtime(mode, keys, rounds)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3,
             0x5c: 0, 0x5d: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfd: 0, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)])
                   for op, n in counts.items())
    reads = 100 * counts['opcode:0x5c']; writes = 100 * counts['opcode:0x5d']
    gross = 21000 + ordinary + 18 + reads + writes
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=18,
        transient_read_gas=reads, transient_write_gas=writes, gross_gas=gross,
        refund_counter_before_terminal=0, refund_counter=0, refund_applied=0, charged_gas=gross)


def fits(mode, keys, rounds):
    """Apply the candidate's own envelope, charged block cap and ordered prefixes."""
    _validate(mode, keys); original.integer(rounds, 0, 65536, 'rounds per key')
    gas = gas_accounting(mode, keys, rounds); envelope = gas['gross_gas'] + ENVELOPE_RESERVE
    code, _ = _runtime(mode, keys, rounds)
    return (len(code) <= 24576 and envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_rounds(mode, keys):
    _validate(mode, keys)
    if not fits(mode, keys, 0): raise ValueError('R0 does not fit the frozen constraints')
    low, high = 0, 65536
    while low < high:
        middle = (low + high + 1) // 2
        if fits(mode, keys, middle): low = middle
        else: high = middle - 1
    assert 0 < low < 65536 and fits(mode, keys, low) and not fits(mode, keys, low + 1)
    return low


def make_case(mode='stop', keys=16, level='full'):
    _validate(mode, keys)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero or full')
    maximum = full_rounds(mode, keys); rounds = 0 if level == 'zero' else maximum
    gas = gas_accounting(mode, keys, rounds)
    envelope = gas_accounting(mode, keys, maximum)['gross_gas'] + ENVELOPE_RESERVE
    code, counts = _runtime(mode, keys, rounds)
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']
    stop = mode == 'stop'; request['expected_user_success'] = stop
    program = request['program']; routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = [OWNER] * USERS
    program['prestate_contracts'] = [dict(address=OWNER, code_hex='0x' + code.hex(), initial_storage={})]
    block_gas = ANCHOR_GAS + USERS * gas['charged_gas']; prefix = f'transient-storage-{mode}-k{keys}'
    return dict(sample_id=f'{prefix}-r{rounds}', family='transient-storage', corpus_id=CORPUS,
        role='control' if rounds == 0 else 'candidate', baseline_sample_id=f'{prefix}-r0',
        data_role='unfitted_transient_storage_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent source review and all8 native/signature/receipt/whole-world/authenticated-code/account/root/witness/original-real-quote/counter gates precede freeze and actuals. Native acceptance alone is insufficient.',
        scope='Finite transient-state/reset/journaling composition under frozen c893 pricing. Preserve own-R0 controls and failures. Compare actual same-proposal SP1 PGU, literal node per-block estimates/sum and complete proposal estimate separately. No refit, per-operation actual allocation, global or production bound, generated proof or GPU/throughput claim.',
        parameters=dict(mode=mode, keys_per_account=keys, account_count=1, aggregate_state_size=0,
            state_size_per_account=0, peak_transient_state_entries_per_transaction=keys + 1,
            workload_level=level, rounds_per_key=rounds, full_rounds_per_key=maximum,
            user_transactions_per_block=USERS, all_user_transactions=USERS, visits_per_account=USERS,
            block_count=1, all_transactions_including_anchor=USERS + 1, transaction_kind='call',
            transaction_gas_limit=envelope, transaction_envelope_reserve=ENVELOPE_RESERVE,
            header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS,
            expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter_before_terminal=0,
            expected_user_refund_counter=0, expected_user_refund_applied=0,
            expected_user_charged_gas=gas['charged_gas'], gas_accounting_per_user=gas,
            expected_block_charged_gas=block_gas, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            expected_distinct_transient_owner_slot_pairs=keys + 1,
            expected_transient_reads=USERS * (2 + 6 * keys), expected_transient_writes=USERS * (1 + keys * (2 + 2 * rounds)),
            expected_seed_transient_writes=USERS * keys, expected_dirty_transient_writes=2 * USERS * keys * rounds,
            expected_cleanup_transient_writes=USERS * keys, expected_sentinel_transient_writes=USERS,
            expected_transient_writes_per_user=1 + keys * (2 + 2 * rounds),
            expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0,
            expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            recipient_schedule=[OWNER] * USERS, body_key_schedule=list(range(1, keys + 1)),
            transient_phase_order=['seed', 'dirty', 'cleanup'], expected_counter=keys,
            expected_round_counter=rounds, expected_marker=keys * rounds,
            counter_memory_offset=0, phase_memory_offset=64, key_memory_offset=96,
            round_counter_memory_offset=128, marker_memory_offset=160,
            dirty_batch_size=64, dirty_batches_per_key=rounds // 64, dirty_tail_pairs_per_key=rounds % 64,
            initial_storage_rule='One code account with empty persistent storage. Every user starts with an empty owner-qualified transient map.',
            range_rule='Four users visit one account: guard transient0=0, set sentinel1; seed keys1..K 0->5; each key R 5->7->5 pairs in64-pair batches plus tail; cleanup5->0; check sentinel1. STOP retains sentinel until transaction finalization; root REVERT restores transient frame state and retains attempted traces.',
            witness_policy='Transient owner-slot pairs are not persistent MPT targets. Authenticate code/account/whole-world evidence; capture observed persistent tracer selectors and derive their source/system scope during qualification.',
            expected_terminal='STOP' if stop else 'REVERT', expected_user_success=stop,
            expected_user_receipt_statuses=[stop] * USERS, expected_anchor_success=True,
            expected_exceptional_instructions=0, outer_calldata_bytes=0, execution_profile='evm-gas-only',
            preload_created_accounts_or_code=False, runtime_bytes=len(code), runtime_sha256=digest(code),
            shared_runtime=True, unused_primary_code_hex='0x00'), request=request)


def source_modules():
    return {'initcode_preparation.py': initcode_preparation, 'spawn_memory.py': spawn_memory,
            'storage_fanout.py': storage_fanout, 'storage_mutation.py': mutation, 'top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._key': mutation._key,
        'storage_mutation._check': mutation._check, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        'transient_storage._runtime': _runtime, 'transient_storage.gas_accounting': gas_accounting,
        'transient_storage.fits': fits, 'transient_storage.full_rounds': full_rounds,
        'transient_storage._validate': _validate, 'transient_storage.make_case': make_case,
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for mode in MODES:
        for keys in KEY_COUNTS:
            for level in LEVELS:
                row = make_case(mode, keys, level); raw = encoded(row.pop('request'))
                path = f"requests/{row['sample_id']}.json"
                write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 8
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='8 guarded transient-storage inputs: stop/revert, keys16/4096, own-R0/maximal-uniform-full.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        modes=list(MODES), keys_per_account=list(KEY_COUNTS), levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False,
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Finite frozen-pricing validation. Independent source review and all8 native/signature/receipt/whole-world/authenticated-code/account/root/witness/original-real-quote/counter gates precede freeze and actuals. Observe persistent tracer selectors before assigning their meaning; transient owner-slot pairs are not persistent MPT targets. Whole-world fees/nonces/Anchor change. Preserve controls and failures; no refit, per-operation actual allocation or global bound.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
