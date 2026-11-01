#!/usr/bin/env python3
"""Generate26 honest minimal-multiproof warm/present/absent storage growth probes."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from spawn_memory import digest, encoded, write
import top_level_create as original

MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
MODES = ('warm', 'present', 'absent')
STATE_SIZES = (8192, 16384)
READ_COUNTS = (0, 64, 1024, 4096)


def _check(asm, expected):
    asm.push(expected); asm.op(0x14); asm.op(0x15); asm.target('failure'); asm.op(0x57)


def make_case(mode='present', state_size=8192, reads=1024):
    if type(mode) is not str or mode not in MODES: raise ValueError('unknown storage access mode')
    if type(state_size) is not int or state_size not in STATE_SIZES: raise ValueError('state_size must be8192 or16384')
    original.integer(reads, 0, 65536 if mode == 'warm' else 4096, 'bulk reads')
    if reads % 64: raise ValueError('bulk reads must be divisible by64')
    base, stride = {'warm': (0, 0), 'present': (1, 1), 'absent': (1048577, 1)}[mode]
    checksum = 0 if mode == 'absent' else reads
    batches = reads//64; asm = _Factory()
    # Warmup is outside the checksum and always requires the authenticated value1.
    asm.push(0); asm.op(0x54); _check(asm, 1)
    for offset in (0, 32): asm.push(0); asm.push(offset); asm.op(0x52)
    asm.label('loop', batches+1); asm.push(0, times=batches+1); asm.op(0x51, batches+1)
    asm.push(reads, times=batches+1); asm.op(0x14, batches+1); asm.target('done', batches+1); asm.op(0x57, batches+1)
    # One fixed64-site body for all modes and bounds. Every site derives its key
    # from the current concrete i, then accumulates the actual SLOAD result.
    for _ in range(64):
        asm.push(0, times=batches); asm.op(0x51, batches); asm.push(stride, times=batches); asm.op(2, batches)
        asm.push(base, times=batches); asm.op(1, batches); asm.op(0x54, batches)
        asm.push(32, times=batches); asm.op(0x51, batches); asm.op(1, batches); asm.push(32, times=batches); asm.op(0x52, batches)
        asm.push(0, times=batches); asm.op(0x51, batches); asm.push(1, times=batches); asm.op(1, batches); asm.push(0, times=batches); asm.op(0x52, batches)
    asm.target('loop', batches); asm.op(0x56, batches); asm.label('done')
    asm.push(0); asm.op(0x51); _check(asm, reads)
    asm.push(32); asm.op(0x51); _check(asm, checksum); asm.op(0)
    asm.label('failure', 0); asm.op(0xfe, 0); code = asm.finish()
    keys = list(dict.fromkeys([0]+[base+stride*i for i in range(reads)]))
    row = original.make_case('call', 'stop', gas_limit=15_000_000)
    program = row['request']['program']; program.pop('transactions')
    routes = {key: 0 for key in program['expected_route_counts']}; routes.update(asm.counts)
    program.update(code_hex='0x'+code.hex(), expected_route_counts=routes, storage_state={
        'initial_storage': {hex(key): '0x1' for key in range(state_size)},
        'proof_slots': [hex(key) for key in keys], 'insert_slots': [], 'delete_slots': []})
    sid = f'storage-access-{mode}-s{state_size}-n{reads}'
    row.update(sample_id=sid, family='storage-access-growth', role='control' if reads == 0 else 'candidate',
        baseline_sample_id=f'storage-access-{mode}-s{state_size}-n0',
        state_size_reference_sample_id=f'storage-access-{mode}-s8192-n{reads}',
        data_role='unfitted_storage_growth_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Concrete ordered raw SLOAD keys/values, warm/cold history, loop counter, memory checksum, exact proof targets, authenticated prestate roots, unchanged storage, opcode counts and successful receipts; independent resource observations required.',
        scope='Finite honest minimal-target multiproof and state-size growth under frozen pricing. Retain complete proposal estimate, node per-block estimate and literal sum, same-proposal actual SP1 PGU, both deficit flags and all own-N0 controls. Trie reconstruction occurs once; no repeated-materialization, universal depth bound, fitted price, per-block actual or capacity claim.')
    row['parameters'] = {'mode': mode, 'state_size': state_size, 'reads': reads, 'unroll_factor': 64, 'batch_count': batches,
        'warmup_reads': 1, 'all_reads_per_proposal': reads+1, 'bulk_key_base': base, 'bulk_key_stride': stride,
        'bulk_expected_value': 0 if mode == 'absent' else 1, 'warmup_key': 0, 'warmup_expected_value': 1,
        'expected_counter': reads, 'expected_checksum': checksum, 'counter_memory_offset': 0, 'checksum_memory_offset': 32,
        'expected_cold_reads': 1 if mode == 'warm' else reads+1, 'expected_warm_reads': reads if mode == 'warm' else 0,
        'expected_distinct_storage_keys': len(keys), 'proof_slot_count': len(keys),
        'initial_storage_rule': 'Raw logical slots0throughstate_size-1 each contain1; all other slots absent.',
        'witness_policy': 'Canonical builder target storage multiproof with complete system storage; no padding or hashed-key injection.',
        'expected_storage_unchanged': True, 'insert_slot_count': 0, 'delete_slot_count': 0,
        'parameter_scope': 'One block, one default EIP-1559 user CALL and its ordinary Anchor; counts aggregate the user execution. Bulk reads exclude once-only warmup.',
        'transaction_kind': 'call', 'transaction_gas_limit': 15_000_000, 'transaction_envelope': 'EIP-1559 default constructor; explicit transactions descriptor omitted for storage_state',
        'block_count': 1, 'user_transactions_per_block': 1, 'all_transactions_including_anchor': 2,
        'execution_profile': 'evm-gas-only', 'outer_calldata_bytes': 0, 'expected_user_success': True,
        'expected_exceptional_instructions': 0, 'storage_writes': 0, 'preload_created_accounts_or_code': False,
        'runtime_bytes': len(code), 'runtime_sha256': digest(code)}
    return row


def prepare(output, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    plans = [make_case(mode, size, n) for mode in MODES for size in STATE_SIZES for n in READ_COUNTS]
    plans += [make_case('warm', size, 65536) for size in STATE_SIZES]
    ids = {row['sample_id'] for row in plans}; assert len(plans) == len(ids) == 26
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows = []
    for row in plans:
        assert row['baseline_sample_id'] in ids and row['state_size_reference_sample_id'] in ids
        raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"; write(output/path, raw)
        rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    write(output/'catalog.json', encoded({'schema_version': 1, 'frozen_model_id': model_id,
        'purpose': 'Twenty-six minimal-multiproof storage read growth inputs with fixed warmup, warm/present/absent bulk reads and two authenticated prestate sizes',
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'source_sha256': {name: digest(Path(__file__).with_name(name).read_bytes()) for name in ('initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        'required_comparison_quantities': ['complete proposal estimate', 'node per-block estimates plus their literal sum and aggregate ceiling', 'actual same-proposal SP1 PGU'],
        'interpretation': 'Frozen modelc893; all inputs and own-N0 controls retained with both deficit flags and failures. Warm bulk repeats slot0; present/absent bulk visits distinct raw keys. Minimal target witnesses reflect real background state. Compare within mode/size growth and matched state-size effects. No proposal allocation to mining, per-block actual attribution, refit or universal workload claim.',
        'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL); args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
