#!/usr/bin/env python3
"""Generate constant-code empty-storage warm-read calldata catalog probes."""
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
from storage_fanout import MODEL
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-warm-read-catalogs-20261104'
CATALOGS = ('ordered', 'mixed')
LEVELS = ('zero', 'full')
QUERY = 0x1122334455667788 << 192
SENTINEL = 0xa55a
KEY_COUNT = 512
UNROLL = 64
MAX_REPETITIONS = 1_000_000
CATALOG_FILE_CAP = 128 * 1024
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
PRIMARY = '0x' + '66' * 20


def _validate_keys(keys):
    if type(keys) not in (list, tuple) or len(keys) != KEY_COUNT:
        raise ValueError('keys must be a bounded list or tuple of512 keys')
    for key in keys:
        if type(key) is not str or re.fullmatch(r'0x[0-9a-f]{64}', key) is None:
            raise ValueError('keys must be canonical lowercase U256 hex')
        raw = bytes.fromhex(key[2:])
        if not all(raw[:8]) or raw[8:] != bytes(24):
            raise ValueError('keys require nonzero high64 bytes and zero low192 bits')
    if len(set(keys)) != KEY_COUNT or int(keys[-1], 16) != QUERY:
        raise ValueError('keys must be unique with fixed query last')
    return tuple(keys)


def _catalog(catalog):
    if type(catalog) is not str or catalog not in CATALOGS: raise ValueError('unknown catalog')
    values = []; seen = set(); state = 0x243f6a8885a308d3; current = 0x0101010101010101; mask = (1 << 64) - 1
    while len(values) < KEY_COUNT - 1:
        if catalog == 'ordered': value = current; current += 1
        else:
            state = (state + 0x9e3779b97f4a7c15) & mask; z = state
            z = ((z ^ (z >> 30)) * 0xbf58476d1ce4e5b9) & mask
            z = ((z ^ (z >> 27)) * 0x94d049bb133111eb) & mask
            value = z ^ (z >> 31)
        if not all(value.to_bytes(8, 'big')) or value << 192 == QUERY or value in seen: continue
        seen.add(value); values.append(f'0x{value << 192:064x}')
    return _validate_keys(values + [f'0x{QUERY:064x}'])


def _runtime(repetitions):
    """Keys are ONLY calldata; fixed-width code depends solely on R."""
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions'); asm = _Factory()
    def store(offset, value): asm.push(value); asm.push(offset); asm.op(0x52)
    def load(offset, times=1): asm.push(offset, times=times); asm.op(0x51, times)
    def guard(value, times=1): asm.push(value, 32, times); asm.op(0x14, times); mutation._require(asm, times)
    asm.op(0x36); guard(16384)
    for offset, value in ((0, 0), (64, 0), (96, SENTINEL)): store(offset, value)
    asm.label('seed', 513); load(0, 513); asm.push(512, times=513); asm.op(0x14, 513); asm.target('seed_done', 513); asm.op(0x57, 513)
    load(0, 512); asm.push(5, times=512); asm.op(0x1b, 512); asm.op(0x35, 512); asm.op(0x54, 512); guard(0, 512)
    load(0, 512); asm.push(1, times=512); asm.op(1, 512); asm.push(0, times=512); asm.op(0x52, 512); asm.target('seed', 512); asm.op(0x56, 512)
    asm.label('seed_done'); asm.push(16352); asm.op(0x35); asm.op(0x80); asm.op(0x54); guard(0)
    visits = repetitions + 1; asm.label('loop', visits)
    load(64, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for _ in range(UNROLL): asm.op(0x80, repetitions); asm.op(0x54, repetitions); asm.op(0x50, repetitions)
    load(64, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(64, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); asm.op(0x80); guard(QUERY); asm.op(0x50)
    for offset, value in ((0, 512), (64, repetitions), (96, SENTINEL)): load(offset); guard(value)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(repetitions):
    code, counts = _runtime(repetitions)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x1b: 3, 0x35: 3, 0x36: 2, 0x50: 2,
        0x51: 3, 0x52: 3, 0x54: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0x80: 3, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= (op := int(key[-2:], 16)) <= 0x7f else costs[op]) for key, n in counts.items())
    cold = 512 * 2100; warm = (64 * repetitions + 1) * 100
    gross = 135688 + ordinary + 12 + cold + warm; floor = 307720
    return dict(intrinsic_gas=135688, eip7623_floor_gas=floor, eip7623_floor_active=gross < floor,
        ordinary_opcode_gas=ordinary, memory_expansion_gas=12, cold_sload_gas=cold, warm_sload_gas=warm,
        cold_sload_count=512, warm_sload_count=64 * repetitions + 1, body_gas_per_repetition=64 * 105,
        gross_gas=gross, charged_gas=max(gross, floor), refund_counter=0, refund_applied=0, runtime_bytes=len(code))


def _constraint_failures(gas):
    envelope = gas['gross_gas'] + 1000
    return [name for name, failed in (
        ('charged_block_budget', ANCHOR_GAS + gas['charged_gas'] > BLOCK_BUDGET),
        ('transaction_fixture_cap', envelope > TRANSACTION_GAS_CAP),
        ('eip7825_cap', envelope > EIP7825_GAS_CAP),
        ('ordered_header_prefix', HEADER_GAS_LIMIT - ANCHOR_GAS < envelope),
        ('runtime_size', gas['runtime_bytes'] >= 24576)) if failed]


def fits(repetitions): return not _constraint_failures(gas_accounting(repetitions))


def full_repetitions():
    low, high = 0, MAX_REPETITIONS
    if not fits(0): raise ValueError('zero control cannot fit')
    while low < high:
        middle = (low + high + 1) // 2
        if fits(middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(low) and not fits(low + 1)
    return low


def _make_case(keys, catalog_id, level, *, custom, repetitions=None):
    keys = _validate_keys(keys)
    if type(level) is not str or level not in (*LEVELS, 'probe'): raise ValueError('level must be zero, probe or full')
    maximum = full_repetitions()
    if level == 'probe':
        if not custom: raise ValueError('probe requires an external catalog')
        original.integer(repetitions, 1, maximum - 1, 'probe repetitions')
        r = repetitions
    else:
        if repetitions is not None: raise ValueError('repetitions override requires probe level')
        r = 0 if level == 'zero' else maximum
    gas = gas_accounting(r)
    envelope = gas_accounting(maximum)['gross_gas'] + 1000; code, counts = _runtime(r)
    data = b''.join(bytes.fromhex(k[2:]) for k in keys); data_sha = digest(data)
    request = original.make_case('call', 'stop', count=1, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update(counts)
    program.update(code_hex='0x' + code.hex(), input_hex='0x' + data.hex(), expected_route_counts=routes)
    prefix = f'warm-read-{catalog_id}'; block_gas = ANCHOR_GAS + gas['charged_gas']
    return dict(sample_id=f'{prefix}-n{64 * r}', baseline_sample_id=f'{prefix}-n0', family='warm-read-catalogs',
        corpus_id=CORPUS + ('-custom-' + data_sha if custom else ''), role='control' if r == 0 else 'candidate',
        data_role='development_probe' if custom else 'unfitted_warm_read_catalog_validation', holdout_only=not custom, actuals_used_for_fitting=False,
        completion_gate='Independent byte/calldata order, cold/warm, zero-state, stack, gas, signature/account/root/witness and original quote gates before actuals.',
        scope='Finite whole-input single-user pilot; seeds/layout equality unmeasured, no collision, per-lookup, global bound or pricing claim.',
        parameters=dict(catalog_id=catalog_id, workload_level=level, ordered_keys=list(keys), raw_key_sha256=data_sha, calldata_sha256=data_sha,
            query_key=f'0x{QUERY:064x}', high_tail_rule='low192 bits zero; all eight high64 bytes nonzero;512 unique; fixed query last',
            catalog_source='external-data' if custom else 'deterministic-recipe', repetitions=r, full_repetitions=maximum, unroll_factor=64,
            bulk_operations=64 * r, seed_reads=512, target_query_reads=64 * r + 1, target_operations_per_user=64 * r + 1,
            warmup_target_operations_per_user=1, body_target_operations_per_user=64 * r, expected_persistent_storage_reads=513 + 64 * r,
            read_events=513 + 64 * r, accessed_unique=512, accessed_unique_keys=512, expected_persistent_storage_writes=0,
            initial_storage_entries=0, initial_state_size=0, aggregate_state_size=0, state_size_per_account=0,
            expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0, primary_code_owner_count=1, account_count=1,
            seed_counter_memory_offset=0, counter_memory_offset=64, protected_memory_offset=96, expected_counter=r,
            sentinel=SENTINEL, peak_memory_bytes=128, memory_words=4, outer_calldata_bytes=16384, calldata_zero_bytes=12288, calldata_nonzero_bytes=4096,
            all_user_transactions=1, user_transactions_per_block=1, all_transactions_including_anchor=2, block_count=1, visits_per_account=1,
            transaction_kind='call', transaction_nonce=0, primary_code_owner=PRIMARY, recipient_schedule=[PRIMARY], transaction_gas_limit=envelope,
            transaction_envelope_reserve=1000, header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS,
            gas_accounting_per_user=gas, expected_user_gross_gas=gas['gross_gas'], expected_user_charged_gas=gas['charged_gas'],
            expected_user_refund_counter=0, expected_user_refund_applied=0, expected_block_charged_gas=block_gas,
            near_full=block_gas >= 29_700_000, transaction_occupancy_numerator=gas['charged_gas'], transaction_occupancy_denominator=envelope,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS], full_plus_one_constraint_failures=_constraint_failures(gas_accounting(maximum + 1)),
            fullness_scope=('Intermediate development probe within original full signed envelope; no maximal-gas claim.' if level == 'probe'
                else 'Maximal uniform single-user R within15M fixture envelope; not a30M full-block certificate.'),
            expected_terminal='STOP', expected_user_success=True, expected_user_receipt_statuses=[True], expected_anchor_success=True, expected_exceptional_instructions=0,
            execution_profile='evm-gas-only', preload_created_accounts_or_code=False, shared_runtime=True, runtime_bytes=len(code), runtime_sha256=digest(code),
            initial_storage_rule='Default primary empty persistent storage, no extra prestate contracts, all reads zero; no per-key MPT witness padding.',
            range_rule='512 ordered unique cold reads followed by retained-query warmup and64-site warm zero reads per batch; counters/sentinel/query/empty stack guarded.'), request=request)


def make_case(catalog='ordered', level='full'): return _make_case(_catalog(catalog), catalog, level, custom=False)


def make_custom_case(keys, level='full'):
    keys = _validate_keys(keys); data = b''.join(bytes.fromhex(k[2:]) for k in keys)
    return _make_case(keys, 'custom-' + digest(data), level, custom=True)


def make_probe_case(keys, repetitions):
    keys = _validate_keys(keys); data = b''.join(bytes.fromhex(k[2:]) for k in keys)
    return _make_case(keys, 'custom-' + digest(data), 'probe', custom=True, repetitions=repetitions)


def source_modules():
    return {'cases/initcode_preparation.py': initcode_preparation, 'cases/spawn_memory.py': spawn_memory,
        'cases/storage_fanout.py': storage_fanout, 'cases/storage_mutation.py': mutation, 'cases/top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'warm_read_catalogs.{fn.__name__}': fn for fn in (_validate_keys, _catalog, _runtime, gas_accounting, _constraint_failures, fits, full_repetitions, _make_case, make_case, make_custom_case, make_probe_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def _read_catalog(path):
    if not isinstance(path, Path): raise ValueError('catalog_file must be a Path')
    with path.open('rb') as source: raw = source.read(CATALOG_FILE_CAP + 1)
    if len(raw) > CATALOG_FILE_CAP: raise ValueError('catalog file exceeds128KiB')
    def unique_object(pairs):
        obj = {}
        for key, value in pairs:
            if key in obj: raise ValueError('duplicate JSON field')
            obj[key] = value
        return obj
    obj = json.loads(raw, object_pairs_hook=unique_object)
    if type(obj) is not dict or set(obj) != {'schema_version', 'keys'} or type(obj['schema_version']) is not int or obj['schema_version'] != 1:
        raise ValueError('catalog requires exactly schema_version1 and keys')
    return _validate_keys(obj['keys']), digest(raw)


def prepare(output: Path, *, model_id=MODEL, catalog_file: Path | None = None, probe_repetitions=None):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    if output.exists() or output.is_symlink(): raise FileExistsError(output)
    external = None
    if probe_repetitions is not None:
        if catalog_file is None: raise ValueError('probe repetitions require an external catalog')
        original.integer(probe_repetitions, 1, full_repetitions() - 1, 'probe repetitions')
    if catalog_file is not None: keys, external = _read_catalog(catalog_file)
    levels = ('zero', 'probe', 'full') if probe_repetitions is not None else LEVELS
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for catalog in ((None,) if external else CATALOGS):
        for level in levels:
            row = make_probe_case(keys, probe_repetitions) if level == 'probe' else (make_custom_case(keys, level) if external else make_case(catalog, level))
            raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
            write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=rows[0]['corpus_id'], frozen_model_id=model_id,
        purpose='Finite constant-code512-key empty-storage calldata catalogs; ownzero/maximal single-user full.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(fn).encode()) for name, fn in helpers().items()},
        external_catalog_json_sha256=external, external_catalog_source_sha256=external,
        holdout_only=external is None, actuals_used_for_fitting=False, levels=list(levels), users=1,
        native_observability='Status/gas/routes and whole execution_result replay; no per-read native key/value export.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    parser.add_argument('--catalog-file', type=Path)
    parser.add_argument('--probe-repetitions', type=int)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id, catalog_file=args.catalog_file, probe_repetitions=args.probe_repetitions)}))
