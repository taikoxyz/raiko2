#!/usr/bin/env python3
"""Generate45 frozen-model storage/hash, frame rollback and equivalent-packing probes."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import top_level_create as original

MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
KERNELS = ('dirty', 'hash', 'mixed')
LAYOUTS = ('direct', 'return', 'revert')
ZERO_HASH = int('290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563', 16)
TAG = int.from_bytes(b'storage-hash-child-v1'.ljust(32, b'\0'), 'big')


def _check(asm, value, *, width=3):
    """Consume one stack value and reject a semantic mismatch through the unreachable INVALID."""
    asm.push(value, width); asm.op(0x14); asm.op(0x15); asm.target('failure'); asm.op(0x57)


def _load_slot(asm, expected):
    asm.push(2); asm.op(0x54); _check(asm, expected)


def _store(asm, value, *, times=1):
    asm.push(value, times=times); asm.push(2, times=times); asm.op(0x55, times)


def _hash(asm, *, times=1):
    asm.push(32, times=times); asm.push(0, times=times); asm.op(0x20, times)


def _kernel(asm, kind, rounds):
    """Run one guarded5→7→{7,11}→5 kernel; original transaction storage remains zero."""
    batches = rounds//64
    _load_slot(asm, 5); _store(asm, 7)
    asm.push(0); asm.push(64); asm.op(0x52)
    _hash(asm); _check(asm, ZERO_HASH, width=32)
    asm.label('loop', batches+1); asm.push(64, times=batches+1); asm.op(0x51, batches+1)
    asm.push(batches, times=batches+1); asm.op(0x14, batches+1); asm.target('done', batches+1); asm.op(0x57, batches+1)
    for _ in range(64):
        if kind in ('dirty', 'mixed'): _store(asm, 11, times=batches)
        if kind in ('hash', 'mixed'): _hash(asm, times=batches); asm.op(0x50, batches)
        if kind in ('dirty', 'mixed'): _store(asm, 7, times=batches)
    asm.push(64, times=batches); asm.op(0x51, batches); asm.push(1, times=batches); asm.op(1, batches)
    asm.push(64, times=batches); asm.op(0x52, batches); asm.target('loop', batches); asm.op(0x56, batches)
    asm.label('done'); _load_slot(asm, 7); _store(asm, 5)
    # Derive the marker from the completed loop counter; a shortened body cannot claim N.
    asm.push(64); asm.op(0x51); asm.push(64); asm.op(2); asm.push(128); asm.op(0x52)


def _cleanup(asm, rounds):
    asm.push(128); asm.op(0x51); _check(asm, rounds)
    _load_slot(asm, 5); _store(asm, 0); _load_slot(asm, 0); asm.op(0)


def make_case(kernel='dirty', layout='direct', rounds=2048, transactions=1, blocks=1):
    if type(kernel) is not str or kernel not in KERNELS: raise ValueError('unknown storage/hash kernel')
    if type(layout) is not str or layout not in LAYOUTS: raise ValueError('unknown frame layout')
    original.integer(rounds, 0, 32768, 'rounds per user')
    if rounds % 64: raise ValueError('rounds must be divisible by64')
    original.integer(transactions, 1, 16, 'transactions per block'); original.integer(blocks, 1, 4, 'blocks')
    asm = _Factory()
    if layout != 'direct':
        asm.op(0x36, 2); asm.op(0x15, 2); asm.target('parent', 2); asm.op(0x57, 2)
        asm.op(0x36); _check(asm, 32); asm.push(0); asm.op(0x35); _check(asm, TAG, width=32)
        asm.target('kernel'); asm.op(0x56); asm.label('parent')
    _load_slot(asm, 0); _store(asm, 5)
    if layout == 'direct':
        _kernel(asm, kernel, rounds); _cleanup(asm, rounds)
    else:
        # Preallocate the entire parent image, then clear the separate output buffer.
        asm.push(TAG, 32); asm.push(256); asm.op(0x52)
        asm.push(0); asm.push(128); asm.op(0x52)
        asm.push(32); asm.push(128); asm.push(32); asm.push(256); asm.op(0x30); asm.push(12_000_000); asm.op(0xf4)
        _check(asm, int(layout == 'return'))
        asm.op(0x3d); _check(asm, 32); _cleanup(asm, rounds)
        asm.label('kernel'); _kernel(asm, kernel, rounds)
        asm.push(32); asm.push(128); asm.op(0xf3 if layout == 'return' else 0xfd)
    asm.label('failure', 0); asm.op(0xfe, 0)
    code = asm.finish(); users = transactions*blocks
    row = original.make_case('call', 'stop', count=transactions, block_count=blocks, gas_limit=15_000_000)
    routes = {key: 0 for key in row['request']['program']['expected_route_counts']}
    routes.update({key: count*users for key, count in asm.counts.items()})
    row['request']['program'].update(code_hex='0x'+code.hex(), expected_route_counts=routes)
    sid = f'storage-hash-{kernel}-{layout}-n{rounds}-t{transactions}-b{blocks}'
    row.update(sample_id=sid, family='storage-hash-composition', role='control' if rounds == 0 else 'candidate',
        baseline_sample_id=f'storage-hash-{kernel}-{layout}-n0-t{transactions}-b{blocks}',
        data_role='unfitted_composition_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='actual slot2 original0/current5→7↔11→5→0, zero-word EthereumKeccak, loop-derived marker, child status/returndata and caught rollback, all-user opcode maps, successful outer receipts and ordered canonical transactions',
        scope='Finite warm dirty-journal/hash composition under frozen pricing. Keep complete proposal estimate, node per-block estimates plus literal sum and aggregate ceiling, and actual same-proposal SP1PGU with both deficit flags and all controls. Own-N0 increments explain composition only. No cold-first-access warm-set claim, per-block actual attribution, price refit, capacity or proposal allocation.')
    row['parameters'] = {'kernel': kernel, 'layout': layout, 'rounds_per_user': rounds, 'transactions_per_block': transactions,
        'block_count': blocks, 'all_user_transactions': users, 'all_transactions_including_anchor': (transactions+1)*blocks,
        'all_rounds_per_block': rounds*transactions, 'all_rounds_per_proposal': rounds*users, 'unroll_factor': 64,
        'batch_count_per_user': rounds//64, 'parameter_scope': 'Kernel, marker and guard values describe one user execution; expected_route_counts are proposal aggregates.',
        'storage_slot': 2, 'original_storage_per_transaction': 0, 'parent_seed': 5, 'kernel_entry': 5, 'dirty_values': [7, 11],
        'kernel_exit': 5, 'parent_final_storage': 0, 'all_other_storage_expected_zero': True,
        'storage_scope': 'Parent prewarms slot2 and self target. This panel tests warm dirty journal work and rollback, not child cold-first-access warm-set rollback.',
        'hash_offset': 0, 'hash_length': 32, 'hash_input_hex': '0x'+bytes(32).hex(), 'hash_expected_hex': f'0x{ZERO_HASH:064x}',
        'hash_warmup_events_per_user': 1, 'counter_memory_offset': 64, 'marker_memory_offset': 128, 'input_tag_memory_offset': 256,
        'child_input_tag_hex': f'0x{TAG:064x}', 'forwarded_gas': 12_000_000 if layout != 'direct' else None,
        'expected_child_status': None if layout == 'direct' else int(layout == 'return'),
        'expected_child_return_bytes': 0 if layout == 'direct' else 32, 'expected_marker_per_user': rounds,
        'expected_user_success': True, 'expected_exceptional_instructions': 0, 'expected_code_preparation': {'nonempty_events': 0, 'total_bytes': 0},
        'expected_program_nonce': 0, 'transaction_kind': 'call', 'transaction_gas_limit': 15_000_000, 'outer_calldata_bytes': 0,
        'code_location': 'prestate runtime', 'runtime_bytes': len(code), 'runtime_sha256': digest(code), 'preload_created_accounts_or_code': False}
    if transactions == 1 and blocks == 1 and rounds > 2048:
        row['repetition_reference_sample_id'] = f'storage-hash-{kernel}-{layout}-n2048-t1-b1'
    if kernel == 'mixed' and blocks == 1 and transactions > 1 and rounds:
        row['equivalent_work_reference_sample_id'] = f'storage-hash-mixed-{layout}-n32768-t1-b1'
    if blocks > 1:
        row['single_block_reference_sample_id'] = f'storage-hash-{kernel}-{layout}-n{rounds}-t{transactions}-b1'
    return row


def prepare(output, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    plans = [make_case(k, l, n) for k in KERNELS for l in LAYOUTS for n in (0, 2048, 32768)]
    plans += [make_case('mixed', l, n, t) for l in LAYOUTS for t, ns in ((4, (0, 8192)), (16, (0, 2048))) for n in ns]
    plans += [make_case('mixed', l, n, 1, 4) for l in LAYOUTS for n in (0, 32768)]
    ids = {r['sample_id'] for r in plans}; assert len(plans) == len(ids) == 45
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows = []
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"; write(output/path, raw)
        rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    write(output/'catalog.json', encoded({'schema_version': 1, 'frozen_model_id': model_id,
        'purpose': 'Forty-five warm dirty-storage/hash composition, caught rollback, equivalent packing and four-block validation inputs; no fitted tariff change',
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'source_sha256': {name: digest(Path(__file__).with_name(name).read_bytes()) for name in ('initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        'required_comparison_quantities': ['complete proposal estimate', 'node per-block estimates plus their literal sum and aggregate ceiling', 'actual same-proposal SP1 PGU'],
        'interpretation': '45new validation inputs under frozen modelc893. Keep failures and both underpricing flags; compare matching user/block/layout N0 controls. Revert also executes every dirty write/hash. Hash memory, counter, return marker and input tag are disjoint. No input padding, custom storage prestate, isolated opcode fit, cold-first-access coverage, per-block actual or capacity claim.',
        'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL); args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
