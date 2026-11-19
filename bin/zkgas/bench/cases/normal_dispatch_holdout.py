#!/usr/bin/env python3
"""Generate24 prospective dispatch-reserve probes without changing the fitted development panel."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from stack_density import SEEDS
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
PATTERNS = ('dup16-pop', 'swap16-swap16', 'xor', 'shl255', 'sar255',
            'callvalue-pop', 'timestamp-pop', 'number-pop')
# The frozen native constructor uses block1166000 at mainnet Unzen timestamp+1.
# Qualification must compare these declared values with the actual witness header.
DECLARED_CONTEXT = {'callvalue-pop': 0, 'timestamp-pop': 1786021201, 'number-pop': 1166000}
MASK = (1 << 256) - 1


def _operation(asm, pattern, times):
    """Emit one balanced body site, consuming only temporary copies of the17 seed words."""
    if pattern == 'dup16-pop': asm.op(0x8f, times)
    elif pattern == 'swap16-swap16': asm.op(0x9f, times); asm.op(0x9f, times); return
    elif pattern == 'xor': asm.op(0x81, times); asm.op(0x81, times); asm.op(0x18, times)
    elif pattern in ('shl255', 'sar255'):
        asm.op(0x80, times); asm.push(255, 1, times); asm.op(0x1b if pattern == 'shl255' else 0x1d, times)
    else: asm.op({'callvalue-pop': 0x34, 'timestamp-pop': 0x42, 'number-pop': 0x43}[pattern], times)
    asm.op(0x50, times)


def _warmup(asm, pattern):
    """Check arithmetic/context output once; bulk sites intentionally omit per-event guards."""
    if pattern in ('dup16-pop', 'swap16-swap16'): return None
    if pattern == 'xor':
        asm.op(0x81); asm.op(0x81); asm.op(0x18); expected = SEEDS[-1] ^ SEEDS[-2]
    elif pattern in ('shl255', 'sar255'):
        asm.op(0x80); asm.push(255, 1); asm.op(0x1b if pattern == 'shl255' else 0x1d)
        signed = SEEDS[-1] - (1 << 256) if SEEDS[-1] & (1 << 255) else SEEDS[-1]
        expected = ((SEEDS[-1] << 255) if pattern == 'shl255' else (signed >> 255)) & MASK
    else:
        asm.op({'callvalue-pop': 0x34, 'timestamp-pop': 0x42, 'number-pop': 0x43}[pattern]); expected = DECLARED_CONTEXT[pattern]
    asm.push(expected, 32); asm.op(0x14); asm.op(0x15); asm.target('failure'); asm.op(0x57)
    return expected


def make_case(pattern='dup16-pop', bulk_operations=8192):
    if type(pattern) is not str or pattern not in PATTERNS: raise ValueError('unknown dispatch holdout pattern')
    original.integer(bulk_operations, 0, 196608, 'bulk operations')
    if bulk_operations % 128: raise ValueError('bulk operations must be divisible by128')
    batches = bulk_operations // 128; asm = _Factory()
    asm.push(0); asm.push(0); asm.op(0x52)
    for value in SEEDS: asm.push(value, 32)
    expected = _warmup(asm, pattern)
    asm.label('loop', batches+1); asm.push(0, times=batches+1); asm.op(0x51, batches+1)
    asm.push(batches, times=batches+1); asm.op(0x14, batches+1); asm.target('done', batches+1); asm.op(0x57, batches+1)
    # All128 sites remain present at N0. The counter is absent from the body stack.
    for _ in range(128): _operation(asm, pattern, batches)
    asm.push(0, times=batches); asm.op(0x51, batches); asm.push(1, 1, batches); asm.op(1, batches)
    asm.push(0, times=batches); asm.op(0x52, batches); asm.target('loop', batches); asm.op(0x56, batches)
    asm.label('done')
    for value in reversed(SEEDS):
        asm.push(value, 32); asm.op(0x14); asm.op(0x15); asm.target('failure'); asm.op(0x57)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    code = asm.finish(); row = original.make_case('call', 'stop', gas_limit=15_000_000)
    routes = {key: 0 for key in row['request']['program']['expected_route_counts']}; routes.update(asm.counts)
    row['request']['program'].update(code_hex='0x'+code.hex(), expected_route_counts=routes)
    sid = f'normal-dispatch-holdout-{pattern}-n{bulk_operations}'
    row.update(sample_id=sid, family='normal-dispatch-prospective', role='control' if bulk_operations == 0 else 'candidate',
        baseline_sample_id=f'normal-dispatch-holdout-{pattern}-n0', data_role='prospective_validation_probe', holdout_only=True,
        completion_gate='independent actual operand/depth/signed-shift/context header and value oracles; exact opcode maps,17 ending seed checks, memory counter, successful receipt and no exceptions',
        scope='Twenty-four fresh validation inputs after64PGU reserve freeze; never fit from these results. Preserve complete proposal estimate, node per-block estimate plus sum, and actual same-proposal SP1PGU with independent underpricing flags including N0. Matched increments explain gaps only; no isolated opcode price, universal bound, capacity or proposal-to-mining allocation claim.')
    row['parameters'] = {'pattern': pattern, 'bulk_operations': bulk_operations, 'unroll_factor': 128, 'batch_count': batches,
        'body_stack_words': 17, 'stack_words_bottom_to_top': [f'0x{v:064x}' for v in SEEDS], 'ending_word_guards': 17,
        'counter_memory_offset': 0, 'counter_memory_bytes': 32, 'counter_present_on_body_stack': False,
        'expected_counter_final': batches, 'counter_ADD_events': batches, 'counter_MLOAD_events': 2*batches+1, 'counter_MSTORE_events': batches+1,
        'semantic_warmup_target_events': int(expected is not None), 'warmup_expected_result_hex': None if expected is None else f'0x{expected:064x}',
        'declared_context': dict(DECLARED_CONTEXT), 'context_verification': 'match value0 canonical signed user and actual witness block number/timestamp; do not trust producer metadata alone',
        'bulk_per_event_guard': False, 'expected_completed_user_opcodes': sum(routes.values()),
        'runtime_bytes': len(code), 'runtime_sha256': digest(code), 'expected_user_success': True, 'expected_exceptional_instructions': 0,
        'expected_code_preparation': {'nonempty_events': 0, 'total_bytes': 0}, 'expected_program_nonce': 0,
        'transaction_kind': 'call', 'count_per_block': 1, 'block_count': 1, 'transaction_gas_limit': 15_000_000,
        'outer_calldata_bytes': 0, 'code_location': 'prestate runtime', 'preload_created_accounts_or_code': False}
    if bulk_operations > 8192: row['repetition_reference_sample_id'] = f'normal-dispatch-holdout-{pattern}-n8192'
    return row


def prepare(output, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be a lowercase SHA256')
    plans = [make_case(pattern, n) for pattern in PATTERNS for n in (0, 8192, 196608)]
    ids = {r['sample_id'] for r in plans}; assert len(plans) == len(ids) == 24
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows = []
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"; write(output/path, raw)
        rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    write(output/'catalog.json', encoded({'schema_version': 1, 'frozen_model_id': model_id, 'unroll_factor': 128,
        'purpose': 'Prospective24-case normal-dispatch reserve validation; fixed128-site bodies and memory counter, all17 seeds checked',
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'source_sha256': {name: digest(Path(__file__).with_name(name).read_bytes()) for name in ('stack_density.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        'seed_derivation': 'Reuse the17 distinct full-width stack-density SHA256 seeds; exact words retained per case',
        'required_comparison_quantities': ['complete proposal zkGas estimate', 'node-observable estimate for each block and their sum', 'actual same-proposal SP1 PGU'],
        'interpretation': 'Holdout-only after64PGU reserve freeze. Preserve literal actual/node ranking, both underpricing flags and every control; matched increments are diagnostic. Model ID is metadata, not compiled price selection. No quiet price refit or earlier development-input change.',
        'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL); args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
