#!/usr/bin/env python3
"""Prepare finite absent-account calldata probes; pricing and native construction stay unchanged."""
import argparse
import hashlib
import inspect
import json
from pathlib import Path
import re

import initcode_preparation
from initcode_preparation import _Factory
import spawn_memory
import top_level_create

MODEL = '5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11'
CORPUS = 'unzen-account-read-catalogs-20261105'
QUERY = '0x1122334400000000000000000000000055667788'
PRIMARY = '0x' + '66' * 20
KEY_COUNT = 512
UNROLL = 64
SENTINEL = 0xa55a
CATALOG_FILE_CAP = 128 * 1024
MAX_REPETITIONS = 1_000_000
OPERATIONS = {'balance': 0x31, 'extcodesize': 0x3b}
RESERVED = frozenset((PRIMARY, '0x' + '88' * 20, '0x' + '99' * 20,
    '0x' + '33' * 20, '0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a',
    '0x1670000000000000000000000000000000010001',
    '0x0000777735367b36bc9b61c50022d9d0700db4ec'))


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode() + b'\n'


def _operation(operation):
    if type(operation) is not str or operation not in OPERATIONS:
        raise ValueError('operation must be balance or extcodesize')
    return OPERATIONS[operation]


def _integer(value, low, high, name):
    if type(value) is not int or not low <= value <= high:
        raise ValueError(name + ' is outside its integer domain')


def _validate_keys(keys):
    if type(keys) not in (list, tuple) or len(keys) != KEY_COUNT:
        raise ValueError('catalog requires exactly512 Address20 keys')
    for key in keys:
        if type(key) is not str or re.fullmatch('0x[0-9a-f]{40}', key) is None:
            raise ValueError('address requires canonical lowercase20-byte hex')
        if sum(b != 0 for b in bytes.fromhex(key[2:])) != 8:
            raise ValueError('address requires exactly eight nonzero bytes')
        if key in RESERVED or int(key, 16) <= 0xffff:
            raise ValueError('reserved/system/precompile address')
    if len(set(keys)) != KEY_COUNT or keys[-1] != QUERY:
        raise ValueError('addresses must be unique with the fixed query last')
    return tuple(keys)


def _catalog():
    """Deterministic mixed suffixes; raw common prefix is not a hash-pressure claim."""
    state = 0x243f6a8885a308d3; mask = (1 << 64) - 1; values = []; seen = set()
    while len(values) < KEY_COUNT - 1:
        state = (state + 0x9e3779b97f4a7c15) & mask; value = state
        value = ((value ^ (value >> 30)) * 0xbf58476d1ce4e5b9) & mask
        value = ((value ^ (value >> 27)) * 0x94d049bb133111eb) & mask
        suffix = ((value ^ (value >> 31)) & 0xffffffff).to_bytes(4, 'big')
        address = '0x11223344' + '00' * 12 + suffix.hex()
        if not all(suffix) or address == QUERY or address in seen: continue
        values.append(address); seen.add(address)
    return _validate_keys(values + [QUERY])


def _validate_sized_keys(keys):
    """Accept unique canonical Address20 data; final key is the query, never a seed guess."""
    if type(keys) not in (list, tuple) or not 1 <= len(keys) <= 1024:
        raise ValueError('sized catalog requires 1..1024 addresses')
    for key in keys:
        if type(key) is not str or re.fullmatch('0x[0-9a-f]{40}', key) is None:
            raise ValueError('address requires canonical lowercase20-byte hex')
        if key in RESERVED or int(key, 16) <= 0xffff:
            raise ValueError('reserved/system/precompile address')
    if len(set(keys)) != len(keys): raise ValueError('sized addresses must be unique')
    return tuple(keys)


def _runtime(operation, repetitions, *, key_count=KEY_COUNT, query=QUERY):
    opcode = _operation(operation); _integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    _integer(key_count, 1, 1024, 'key count'); _validate_sized_keys([query])
    n = key_count; seed_visits = n + 1
    asm = _Factory()
    def store(offset, value): asm.push(value); asm.push(offset); asm.op(0x52)
    def load(offset, times=1): asm.push(offset, times=times); asm.op(0x51, times)
    def guard(value, times=1):
        asm.push(value, 32, times); asm.op(0x14, times); asm.op(0x15, times)
        asm.target('failure', times); asm.op(0x57, times)
    asm.op(0x36); guard(32 * n)
    for offset, value in ((0, 0), (64, 0), (96, SENTINEL)): store(offset, value)
    asm.label('seed', seed_visits); load(0, seed_visits); asm.push(n, times=seed_visits); asm.op(0x14, seed_visits); asm.target('seed_done', seed_visits); asm.op(0x57, seed_visits)
    load(0, n); asm.push(5, times=n); asm.op(0x1b, n); asm.op(0x35, n); asm.op(opcode, n); guard(0, n)
    load(0, n); asm.push(1, times=n); asm.op(1, n); asm.push(0, times=n); asm.op(0x52, n); asm.target('seed', n); asm.op(0x56, n)
    asm.label('seed_done'); asm.push(32 * (n - 1)); asm.op(0x35); asm.op(0x80); asm.op(opcode); guard(0)
    visits = repetitions + 1; asm.label('loop', visits)
    load(64, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    for _ in range(UNROLL): asm.op(0x80, repetitions); asm.op(opcode, repetitions); asm.op(0x50, repetitions)
    load(64, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(64, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); asm.op(0x80); guard(int(query, 16)); asm.op(0x50)
    for offset, value in ((0, n), (64, repetitions), (96, SENTINEL)): load(offset); guard(value)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _ordinary_gas(counts):
    """Account for supported ordinary opcode multiplicities, excluding account access gas."""
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x1b: 3, 0x35: 3, 0x36: 2, 0x50: 2,
        0x51: 3, 0x52: 3, 0x31: 0, 0x3b: 0, 0x56: 8, 0x57: 10, 0x5b: 1, 0x80: 3, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= (op := int(key[-2:], 16)) <= 0x7f else costs[op]) for key, n in counts.items())
    return ordinary


def gas_accounting(operation, repetitions):
    code, counts = _runtime(operation, repetitions)
    ordinary = _ordinary_gas(counts)
    cold = 512 * 2600; warm = (64 * repetitions + 1) * 100
    gross = 135688 + ordinary + 12 + cold + warm; floor = 307720
    return dict(intrinsic_gas=135688, eip7623_floor_gas=floor, eip7623_floor_active=gross < floor,
        ordinary_opcode_gas=ordinary, memory_expansion_gas=12, cold_account_gas=cold, warm_account_gas=warm,
        cold_account_count=512, warm_account_count=64 * repetitions + 1,
        gross_gas=gross, charged_gas=max(gross, floor), refund_counter=0, refund_applied=0, runtime_bytes=len(code))


def _constraint_failures(operation, repetitions):
    gas = gas_accounting(operation, repetitions); envelope = gas['gross_gas'] + 1000
    return [name for name, fail in [('charged_block_budget', 167355 + gas['charged_gas'] > 30000000),
        ('transaction_fixture_cap', envelope > 15000000), ('eip7825_cap', envelope > 16777216),
        ('ordered_header_prefix', 31000000 - 167355 < envelope), ('runtime_size', gas['runtime_bytes'] > 24576)] if fail]


def full_repetitions(operation='balance'):
    _operation(operation); low, high = 0, MAX_REPETITIONS
    if _constraint_failures(operation, 0): raise ValueError('zero case cannot fit')
    while low < high:
        middle = (low + high + 1) // 2
        if not _constraint_failures(operation, middle): low = middle
        else: high = middle - 1
    return low


def make_case(operation='balance', level='probe', *, keys=None, repetitions=512):
    _operation(operation)
    if type(level) is not str or level not in ('zero', 'probe'): raise ValueError('level must be zero or probe')
    maximum = full_repetitions(operation)
    _integer(repetitions, 0 if level == 'zero' else 1, maximum - 1, 'probe repetitions')
    addresses = _catalog() if keys is None else _validate_keys(keys)
    r = 0 if level == 'zero' else repetitions; code, counts = _runtime(operation, r)
    data = b''.join(bytes(12) + bytes.fromhex(address[2:]) for address in addresses)
    data_sha = digest(data); catalog_id = ('default-mixed-long-prefix' if keys is None else 'custom') + '-' + data_sha
    gas = gas_accounting(operation, r)
    envelope = gas_accounting(operation, maximum)['gross_gas'] + 1000
    return _case_row(operation, level, addresses, r, code, counts, data, catalog_id, gas, envelope, maximum)


def _case_row(operation, level, addresses, r, code, counts, data, catalog_id, gas, envelope, maximum):
    """Build common read intent without imposing the legacy strict512 catalog validation."""
    n = len(addresses); prefix = 'account-read-' + operation + '-' + catalog_id
    data_sha = digest(data)
    request = dict(schema_version=1, expected_user_success=True, program=dict(block_count=1,
        code_hex='0x' + code.hex(), input_hex='0x' + data.hex(), expected_route_counts=dict(counts),
        transactions=dict(kind='call', count_per_block=1, gas_limit=envelope)))
    return dict(sample_id=prefix + f'-n{64 * r}', baseline_sample_id=prefix + '-n0', corpus_id=CORPUS + '-' + catalog_id,
        family='account-read-catalogs', role='control' if r == 0 else 'candidate', data_role='development_probe',
        holdout_only=False, actuals_used_for_fitting=False,
        completion_gate='Decoded-byte address/value/cold-warm/count/gas guards plus native signed-input/root/absence witness authentication before actuals.',
        scope='Finite one-user account-journal pilot; not IID, concentration realization, fitting, maximal-block, global-bound or production-SDK evidence.',
        parameters=dict(operation=operation, key_kind='address20', ordered_addresses=list(addresses), query_address=addresses[-1],
            catalog_id=catalog_id, catalog_source='deterministic-mixed-suffix-recipe' if catalog_id.startswith('default-') else 'external-data',
            repetitions=r, full_repetitions=maximum, workload_level=level, unroll_factor=64, bulk_operations=64 * r,
            seed_reads=n, query_reads=64 * r + 1, read_events=n + 1 + 64 * r, accessed_unique_addresses=n,
            expected_persistent_storage_reads=0, expected_persistent_storage_writes=0, target_balance=0, target_code_bytes=0,
            target_initial_state='ABSENT accounts; no target prestate code or storage is provisioned.',
            witness_policy='Canonical honest full tiny base-state trie; not an assertion of a globally minimal node subset.',
            all_user_transactions=1, all_transactions_including_anchor=2, block_count=1, primary_code_owner=PRIMARY,
            recipient_schedule=[PRIMARY], transaction_kind='call', transaction_nonce=0, transaction_gas_limit=envelope,
            transaction_envelope_reserve=1000, header_gas_limit=31000000, header_prefix_remaining_gas=[30832645],
            charged_gas_budget=30000000, expected_anchor_gas=167355, expected_block_charged_gas=gas['charged_gas'] + 167355,
            gas_accounting_per_user=gas, expected_user_gross_gas=gas['gross_gas'], expected_user_charged_gas=gas['charged_gas'],
            expected_user_refund_counter=0, expected_user_refund_applied=0, full_plus_one_constraint_failures=_constraint_failures(operation, maximum + 1) if maximum is not None else [],
            fullness_scope='Intermediate own-zero/probe in a uniform15M envelope; not a30M block capacity certificate.',
            near_full=False, expected_terminal='STOP', expected_user_success=True, expected_user_receipt_statuses=[True],
            expected_anchor_success=True, expected_exceptional_instructions=0, execution_profile='evm-gas-only',
            preload_created_accounts_or_code=False, shared_runtime=True, runtime_bytes=len(code), runtime_sha256=digest(code),
            outer_calldata_bytes=len(data), calldata_zero_bytes=data.count(0), calldata_nonzero_bytes=len(data) - data.count(0),
            calldata_sha256=data_sha, address_words_sha256=data_sha, seed_counter_memory_offset=0, counter_memory_offset=64,
            protected_memory_offset=96, expected_counter=r, sentinel=SENTINEL, peak_memory_bytes=128), request=request)


def make_sized_case(keys, *, operation='balance', repetitions=512, access_list=False):
    """One opt-in variable-sized CALL with literal calldata accounting and a fixed15M envelope.

    This does not select hash state or certify actual map size, placement or workload. Legacy
    make_case/prepare retain their exact strict512 catalog, envelope and byte contracts.
    """
    _operation(operation); _integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    if type(access_list) is not bool: raise ValueError('access_list must be boolean')
    addresses = _validate_sized_keys(keys); n = len(addresses); r = repetitions
    code, counts = _runtime(operation, r, key_count=n, query=addresses[-1])
    data = b''.join(bytes(12) + bytes.fromhex(address[2:]) for address in addresses)
    ordinary = _ordinary_gas(counts)
    al_intrinsic = n * 2400 if access_list else 0
    intrinsic = 21000 + data.count(0) * 4 + (len(data) - data.count(0)) * 16 + al_intrinsic
    cold_count = 0 if access_list else n; warm_count = 64 * r + 1 + (n if access_list else 0)
    gross = intrinsic + ordinary + 12 + cold_count * 2600 + warm_count * 100
    floor = 21000 + 10 * (data.count(0) + 4 * (len(data) - data.count(0)))
    charged = max(gross, floor); envelope = 15000000
    if (charged > envelope or envelope > 16777216 or 167355 + charged > 30000000 or
            envelope > 31000000 - 167355 or len(code) > 24576 or len(data) > 32768):
        raise ValueError('sized case exceeds transaction/block/code/calldata limits')
    gas = dict(intrinsic_gas=intrinsic, access_list_intrinsic_gas=al_intrinsic,
        eip7623_floor_gas=floor, eip7623_floor_active=gross < floor, ordinary_opcode_gas=ordinary,
        memory_expansion_gas=12, cold_account_gas=cold_count * 2600, warm_account_gas=warm_count * 100,
        cold_account_count=cold_count, warm_account_count=warm_count, gross_gas=gross,
        charged_gas=charged, refund_counter=0, refund_applied=0, runtime_bytes=len(code))
    catalog_id = f'sized-raw-k{n}-al{int(access_list)}-' + digest(data)
    row = _case_row(operation, 'zero' if r == 0 else 'probe', addresses, r, code, counts,
        data, catalog_id, gas, envelope, None)
    row['scope'] = 'Variable-sized absent-account development probe; own map state and actual workload unqualified. No fitting, full-block, holdout, global-bound or production-SDK claim.'
    p = row['parameters']; p.update(sized_account_catalog=True, key_count=n, access_list=access_list,
        transaction_envelope_reserve=None, catalog_source='explicit-raw-addresses',
        fullness_scope='Fixed15M envelope with one user; not a maximum or full30M block certificate.')
    for name in ('full_repetitions', 'full_plus_one_constraint_failures'): del p[name]
    if access_list:
        row['request']['program']['access_list'] = dict(pattern='raw_addresses', count=n, addresses=list(addresses))
    return row


def source_modules():
    return {'cases/initcode_preparation.py': initcode_preparation,
        'cases/spawn_memory.py': spawn_memory, 'cases/top_level_create.py': top_level_create}


def helpers():
    return {'initcode_preparation._Factory': _Factory, **{'account_read_catalogs.' + fn.__name__: fn for fn in
        (digest, encoded, _operation, _integer, _validate_keys, _catalog, _runtime, gas_accounting,
         _constraint_failures, full_repetitions, make_case, _read_catalog, _validate_sized_keys,
         _ordinary_gas, _case_row, make_sized_case)}}


def _read_catalog(path):
    if not isinstance(path, Path): raise ValueError('catalog_file must be a Path')
    with path.open('rb') as source: raw = source.read(CATALOG_FILE_CAP + 1)
    if len(raw) > CATALOG_FILE_CAP: raise ValueError('catalog exceeds128KiB')
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result: raise ValueError('duplicate JSON field')
            result[key] = value
        return result
    obj = json.loads(raw, object_pairs_hook=unique)
    if type(obj) is not dict or set(obj) != {'schema_version', 'keys'} or type(obj['schema_version']) is not int or obj['schema_version'] != 1:
        raise ValueError('catalog requires exactly schema_version1 and keys')
    return _validate_keys(obj['keys']), digest(raw)


def prepare(output: Path, *, model_id=MODEL, catalog_file=None, probe_repetitions=512):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    if output.exists() or output.is_symlink(): raise FileExistsError(output)
    _integer(probe_repetitions, 1, full_repetitions() - 1, 'probe repetitions')
    keys, external = (None, None) if catalog_file is None else _read_catalog(catalog_file)
    rows = [make_case(level=level, keys=keys, repetitions=probe_repetitions) for level in ('zero', 'probe')]
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir()
    for row in rows:
        raw = encoded(row.pop('request')); path = 'requests/' + row['sample_id'] + '.json'
        with (output/path).open('xb') as target: target.write(raw)
        row.update(request_path=path, request_sha256=digest(raw))
    catalog = dict(schema_version=1, corpus_id=rows[0]['corpus_id'], frozen_model_id=model_id, key_kind='address20',
        purpose='Opened own-zero/probe absent-account journal workload experiment; no fitting or concentration claim.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(fn).encode()) for name, fn in helpers().items()},
        external_catalog_json_sha256=external, external_catalog_source_sha256=external,
        levels=['zero', 'probe'], holdout_only=False, actuals_used_for_fitting=False, users=1,
        native_observability='Status/routes/receipt gas and canonical full replay; backing absence proofs require separate authentication, not declared zero values.', cases=rows)
    with (output/'catalog.json').open('xb') as target: target.write(encoded(catalog))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    parser.add_argument('--catalog-file', type=Path); parser.add_argument('--probe-repetitions', type=int, default=512)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id, catalog_file=args.catalog_file, probe_repetitions=args.probe_repetitions)}))
