#!/usr/bin/env python3
"""Emit finite signed-calldata byte/content/count probes with one-byte STOP code."""
import argparse
import json
from pathlib import Path

from spawn_memory import digest, encoded, write
import top_level_create as original
from warm_read_catalogs import ANCHOR_GAS

LENGTHS = (0, 32, 4096, 65536)
PATTERNS = ('zero', 'sequence')
COUNTS = (1, 8)
MAX_LENGTH = 65536
MAX_COUNT = 8
TRANSACTION_GAS_LIMIT = 3_000_000


def make_case(length, pattern, count=1):
    original.integer(length, 0, MAX_LENGTH, 'calldata length')
    original.integer(count, 1, MAX_COUNT, 'count per block')
    if type(pattern) is not str or pattern not in PATTERNS:
        raise ValueError('pattern must be zero or sequence')
    if length == 0 and pattern != 'zero':
        raise ValueError('empty calldata uses the single zero-pattern control')
    payload = bytes(length) if pattern == 'zero' else bytes((73*i+19)%256 for i in range(length))
    zero = payload.count(0); nonzero = length-zero
    intrinsic = 21000+4*zero+16*nonzero
    floor = 21000+10*(zero+4*nonzero)
    charged = max(intrinsic, floor)
    row = original.make_case('call', 'stop', count=count, gas_limit=TRANSACTION_GAS_LIMIT)
    row['request']['program']['input_hex'] = '0x'+payload.hex()
    prefix = f'signed-payload-{pattern}-l{length}'
    row.update(sample_id=f'{prefix}-n{count}', family='signed-payload',
        role='control' if length == 0 else 'candidate',
        pair_group=f'signed-payload-n{count}', baseline_sample_id=f'signed-payload-zero-l0-n{count}',
        same_payload_single_user_reference_sample_id=f'{prefix}-n1',
        completion_gate='all canonical users included in nonce order with fixed3M envelope; successful receipts, exact user STOP count, exact EIP7623-charged gas and unchanged one-byte runtime',
        scope='Finite signed-calldata byte/content/count observations with constant STOP execution; canonical EIP2718 envelope bytes must be read from native/quote data, not inferred from calldata length')
    row['parameters'] = dict(length=length, pattern=pattern, count_per_block=count,
        calldata_bytes_per_user=length, calldata_zero_bytes_per_user=zero,
        calldata_nonzero_bytes_per_user=nonzero, all_user_calldata_bytes=length*count,
        all_user_calldata_zero_bytes=zero*count, all_user_calldata_nonzero_bytes=nonzero*count,
        payload_sha256=digest(payload), payload_pattern_formula='zero bytes' if pattern == 'zero' else '(73*i+19)%256 for i=0..length-1',
        expected_user_intrinsic_gas=intrinsic, expected_user_floor_gas=floor,
        expected_user_charged_gas=charged, expected_all_user_charged_gas=count*charged,
        expected_user_execution_gas=0, expected_user_refund_counter=0, expected_user_refund_applied=0,
        expected_anchor_gas=ANCHOR_GAS, expected_block_charged_gas=ANCHOR_GAS+count*charged,
        transaction_gas_limit=TRANSACTION_GAS_LIMIT, transaction_kind='call', transaction_type='eip1559',
        block_count=1, all_user_transactions=count, all_transactions_including_anchor=count+1,
        expected_user_nonces=list(range(count)), expected_user_receipt_statuses=[True]*count,
        expected_user_success=True, expected_terminal='STOP', expected_user_stop_count=count,
        runtime_bytes=1, runtime_sha256=digest(b'\x00'), execution_profile='evm-gas-only',
        expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
        expected_precompile_calls=0, expected_logs=0, expected_deployments=0,
        canonical_signed_envelope_bytes=None, calldata_bytes_equal_canonical_signed_envelope_bytes=False,
        canonical_signed_envelope_byte_source='PayloadAccumulator::record_transaction sums TransactionSigned::encode_2718_len() for anchor and user transactions; read canonical_transaction_bytes from native/quote selectors',
        signed_envelope_ownership='node transaction_payload selector owns canonical signed EIP2718 envelope bytes and nonempty payload transaction count; separate proposal completion remains distinct')
    return row


def prepare(output_dir):
    if not isinstance(output_dir, Path):
        raise ValueError('output_dir must be a Path')
    plans = [make_case(length, pattern, count) for length in LENGTHS for pattern in PATTERNS
             if length or pattern == 'zero' for count in COUNTS]
    ids = {row['sample_id'] for row in plans}
    if len(plans) != len(ids) or len(ids) != 14:
        raise ValueError('finite signed-payload catalog is inconsistent')
    output_dir.mkdir(parents=True, exist_ok=False); (output_dir/'requests').mkdir()
    rows = []
    for plan in plans:
        raw = encoded(plan.pop('request')); relative = f"requests/{plan['sample_id']}.json"
        write(output_dir/relative, raw)
        rows.append(plan | dict(request_path=relative, request_sha256=digest(raw)))
    write(output_dir/'catalog.json', encoded(dict(schema_version=1,
        purpose='Pure signed-calldata byte/content/count probes with constant one-byte STOP runtime',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes())
            for name in ('top_level_create.py', 'spawn_memory.py', 'warm_read_catalogs.py')},
        interpretation='Calldata length is a controlled physical input axis, not canonical signed-envelope byte length. Derive exact node payload selectors from the resulting native/quote artifacts.',
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    print(json.dumps({'prepared': prepare(parser.parse_args().output_dir)}))
