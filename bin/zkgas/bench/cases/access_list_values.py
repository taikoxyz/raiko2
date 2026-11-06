#!/usr/bin/env python3
"""Generate 23 finite access-list preexecution probes with unchanged execution limits."""
import argparse
import json
from pathlib import Path

from spawn_memory import digest, encoded, write

RUNTIME = bytes.fromhex('60006000f3')
PATTERNS = ('repeated_address', 'distinct_addresses', 'repeated_key', 'distinct_keys',
            'repeated_items', 'reversed_distinct_keys')
MAX_COUNT = 8192


def expected_counts(pattern, count):
    """Independent expected raw and unique counts for one signed transaction."""
    if pattern is None:
        return dict(raw_address_items=0, raw_storage_keys=0, unique_addresses=0,
                    unique_storage_pairs=0)
    addresses = pattern in ('repeated_address', 'distinct_addresses', 'repeated_items')
    storage = pattern not in ('repeated_address', 'distinct_addresses')
    return dict(raw_address_items=count if addresses else 1,
                raw_storage_keys=count if storage else 0,
                unique_addresses=count if pattern == 'distinct_addresses' else 1,
                unique_storage_pairs=(count if pattern in ('distinct_keys', 'reversed_distinct_keys')
                                      else 1) if storage else 0)


def make_case(pattern=None, count=None, block_count=1):
    if type(block_count) is not int or not 1 <= block_count <= 32:
        raise ValueError('access-list block count must be in 1..32')
    if pattern is None:
        if count is not None:
            raise ValueError('empty reference omits the access-list descriptor')
    elif (type(pattern) is not str or pattern not in PATTERNS or type(count) is not int or
          not 1 <= count <= MAX_COUNT):
        raise ValueError('access-list pattern/count must be bounded and nonempty')
    program = {'code_hex': '0x'+RUNTIME.hex(), 'input_hex': '0x', 'block_count': block_count,
               'expected_route_counts': {'opcode:0xf3': block_count, 'opcode:0xfd': 0,
                   'opcode:0x00': 0, 'opcode:0xfa': 0, 'opcode:0xf1': 0, 'opcode:0xf2': 0,
                   'opcode:0xf4': 0, 'opcode:0xf0': 0, 'opcode:0xf5': 0}}
    if pattern is not None:
        program['access_list'] = dict(pattern=pattern, count=count)
    shape = 'empty' if pattern is None else f'{pattern}-k{count}'
    sid = f'access-list-values-{shape}-b{block_count}'
    group = 'addresses' if pattern in ('repeated_address', 'distinct_addresses') else 'storage'
    if pattern is None or block_count != 1:
        baseline = f'access-list-values-empty-b{block_count}'
    else:
        base_pattern = 'repeated_address' if group == 'addresses' else 'repeated_key'
        baseline = f'access-list-values-{base_pattern}-k1-b1'
    return {'sample_id': sid, 'family': 'access-list-preexecution',
            'role': 'control' if pattern is None or count == 1 else 'candidate',
            'pair_group': f'access-list-{group}-b{block_count}',
            'baseline_sample_id': baseline,
            'value_boundary_probe_only': True,
            'parameters': {'transaction_type': 'eip1559', 'pattern': pattern, 'count': count,
                'block_count': block_count, 'outer_calldata_bytes': 0,
                'transaction_gas_limit': 15_000_000,
                'expected_access_list_counts_per_block': expected_counts(pattern, count),
                'extra_addresses': 'deterministic initially absent addresses; index zero is PROGRAM',
                'storage_preloads': False},
            'completion_gate': 'canonical signed type-2 inclusion with exact raw access-list order; '
                'native raw/unique counts and successful root RETURN in every block',
            'scope': 'finite preexecution raw-vector probe; shared canonical count-one baselines; '
                'same-block-count comparisons only; no new gas limit or tariff',
            'request': {'schema_version': 1, 'expected_user_success': True, 'program': program}}



def make_raw_case(keys, block_count=1):
    """One PROGRAM address with an exact raw U256 vector; no sorting or deduplication."""
    if type(keys) not in (list, tuple) or not 1 <= len(keys) <= MAX_COUNT:
        raise ValueError('raw access-list keys must be a list/tuple of 1..8192 entries')
    if any(type(key) is not int or not 0 <= key < (1 << 256) for key in keys):
        raise ValueError('raw access-list keys must be strict U256 integers')
    row = make_case('repeated_key', len(keys), block_count)
    raw = b''.join(key.to_bytes(32, 'big') for key in keys)
    key_digest = digest(raw)
    descriptor = dict(pattern='raw_keys', count=len(keys),
                      storage_keys=[f'0x{key:064x}' for key in keys])
    row['request']['program']['access_list'] = descriptor
    row.update(sample_id=f'access-list-raw-keys-k{len(keys)}-{key_digest}-b{block_count}',
               corpus_id=f'unzen-access-list-raw-keys-{key_digest}',
               pair_group=f'access-list-raw-storage-k{len(keys)}-b{block_count}',
               data_role='development_probe', actuals_used_for_fitting=False, holdout_only=False,
               scope='Explicit raw access-list input; order and duplicates preserved. Generation '
                     'does not establish guest hash layout, actual workload or pricing accuracy.')
    row['parameters'].update(pattern='raw_keys',
        expected_access_list_counts_per_block=dict(raw_address_items=1, raw_storage_keys=len(keys),
            unique_addresses=1, unique_storage_pairs=len(set(keys))),
        extra_addresses='one PROGRAM address only', ordered_storage_keys_sha256=key_digest,
        storage_key_encoding='32-byte big-endian concatenation in raw order')
    return row


def make_raw_addresses_case(addresses, block_count=1):
    """Exact empty-key owner items; order and duplicates survive canonical signing."""
    if type(addresses) not in (list, tuple) or not 1 <= len(addresses) <= MAX_COUNT:
        raise ValueError('raw access-list addresses must be a list/tuple of 1..8192 entries')
    if any(type(address) is not str or len(address) != 42 or
           not address.startswith('0x') or
           any(char not in '0123456789abcdefABCDEF' for char in address[2:])
           for address in addresses):
        raise ValueError('raw access-list addresses must be exact 20-byte 0x-prefixed hex strings')
    ordered = [address.lower() for address in addresses]
    row = make_case('repeated_address', len(ordered), block_count)
    address_digest = digest(b''.join(bytes.fromhex(address[2:]) for address in ordered))
    row['request']['program']['access_list'] = dict(
        pattern='raw_addresses', count=len(ordered), addresses=ordered)
    row.update(sample_id=f'access-list-raw-addresses-k{len(ordered)}-{address_digest}-b{block_count}',
               corpus_id=f'unzen-access-list-raw-addresses-{address_digest}',
               pair_group=f'access-list-raw-addresses-k{len(ordered)}-b{block_count}',
               data_role='development_probe', actuals_used_for_fitting=False, holdout_only=False,
               scope='Explicit raw empty-key access-list owners; order and duplicates preserved. '
                     'Generation does not establish guest hash layout, actual workload or pricing accuracy.')
    row['parameters'].update(pattern='raw_addresses',
        expected_access_list_counts_per_block=dict(raw_address_items=len(ordered), raw_storage_keys=0,
            unique_addresses=len(set(ordered)), unique_storage_pairs=0),
        extra_addresses='explicit owners; non-PROGRAM owners must be absent from initial prestate',
        ordered_addresses_sha256=address_digest,
        address_encoding='20-byte concatenation in raw order')
    return row


def prepare(output):
    selections = [(None, None, 1), ('repeated_address', 1, 1)]
    selections += [(pattern, count, 1) for pattern in ('repeated_address', 'distinct_addresses')
                   for count in (16, 256, 4096)]
    selections += [('repeated_key', 1, 1)]
    selections += [(pattern, count, 1) for pattern in ('repeated_key', 'distinct_keys')
                   for count in (16, 256, 4096)]
    selections += [('repeated_items', count, 1) for count in (16, 256, 2048)]
    selections += [('repeated_key', 2048, 1), ('reversed_distinct_keys', 4096, 1),
                   (None, None, 3), ('repeated_address', 4096, 3), ('repeated_key', 4096, 3)]
    plans = [make_case(*selection) for selection in selections]
    assert len(plans) == 23
    output.mkdir(parents=True, exist_ok=False)
    (output/'requests').mkdir()
    write(output/'runtime.hex', (RUNTIME.hex()+'\n').encode())
    rows = []
    for plan in plans:
        data = encoded(plan.pop('request'))
        relative = f"requests/{plan['sample_id']}.json"
        write(output/relative, data)
        rows.append(dict(plan, request_path=relative, request_sha256=digest(data)))
    write(output/'catalog.json', encoded({'schema_version': 1,
        'purpose': 'finite access-list preexecution raw/unique/group/order controls',
        'value_boundary_probe_only': True,
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'shared_generator_sha256': digest(Path(__file__).with_name('spawn_memory.py').read_bytes()),
        'runtime_sha256': digest(RUNTIME), 'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir)}))
