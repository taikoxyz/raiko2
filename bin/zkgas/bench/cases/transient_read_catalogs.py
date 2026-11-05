#!/usr/bin/env python3
"""Generate guarded direct-write transient-map calldata read catalogs."""
import argparse
import inspect
import json
from pathlib import Path
import re

import warm_read_catalogs as catalogs
from initcode_preparation import _Factory
import storage_mutation as mutation
import top_level_create as original
from spawn_memory import digest, encoded, write

CORPUS = 'unzen-transient-read-catalogs-20261105'
MODEL = '5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11'
KEY_COUNT = 512
QUERY = catalogs.QUERY
PRIMARY = catalogs.PRIMARY
SENTINEL = catalogs.SENTINEL
UNROLL = 64
MAX_REPETITIONS = catalogs.MAX_REPETITIONS
LEVELS = ('zero', 'full')


def _runtime(repetitions, *, key_count=KEY_COUNT):
    catalogs._key_count(key_count); original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    asm = _Factory()
    def store(offset, value): asm.push(value); asm.push(offset); asm.op(0x52)
    def load(offset, times=1): asm.push(offset, times=times); asm.op(0x51, times)
    def guard(value, times=1):
        asm.push(value, 32, times); asm.op(0x14, times); mutation._require(asm, times)
    asm.op(0x36); guard(32 * key_count)
    asm.push(QUERY, 32); asm.op(0x5c); guard(0)
    for offset, value in ((0, 0), (64, 0), (96, SENTINEL)): store(offset, value)
    visits = key_count + 1; asm.label('seed', visits)
    load(0, visits); asm.push(key_count, times=visits); asm.op(0x14, visits); asm.target('seed_done', visits); asm.op(0x57, visits)
    asm.push(1, times=key_count); load(0, key_count); asm.push(5, times=key_count); asm.op(0x1b, key_count); asm.op(0x35, key_count); asm.op(0x5d, key_count)
    load(0, key_count); asm.push(1, times=key_count); asm.op(1, key_count); asm.push(0, times=key_count); asm.op(0x52, key_count); asm.target('seed', key_count); asm.op(0x56, key_count)
    asm.label('seed_done'); asm.push(32 * (key_count - 1)); asm.op(0x35); asm.op(0x80); asm.op(0x5c); guard(1)
    visits = repetitions + 1; asm.label('loop', visits)
    load(64, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for _ in range(UNROLL): asm.op(0x80, repetitions); asm.op(0x5c, repetitions); asm.op(0x50, repetitions)
    load(64, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(64, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); asm.op(0x80); guard(QUERY); asm.op(0x50)
    for offset, value in ((0, key_count), (64, repetitions), (96, SENTINEL)): load(offset); guard(value)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(repetitions, *, key_count=KEY_COUNT):
    code, counts = _runtime(repetitions, key_count=key_count)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x1b: 3, 0x35: 3, 0x36: 2, 0x50: 2,
             0x51: 3, 0x52: 3, 0x5c: 0, 0x5d: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0x80: 3, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= (op := int(key[-2:], 16)) <= 0x7f else costs[op]) for key, n in counts.items())
    intrinsic = 21000 + 224 * key_count; floor = 21000 + 560 * key_count
    reads = 100 * (2 + UNROLL * repetitions); stores = 100 * key_count
    gross = intrinsic + ordinary + 12 + reads + stores
    return dict(intrinsic_gas=intrinsic, eip7623_floor_gas=floor, eip7623_floor_active=gross < floor,
        ordinary_opcode_gas=ordinary, memory_expansion_gas=12, transient_load_gas=reads,
        transient_store_gas=stores, transient_load_count=2 + UNROLL * repetitions, transient_store_count=key_count,
        body_gas_per_repetition=UNROLL * 105, gross_gas=gross, charged_gas=max(gross, floor),
        refund_counter=0, refund_applied=0, runtime_bytes=len(code))


def _constraint_failures(gas):
    envelope = gas['charged_gas'] + 1000
    return [name for name, failed in (
        ('charged_block_budget', catalogs.ANCHOR_GAS + gas['charged_gas'] > catalogs.BLOCK_BUDGET),
        ('transaction_fixture_cap', envelope > catalogs.TRANSACTION_GAS_CAP),
        ('eip7825_cap', envelope > catalogs.EIP7825_GAS_CAP),
        ('ordered_header_prefix', catalogs.HEADER_GAS_LIMIT - catalogs.ANCHOR_GAS < envelope),
        ('runtime_size', gas['runtime_bytes'] > 24576)) if failed]


def fits(repetitions, *, key_count=KEY_COUNT):
    return not _constraint_failures(gas_accounting(repetitions, key_count=key_count))


def full_repetitions(*, key_count=KEY_COUNT):
    catalogs._key_count(key_count); low, high = 0, MAX_REPETITIONS
    if not fits(0, key_count=key_count): raise ValueError('zero control cannot fit')
    while low < high:
        middle = (low + high + 1) // 2
        if fits(middle, key_count=key_count): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(low, key_count=key_count) and not fits(low + 1, key_count=key_count)
    return low


def _make_case(keys, catalog_id, level, *, repetitions=None, key_count=KEY_COUNT, external=False):
    keys = catalogs._validate_keys(keys, key_count=key_count)
    if type(level) is not str or level not in (*LEVELS, 'probe'): raise ValueError('level must be zero, probe or full')
    maximum = full_repetitions(key_count=key_count)
    if level == 'probe':
        original.integer(repetitions, 1, maximum - 1, 'probe repetitions'); r = repetitions
    else:
        if repetitions is not None: raise ValueError('repetitions override requires probe level')
        r = 0 if level == 'zero' else maximum
    code, counts = _runtime(r, key_count=key_count); gas = gas_accounting(r, key_count=key_count)
    envelope = gas_accounting(maximum, key_count=key_count)['charged_gas'] + 1000
    data = b''.join(bytes.fromhex(k[2:]) for k in keys); data_sha = digest(data)
    request = original.make_case('call', 'stop', count=1, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update(counts)
    program.update(code_hex='0x' + code.hex(), input_hex='0x' + data.hex(), expected_route_counts=routes)
    prefix = f'transient-read-{catalog_id}-k{key_count}'
    return dict(sample_id=f'{prefix}-n{UNROLL*r}', baseline_sample_id=f'{prefix}-n0', family='transient-read-catalogs',
        corpus_id=CORPUS + ('-custom-' + data_sha if external else ''), role='control' if r == 0 else 'candidate',
        data_role='development_probe', holdout_only=False, actuals_used_for_fitting=False,
        completion_gate='Independent literal byte/calldata/transient-state/floor-gas/signature/account/root/witness and current original quote gates before actuals.',
        scope='Finite development controls, not IID/holdout or a global/per-read PGU bound. Actual same-proposal SP1 PGU, node-known and full-proposal quotes remain separate; no fit or pricing change.',
        parameters=dict(key_count=key_count, catalog_id=catalog_id, catalog_source='external-data' if external else 'deterministic-recipe',
            workload_level=level, ordered_keys=list(keys), raw_key_sha256=data_sha, calldata_sha256=data_sha, query_key=f'0x{QUERY:064x}',
            repetitions=r, full_repetitions=maximum, unroll_factor=UNROLL, bulk_operations=UNROLL*r,
            seed_writes=key_count, expected_transient_reads=2+UNROLL*r, expected_transient_writes=key_count,
            initial_query_reads=1, seeded_query_reads=1, body_target_operations_per_user=UNROLL*r,
            target_query_reads=2+UNROLL*r, expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            initial_storage_entries=0, initial_state_size=0, aggregate_state_size=0, state_size_per_account=0,
            expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0,
            peak_transient_state_entries_per_transaction=key_count, expected_final_transient_entry_count=0,
            primary_code_owner_count=1, account_count=1, seed_counter_memory_offset=0, counter_memory_offset=64,
            protected_memory_offset=96, expected_counter=r, sentinel=SENTINEL, peak_memory_bytes=128, memory_words=4,
            outer_calldata_bytes=32*key_count, calldata_zero_bytes=24*key_count, calldata_nonzero_bytes=8*key_count,
            all_user_transactions=1, user_transactions_per_block=1, all_transactions_including_anchor=2, block_count=1, visits_per_account=1,
            transaction_kind='call', transaction_nonce=0, primary_code_owner=PRIMARY, recipient_schedule=[PRIMARY], transaction_gas_limit=envelope,
            transaction_envelope_reserve=1000, header_gas_limit=catalogs.HEADER_GAS_LIMIT, charged_gas_budget=catalogs.BLOCK_BUDGET,
            expected_anchor_gas=catalogs.ANCHOR_GAS, header_prefix_remaining_gas=[catalogs.HEADER_GAS_LIMIT-catalogs.ANCHOR_GAS],
            gas_accounting_per_user=gas, expected_user_gross_gas=gas['gross_gas'], expected_user_charged_gas=gas['charged_gas'],
            expected_user_refund_counter=0, expected_user_refund_applied=0, expected_block_charged_gas=catalogs.ANCHOR_GAS+gas['charged_gas'],
            near_full=catalogs.ANCHOR_GAS+gas['charged_gas'] >= 29_700_000,
            full_plus_one_constraint_failures=_constraint_failures(gas_accounting(maximum+1, key_count=key_count)),
            fullness_scope='Maximal uniform single-user full envelope within15M; an intermediate probe or ownzero is not a30M full-block certificate.',
            expected_terminal='STOP', expected_user_success=True, expected_user_receipt_statuses=[True], expected_anchor_success=True,
            expected_exceptional_instructions=0, execution_profile='evm-gas-only', preload_created_accounts_or_code=False,
            shared_runtime=True, runtime_bytes=len(code), runtime_sha256=digest(code),
            initial_storage_rule='Primary empty persistent state, no extra prestate code/accounts. Transient state starts empty and resets after the transaction.',
            range_rule='Guard QUERY0; direct TSTORE1 for each ordered calldata key, QUERY last; guard QUERY1;64 pure TLOAD/pop sites per R; guard counters/sentinel/query and STOP with empty stack.'), request=request)


def make_case(catalog='mixed', level='full', *, key_count=KEY_COUNT):
    return _make_case(catalogs._catalog(catalog, key_count=key_count), catalog, level, key_count=key_count)


def make_custom_case(keys, level='full', *, key_count=KEY_COUNT):
    keys = catalogs._validate_keys(keys, key_count=key_count); data = b''.join(bytes.fromhex(k[2:]) for k in keys)
    return _make_case(keys, 'custom-'+digest(data), level, key_count=key_count, external=True)


def make_probe_case(keys, repetitions, *, key_count=KEY_COUNT):
    keys = catalogs._validate_keys(keys, key_count=key_count); data = b''.join(bytes.fromhex(k[2:]) for k in keys)
    return _make_case(keys, 'custom-'+digest(data), 'probe', repetitions=repetitions, key_count=key_count, external=True)


def source_modules():
    return catalogs.source_modules() | {'cases/warm_read_catalogs.py': catalogs}


def helpers():
    return {f'transient_read_catalogs.{f.__name__}': f for f in (_runtime, gas_accounting, _constraint_failures, fits, full_repetitions, _make_case, make_case, make_custom_case, make_probe_case)} | {
        f'warm_read_catalogs.{f.__name__}': f for f in (catalogs._key_count, catalogs._validate_keys, catalogs._catalog, catalogs._read_catalog)} | {
        '_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer}


def prepare(output: Path, *, model_id, catalog='mixed', catalog_file=None, probe_repetitions=None, key_count=KEY_COUNT):
    catalogs._key_count(key_count)
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    if type(catalog) is not str or catalog not in catalogs.CATALOGS: raise ValueError('unknown catalog')
    if output.exists() or output.is_symlink(): raise FileExistsError(output)
    external = None
    if catalog_file is not None: keys, external = catalogs._read_catalog(catalog_file, key_count=key_count)
    else: keys = catalogs._catalog(catalog, key_count=key_count)
    if probe_repetitions is not None: original.integer(probe_repetitions, 1, full_repetitions(key_count=key_count)-1, 'probe repetitions')
    levels = ('zero', 'probe', 'full') if probe_repetitions is not None else LEVELS
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows=[]
    for level in levels:
        row = (_make_case(keys, 'custom-'+digest(b''.join(bytes.fromhex(k[2:]) for k in keys)), level,
                          repetitions=probe_repetitions if level == 'probe' else None, key_count=key_count, external=True)
               if external else _make_case(keys, catalog, level, repetitions=probe_repetitions if level == 'probe' else None, key_count=key_count))
        raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
        write(output/path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    write(output/'catalog.json', encoded(dict(schema_version=1, corpus_id=rows[0]['corpus_id'], frozen_model_id=model_id,
        purpose='Finite direct-write transient-map single-user read catalogs, ownzero/probe/full; no model fit.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name:digest(Path(module.__file__).read_bytes()) for name,module in source_modules().items()},
        helper_source_sha256={name:digest(inspect.getsource(fn).encode()) for name,fn in helpers().items()},
        external_catalog_json_sha256=external, holdout_only=False, actuals_used_for_fitting=False, levels=list(levels), users=1,
        native_observability='Status/gas/routes and canonical replay; no native per-key transient export. Actual optional profiles are separate diagnostics.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir',type=Path,required=True);parser.add_argument('--model-id',required=True)
    parser.add_argument('--catalog',choices=catalogs.CATALOGS,default='mixed');parser.add_argument('--catalog-file',type=Path)
    parser.add_argument('--probe-repetitions',type=int);parser.add_argument('--key-count',type=int,default=KEY_COUNT)
    args=parser.parse_args();print(json.dumps({'prepared':prepare(args.output_dir,model_id=args.model_id,catalog=args.catalog,catalog_file=args.catalog_file,probe_repetitions=args.probe_repetitions,key_count=args.key_count)}))
