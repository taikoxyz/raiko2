#!/usr/bin/env python3
"""Generate20 matched density CALL blocks near30M charged gas without price refits."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
from stack_density import SEEDS
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL = 'c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef'
PATTERNS = {'jumpdest': (0x5b,), 'push0-pop': (0x5f, 0x50),
            'dup16-pop': (0x8f, 0x50), 'swap16-swap16': (0x9f, 0x9f),
            'jumpdest-dup16-pop': (0x5b, 0x8f, 0x50)}
UNROLL = 2048
TARGET_BLOCK_GAS = 29_990_000
NATIVE_HEADER_GAS_LIMIT = 31_000_000
# This value belongs to the frozen constructor, never to arbitrary Anchor calls.
# Native qualification must compare its actual first receipt before actual sampling.
FIXTURE_ANCHOR_GAS = 167355
GAS_LIMIT_MARGIN = 100_000
OPCODE_GAS = {0x5b: 1, 0x5f: 2, 0x50: 2, 0x8f: 3, 0x9f: 3}


def _program(pattern, batches):
    asm = _Factory()
    asm.push(0); asm.push(0); asm.op(0x52)
    for value in SEEDS: asm.push(value, 32)
    asm.label('loop', batches+1); asm.push(0, times=batches+1); asm.op(0x51, batches+1)
    asm.push(batches, times=batches+1); asm.op(0x14, batches+1); asm.target('done', batches+1); asm.op(0x57, batches+1)
    # Counter temporaries have been consumed: these sites see only17 distinct seeds.
    # N0 retains the identical body and immediate widths.
    for _ in range(UNROLL):
        for opcode in PATTERNS[pattern]: asm.op(opcode, batches)
    asm.push(0, times=batches); asm.op(0x51, batches); asm.push(1, 1, batches); asm.op(1, batches)
    asm.push(0, times=batches); asm.op(0x52, batches); asm.target('loop', batches); asm.op(0x56, batches)
    asm.label('done')
    # An explicit second check provides a final-counter failure gate separate
    # from the loop's branching condition and from the ending seed guards.
    asm.push(0); asm.op(0x51); asm.push(batches); asm.op(0x14); asm.op(0x15); asm.target('failure'); asm.op(0x57)
    for value in reversed(SEEDS):
        asm.push(value, 32); asm.op(0x14); asm.op(0x15); asm.target('failure'); asm.op(0x57)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), dict(asm.counts)


def make_case(pattern, packing=4, level='full'):
    if type(pattern) is not str or pattern not in PATTERNS: raise ValueError('unknown gas-budget density pattern')
    if type(packing) is not int or packing not in (4, 16): raise ValueError('packing must be4or16')
    if type(level) is not str or level not in ('zero', 'full'): raise ValueError('level must be zero or full')
    # Empty calldata CALL: intrinsic21000; fixed instructions489; first MSTORE
    # expands memory by one word for3. Each loop adds55 plus2048 body-site costs.
    fixed_user_gas = 21_000+489+3
    batch_gas = 55+UNROLL*sum(OPCODE_GAS[op] for op in PATTERNS[pattern])
    full_batches = (TARGET_BLOCK_GAS-FIXTURE_ANCHOR_GAS-packing*fixed_user_gas)//(packing*batch_gas)
    full_user_gas = fixed_user_gas+full_batches*batch_gas
    gas_limit = full_user_gas+GAS_LIMIT_MARGIN
    assert gas_limit <= min(15_000_000, NATIVE_HEADER_GAS_LIMIT-FIXTURE_ANCHOR_GAS-(packing-1)*full_user_gas)
    batches = full_batches if level == 'full' else 0
    code, user_routes = _program(pattern, batches)
    row = original.make_case('call', 'stop', count=packing, gas_limit=gas_limit)
    routes = {key: 0 for key in row['request']['program']['expected_route_counts']}
    routes.update({key: value*packing for key, value in user_routes.items()})
    row['request']['program'].update(code_hex='0x'+code.hex(), expected_route_counts=routes)
    user_gas = fixed_user_gas+batches*batch_gas
    block_gas = FIXTURE_ANCHOR_GAS+packing*user_gas
    sid = f'gas-budget-density-{pattern}-p{packing}-{level}'
    row.update(sample_id=sid, family='gas-budget-density', role='control' if level == 'zero' else 'candidate',
        baseline_sample_id=f'gas-budget-density-{pattern}-p{packing}-zero',
        data_role='prospective_validation_probe', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='native canonical signed users in exact order; each successful receipt and fixture Anchor gas; exact all-user opcode map; independently decoded fixed body and ordered actual operands; memory counter and all17 ending words; no exceptions',
        scope='Finite one-block density composition under frozen modelc893. Preserve complete proposal quote, node per-block quote and literal sum, and actual identical-proposal SP1PGU including ownN0. More than2B missingPGU within30M charged block gas is high priority. No isolated opcode fit, global bound, proof-time or capacity claim.')
    row['parameters'] = {'pattern': pattern, 'level': level, 'count_per_block': packing, 'block_count': 1,
        'unroll_factor': UNROLL, 'batch_count': batches, 'full_batch_count': full_batches,
        'bulk_operations_per_user': UNROLL*batches, 'bulk_operations_all_users': UNROLL*batches*packing,
        'body_opcode_hex': [f'0x{op:02x}' for op in PATTERNS[pattern]],
        'body_instruction_events_per_user': UNROLL*batches*len(PATTERNS[pattern]),
        'body_instruction_events_all_users': UNROLL*batches*len(PATTERNS[pattern])*packing,
        'body_stack_words': 17, 'stack_words_bottom_to_top': [f'0x{v:064x}' for v in SEEDS],
        'ending_word_guards': 17, 'counter_memory_offset': 0, 'counter_memory_bytes': 32,
        'counter_present_on_body_stack': False, 'expected_counter_final': batches, 'final_counter_guard': True,
        'counter_ADD_events_per_user': batches, 'counter_MLOAD_events_per_user': 2*batches+2,
        'counter_MSTORE_events_per_user': batches+1, 'expected_user_route_counts': {k: v for k, v in user_routes.items() if v},
        'expected_completed_user_opcodes': sum(user_routes.values()), 'expected_completed_all_user_opcodes': packing*sum(user_routes.values()),
        'expected_user_gas_used': user_gas, 'expected_user_gross_gas': user_gas,
        'expected_all_user_gas_used': packing*user_gas, 'expected_block_gas_used': block_gas,
        'expected_anchor_gas_used': FIXTURE_ANCHOR_GAS, 'anchor_gas_scope': 'frozen native constructor fixture; verify actual first receipt',
        'expected_intrinsic_gas_per_user': 21_000, 'expected_memory_expansion_gas': 3, 'expected_refund_gas': 0,
        'fixed_user_gas': fixed_user_gas, 'gas_per_batch': batch_gas, 'target_block_gas': TARGET_BLOCK_GAS,
        'minimum_full_block_gas': 29_700_000, 'maximum_charged_block_gas': 30_000_000,
        'expected_occupancy_fraction': block_gas/30_000_000,
        'transaction_gas_limit': gas_limit, 'gas_limit_margin_above_full_gross': GAS_LIMIT_MARGIN,
        'native_header_gas_limit': NATIVE_HEADER_GAS_LIMIT,
        'full_last_user_remaining_block_gas': NATIVE_HEADER_GAS_LIMIT-FIXTURE_ANCHOR_GAS-(packing-1)*full_user_gas,
        'runtime_bytes': len(code), 'runtime_sha256': digest(code), 'expected_user_success': True,
        'expected_exceptional_instructions': 0, 'expected_program_nonce': 0,
        'expected_code_preparation': {'nonempty_events': 0, 'total_bytes': 0},
        'transaction_kind': 'call', 'outer_calldata_bytes': 0, 'code_location': 'prestate runtime',
        'preload_created_accounts_or_code': False}
    return row


def prepare(output, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be a lowercase SHA256')
    plans = [make_case(pattern, packing, level) for pattern in PATTERNS for packing in (4, 16) for level in ('zero', 'full')]
    ids = {row['sample_id'] for row in plans}; assert len(ids) == 20
    output.mkdir(parents=True, exist_ok=False); (output/'requests').mkdir(); rows = []
    for row in plans:
        assert row['baseline_sample_id'] in ids
        raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"; write(output/path, raw)
        rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    write(output/'catalog.json', encoded({'schema_version': 1, 'frozen_model_id': model_id,
        'purpose': 'Twenty finite matched density blocks; four or16CALL users, fixed2048-site bodies, near30M charged gas including Anchor',
        'generator_sha256': digest(Path(__file__).read_bytes()),
        'source_sha256': {name: digest(Path(__file__).with_name(name).read_bytes()) for name in ('stack_density.py', 'initcode_preparation.py', 'top_level_create.py', 'spawn_memory.py')},
        'actuals_used_for_fitting': False,
        'required_comparison_quantities': ['complete proposal zkGas estimate', 'node-observable estimate for each block and their sum', 'actual same-proposal SP1 PGU'],
        'priority': 'actual same-proposal SP1PGU minus literal node-observable quote above2B within30M charged block gas is high priority; retain both quote deficits and ownN0',
        'gas_prediction': '21000intrinsic+489fixed opcode gas+3one-word memory expansion+batch_count*(55loop gas+2048body gas); zero refunds; Anchor167355 belongs only to frozen constructor and must be checked',
        'interpretation': 'Prospective validation, never refit from actuals. Native qualification must verify closed-form charged gas and successful ordered receipts before SP1. Model metadata does not select compiled prices. Empty calldata and canonical witnesses only; no guest or pricing change.',
        'cases': rows}))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL); args = parser.parse_args()
    print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
