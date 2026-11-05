#!/usr/bin/env python3
"""Generate 16 finite many-key warm dirty-write inputs with own R0 controls."""
import argparse
import inspect
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
from storage_fanout import BASE_ADDRESS, MODEL
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-storage-dirty-keys-20261103'
MODES = ('stop', 'revert')
KEY_COUNTS = (1, 16, 256, 1024)
LEVELS = ('zero', 'full')
USERS = 4
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
MAX_DISTINCT_PAIRS = 8192
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
ENVELOPE_RESERVE = 2301
SOURCES = ('storage_fanout.py', 'storage_mutation.py', 'initcode_preparation.py',
           'top_level_create.py', 'spawn_memory.py')


def _validate(mode, keys):
    if type(mode) is not str or mode not in MODES:
        raise ValueError('mode must be stop or revert')
    original.integer(keys, 1, 1024, 'keys')


def _slot_shift(value):
    if type(value) is not int or value not in (0, 64, 128, 192):
        raise ValueError('slot_shift must be 0, 64, 128 or 192')


def _runtime(mode, keys, rounds, *, slot_shift=0):
    """Guard all three key traversals; count actual executed 64-pair batches."""
    _validate(mode, keys); _slot_shift(slot_shift)
    original.integer(rounds, 0, 65536, 'rounds per key')
    asm = _Factory(); batches, tail = divmod(rounds, 64)

    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)

    def store(offset, value, times=1):
        asm.push(value, times=times); asm.push(offset, times=times); asm.op(0x52, times)

    def add(offset, value, times):
        load(offset, times); asm.push(value, times=times); asm.op(1, times)
        asm.push(offset, times=times); asm.op(0x52, times)

    def read(expected, times):
        mutation._key(asm, False, times); asm.op(0x54, times); mutation._check(asm, expected, times)

    def write_value(value, times):
        asm.push(value, times=times); mutation._key(asm, False, times); asm.op(0x55, times)

    def begin(name):
        store(0, 0)
        asm.label(name, keys + 1)
        load(0, keys + 1); asm.push(keys, times=keys + 1); asm.op(0x14, keys + 1)
        asm.target(name + '-done', keys + 1); asm.op(0x57, keys + 1)
        load(64, keys); load(0, keys)
        if slot_shift:
            asm.push(slot_shift, times=keys); asm.op(0x1b, keys)
        asm.op(1, keys)
        asm.push(96, times=keys); asm.op(0x52, keys)

    def end(name):
        add(0, 1, keys); asm.target(name, keys); asm.op(0x56, keys)
        asm.label(name + '-done'); load(0); mutation._check(asm, keys)

    asm.push(0); asm.op(0x54); asm.push(64); asm.op(0x52)
    store(160, 0)
    begin('seed')
    read(1, keys); write_value(5, keys); read(5, keys)
    end('seed')
    begin('dirty')
    read(5, keys); store(128, 0, keys)
    visits = keys * (batches + 1)
    asm.label('batch', visits)
    load(128, visits); asm.push(rounds - tail, times=visits); asm.op(0x14, visits)
    asm.target('batch-done', visits); asm.op(0x57, visits)
    for _ in range(64):
        write_value(7, keys * batches); write_value(5, keys * batches)
    add(128, 64, keys * batches)
    asm.target('batch', keys * batches); asm.op(0x56, keys * batches)
    asm.label('batch-done', keys)
    for _ in range(tail):
        write_value(7, keys); write_value(5, keys)
    add(128, tail, keys)
    load(128, keys); mutation._check(asm, rounds, keys)
    load(160, keys); load(128, keys); asm.op(1, keys)
    asm.push(160, times=keys); asm.op(0x52, keys)
    read(5, keys)
    end('dirty')
    begin('cleanup')
    read(5, keys); write_value(1, keys); read(1, keys)
    end('cleanup')
    asm.push(0); asm.op(0x54); load(64); asm.op(0x14); mutation._require(asm)
    load(160); asm.push(keys * rounds, width=4); asm.op(0x14); mutation._require(asm)
    if mode == 'stop': asm.op(0)
    else: asm.push(0); asm.push(0); asm.op(0xfd)
    asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(mode, keys, rounds, *, slot_shift=0) -> dict:
    _validate(mode, keys); _slot_shift(slot_shift)
    original.integer(rounds, 0, 65536, 'rounds per key')
    _, counts = _runtime(mode, keys, rounds, slot_shift=slot_shift)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x1b: 3, 0x51: 3, 0x52: 3,
             0x54: 0, 0x55: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfd: 0, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)]) for op, n in counts.items())
    reads = 2200 + keys * 2600; writes = keys * (3000 + 200 * rounds)
    gross = 21000 + ordinary + 18 + reads + writes
    before = 2800 * keys; refund = before if mode == 'stop' else 0; applied = min(refund, gross // 5)
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=18,
        storage_read_gas=reads, storage_write_gas=writes, gross_gas=gross,
        refund_counter_before_terminal=before, refund_counter=refund, refund_applied=applied, charged_gas=gross - applied)


def fits(mode, keys, rounds, *, slot_shift=0) -> bool:
    """Candidate's own gross envelope, receipt budget and ordered prefixes."""
    _validate(mode, keys); _slot_shift(slot_shift)
    original.integer(rounds, 0, 65536, 'rounds per key')
    gas = gas_accounting(mode, keys, rounds, slot_shift=slot_shift); envelope = gas['gross_gas'] + ENVELOPE_RESERVE
    code, _ = _runtime(mode, keys, rounds, slot_shift=slot_shift)
    return (USERS * (keys + 1) <= MAX_DISTINCT_PAIRS and len(code) <= 24576
        and envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_rounds(mode, keys, *, slot_shift=0) -> int:
    _validate(mode, keys); _slot_shift(slot_shift)
    low, high = 0, 65536
    while low < high:
        middle = (low + high + 1) // 2
        if fits(mode, keys, middle, slot_shift=slot_shift): low = middle
        else: high = middle - 1
    assert 0 < low < 65536 and fits(mode, keys, low, slot_shift=slot_shift) and not fits(mode, keys, low + 1, slot_shift=slot_shift)
    return low


def make_case(mode='stop', keys=1, level='full', *, slot_shift=0) -> dict:
    _validate(mode, keys); _slot_shift(slot_shift)
    if type(level) is not str or level not in LEVELS:
        raise ValueError('level must be zero or full')
    maximum = full_rounds(mode, keys, slot_shift=slot_shift); rounds = 0 if level == 'zero' else maximum
    gas = gas_accounting(mode, keys, rounds, slot_shift=slot_shift)
    envelope = gas_accounting(mode, keys, maximum, slot_shift=slot_shift)['gross_gas'] + ENVELOPE_RESERVE
    code, counts = _runtime(mode, keys, rounds, slot_shift=slot_shift)
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']
    stop = mode == 'stop'; request['expected_user_success'] = stop
    program = request['program']; routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    recipients = [f'0x{BASE_ADDRESS + i:040x}' for i in range(USERS)]
    bases = {owner: (i + 1) * 65536 for i, owner in enumerate(recipients)}
    phases = {owner: base + 1 for owner, base in bases.items()}
    program['transactions']['recipients'] = recipients
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(),
        initial_storage={'0x0': hex(start)} | {hex(key): '0x1' for key in (start + (j << slot_shift) for j in range(keys))})
        for owner, start in phases.items()]
    block_gas = ANCHOR_GAS + USERS * gas['charged_gas']; prefix = f'storage-dirty-keys-{mode}-k{keys}'
    return dict(sample_id=f'{prefix}-r{rounds}', family='storage-dirty-keys', corpus_id=CORPUS,
        role='control' if rounds == 0 else 'candidate', baseline_sample_id=f'{prefix}-r0',
        data_role='unfitted_storage_dirty_keys_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent source review and all16 native/signature/receipt/whole-world/authenticated per-account-root/canonical mutation-witness/original-real-quote/counter gates precede freeze and actuals. Native acceptance alone is insufficient.',
        scope='Finite many-key warm dirty-write composition under frozen c893 pricing. Preserve own-R0 controls and failures; compare actual same-proposal SP1 PGU, literal node block estimates/sum and complete proposal estimate separately. No refit, per-operation actual allocation, global or production bound, generated proof or GPU/throughput claim.',
        parameters=dict(mode=mode, keys_per_account=keys, account_count=USERS, aggregate_state_size=USERS * (keys + 1),
            state_size_per_account=keys + 1, workload_level=level, rounds_per_key=rounds, full_rounds_per_key=maximum,
            user_transactions_per_block=USERS, all_user_transactions=USERS, visits_per_account=1,
            block_count=1, all_transactions_including_anchor=USERS + 1, transaction_kind='call',
            transaction_gas_limit=envelope, transaction_envelope_reserve=ENVELOPE_RESERVE,
            header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS,
            expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter_before_terminal=gas['refund_counter_before_terminal'],
            expected_user_refund_counter=gas['refund_counter'], expected_user_refund_applied=gas['refund_applied'],
            expected_user_charged_gas=gas['charged_gas'], gas_accounting_per_user=gas,
            expected_block_charged_gas=block_gas, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            expected_distinct_account_slot_pairs=USERS * (keys + 1), expected_distinct_write_pairs=USERS * keys,
            expected_distinct_storage_keys_per_account=keys + 1, expected_distinct_body_keys_per_account=keys,
            expected_storage_reads=USERS * (2 + 6 * keys), expected_storage_writes=USERS * keys * (2 + 2 * rounds),
            expected_seed_storage_writes=USERS * keys, expected_dirty_storage_writes=2 * USERS * keys * rounds,
            expected_cleanup_storage_writes=USERS * keys, expected_main_storage_writes=USERS * keys * (2 + 2 * rounds),
            expected_phase_storage_writes=0, expected_storage_writes_per_account=keys * (2 + 2 * rounds),
            expected_initial_storage_entry_count=USERS * (keys + 1), expected_final_storage_entry_count=USERS * (keys + 1),
            expected_initial_storage_entry_count_per_account=keys + 1, expected_final_storage_entry_count_per_account=keys + 1,
            expected_net_main_storage_changes=0, phase_slot=0, phase_base_by_account=bases,
            initial_phase_by_account=phases, expected_final_phase_by_account=phases.copy(),
            recipient_schedule=recipients.copy(), start_phase_schedule=list(phases.values()),
            final_phase_schedule=list(phases.values()), storage_phase_order=['seed', 'dirty', 'cleanup'],
            body_key_schedule_by_account={owner: ([hex(start + (j << slot_shift)) for j in range(keys)] if slot_shift
                else list(range(start, start + keys))) for owner, start in phases.items()},
            expected_counter=keys, expected_round_counter=rounds, expected_marker=keys * rounds,
            counter_memory_offset=0, phase_memory_offset=64, key_memory_offset=96,
            round_counter_memory_offset=128, marker_memory_offset=160,
            dirty_batch_size=64, dirty_batches_per_key=rounds // 64, dirty_tail_pairs_per_key=rounds % 64,
            initial_storage_rule='B_i=(i+1)*65536; read-only raw phase0=B_i+1; raw keys B_i+1..B_i+K=1; all others absent. Identical own-R0/full maps.',
            range_rule='Four distinct users visit four distinct contracts once: seed all K 1->5; each key R warm dirty 5->7->5 pairs in64-pair batches plus tail; cleanup all K 5->1. Phase0 unchanged.',
            witness_policy='Canonical conservative mutation-aware raw-slot witnesses; attempted access pairs include read-only phase0, literal write pairs exclude it, including root rollback. Exposed resource counters can conservatively include read-only proof slots; qualify their definitions separately.',
            expected_terminal='STOP' if stop else 'REVERT', expected_user_success=stop,
            expected_user_receipt_statuses=[stop] * USERS, expected_anchor_success=True,
            expected_exceptional_instructions=0, outer_calldata_bytes=0, execution_profile='evm-gas-only',
            preload_created_accounts_or_code=False, runtime_bytes=len(code), runtime_sha256=digest(code),
            shared_runtime=True, unused_primary_code_hex='0x00'), request=request)


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._key': mutation._key,
        'storage_mutation._check': mutation._check, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        'storage_dirty_keys._runtime': _runtime, 'storage_dirty_keys.gas_accounting': gas_accounting,
        'storage_dirty_keys.fits': fits, 'storage_dirty_keys.full_rounds': full_rounds,
        'storage_dirty_keys._validate': _validate, 'storage_dirty_keys._slot_shift': _slot_shift,
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL) -> int:
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir()
    rows = []
    for mode in MODES:
        for keys in KEY_COUNTS:
            for level in LEVELS:
                row = make_case(mode, keys, level)
                raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
                write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 16
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='16 finite many-key warm dirty-write inputs: stop/revert, keys1/16/256/1024, own-R0/maximal-uniform-full.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in SOURCES},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        modes=list(MODES), keys_per_account=list(KEY_COUNTS), levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False,
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'],
        interpretation='Finite frozen-pricing validation. Independent source review and all16 native/signature/receipt/whole-world/authenticated-root/canonical-witness/original-real-quote/counter gates precede freeze and actuals. Maximal uniform R can be limited by refund/envelope prefixes or granularity before99% charged occupancy. Whole-world fees/nonces/Anchor change. Preserve controls and failures; no refit, per-operation actual allocation or global bound.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
