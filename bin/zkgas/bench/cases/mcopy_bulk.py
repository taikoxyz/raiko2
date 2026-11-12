#!/usr/bin/env python3
"""Generate twelve compiler-free, preallocated bulk-MCOPY relative-layout probes."""
import argparse
import hashlib
import inspect
import json
from pathlib import Path
import re

import initcode_preparation
from initcode_preparation import _Factory
import spawn_memory
from spawn_memory import digest, encoded, write
import storage_fanout
from storage_fanout import BASE_ADDRESS, MODEL
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-mcopy-bulk-20261104'
LENGTH = 65536
LAYOUTS = {'same-residue-disjoint': (64, 65600, 131072),
           'different-residue-disjoint': (64, 65601, 131073),
           'overlap-right': (64, 65, 65537), 'overlap-left': (65, 64, 65537)}
LEVELS = ('zero', 'single', 'full')
MAX_REPETITIONS = 1_000_000
USERS = 2
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
OWNERS = tuple(f'0x{BASE_ADDRESS + tag:040x}' for tag in range(USERS))


def _validate(layout, repetitions=0, tag=0):
    if type(layout) is not str or layout not in LAYOUTS: raise ValueError('unknown MCOPY layout')
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    original.integer(tag, 0, USERS - 1, 'owner tag')


def guard_offsets(layout):
    _validate(layout); source, destination, extent = LAYOUTS[layout]
    words = (64 + extent + 31) // 32
    return sorted({32, source, source + LENGTH // 2, source + LENGTH - 32,
        destination, destination + LENGTH // 2, destination + LENGTH - 32, 32 * (words - 1)}
        | ({64 + LENGTH} if layout == 'different-residue-disjoint' else set()))


def expected_memory(layout, repetitions, tag=0):
    """Selected-input byte simulation only; never called by gas-only sizing."""
    _validate(layout, repetitions, tag); source, destination, extent = LAYOUTS[layout]
    size = 32 * ((64 + extent + 31) // 32); memory = bytearray(size)
    memory[-32:] = b'\xee' * 32; memory[32:64] = b'\x7b' * 32
    a = bytes(i | (tag << 7) for i in range(32)); b = bytes(v ^ 255 for v in a)
    memory[64:64 + LENGTH] = a * (LENGTH // 32)
    if 'disjoint' in layout:
        memory[destination:destination + LENGTH] = b * (LENGTH // 32)
        if layout == 'different-residue-disjoint': memory[64 + LENGTH] = 0xcd
    else: memory[64 + LENGTH] = a[0]
    for _ in range(repetitions): memory[destination:destination + LENGTH] = bytes(memory[source:source + LENGTH])
    memory[:32] = repetitions.to_bytes(32, 'big')
    return bytes(memory)


def _assemble(layout, repetitions, tag, values):
    """Fixed-width guard values may be private placeholders for gas sizing only."""
    _validate(layout, repetitions, tag); source, destination, extent = LAYOUTS[layout]
    asm = _Factory(); words = (64 + extent + 31) // 32
    def store(offset, value, width=32, times=1):
        asm.push(value, width, times); asm.push(offset, times=times); asm.op(0x52, times)
    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)
    def byte(offset, value):
        asm.push(value, 1); asm.push(offset); asm.op(0x53)
    def copy(source_at, destination_at, size, times=1):
        asm.push(size, times=times); asm.push(source_at, times=times); asm.push(destination_at, times=times); asm.op(0x5e, times)
    # Allocate once before all seed and workload copies, preserving unused padding.
    store(32 * (words - 1), int.from_bytes(b'\xee' * 32, 'big'))
    store(0, 0); store(32, int.from_bytes(b'\x7b' * 32, 'big'))
    a = bytes(i | (tag << 7) for i in range(32)); b = bytes(v ^ 255 for v in a)
    seeds = [(64, a)] + ([(destination, b)] if 'disjoint' in layout else [])
    for offset, pattern in seeds:
        store(offset, int.from_bytes(pattern, 'big')); filled = 32
        while filled < LENGTH:
            size = min(filled, LENGTH - filled); copy(offset, offset + filled, size); filled += size
    if layout == 'different-residue-disjoint': byte(64 + LENGTH, 0xcd)
    elif 'overlap' in layout: byte(64 + LENGTH, a[0])
    visits = repetitions + 1; asm.label('loop', visits)
    load(0, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    copy(source, destination, LENGTH, repetitions)
    load(0, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(0, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); load(0); asm.push(repetitions, 32); asm.op(0x14); mutation._require(asm)
    for offset, value in zip(guard_offsets(layout), values):
        load(offset); asm.push(value, 32); asm.op(0x14); mutation._require(asm)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _runtime(layout, repetitions, tag=0):
    memory = expected_memory(layout, repetitions, tag)
    return _assemble(layout, repetitions, tag, [int.from_bytes(memory[o:o + 32], 'big') for o in guard_offsets(layout)])


def gas_accounting(layout, repetitions):
    _validate(layout, repetitions)
    code, counts = _assemble(layout, repetitions, 0, [0] * len(guard_offsets(layout)))
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x51: 3, 0x52: 3, 0x53: 3, 0x56: 8, 0x57: 10, 0x5b: 1, 0x5e: 0, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)]) for op, n in counts.items())
    words = (64 + LAYOUTS[layout][2] + 31) // 32; memory = 3 * words + words * words // 512
    filled = 32; preparation = 0
    while filled < LENGTH:
        size = min(filled, LENGTH - filled); preparation += 3 + 3 * ((size + 31) // 32); filled += size
    preparation *= 2 if 'disjoint' in layout else 1
    per_copy = 3 + 3 * ((LENGTH + 31) // 32); copies = preparation + repetitions * per_copy
    gross = 21000 + ordinary + memory + copies
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=memory,
        preparation_mcopy_gas=preparation, loop_mcopy_gas_per_copy=per_copy, loop_mcopy_gas=repetitions * per_copy,
        mcopy_gas=copies, gross_gas=gross, refund_counter=0, refund_applied=0, charged_gas=gross, runtime_bytes=len(code))


def _envelope(gas):
    return gas['gross_gas'] + 1000


def fits(layout, repetitions):
    gas = gas_accounting(layout, repetitions); envelope = _envelope(gas)
    return (gas['runtime_bytes'] < 24576 and envelope <= TRANSACTION_GAS_CAP and envelope <= EIP7825_GAS_CAP
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_repetitions(layout):
    _validate(layout)
    if not fits(layout, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, MAX_REPETITIONS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(layout, middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(layout, low) and not fits(layout, low + 1)
    return low


def make_case(layout='same-residue-disjoint', level='full'):
    _validate(layout)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero, single or full')
    maximum = full_repetitions(layout); repetitions = {'zero': 0, 'single': 1, 'full': maximum}[level]
    source, destination, extent = LAYOUTS[layout]; gas = gas_accounting(layout, repetitions)
    envelope = _envelope(gas_accounting(layout, maximum)); memories = [expected_memory(layout, repetitions, tag) for tag in range(USERS)]
    guards = [[int.from_bytes(memory[o:o + 32], 'big') for o in guard_offsets(layout)] for memory in memories]
    runtimes = [_assemble(layout, repetitions, tag, guards[tag]) for tag in range(USERS)]
    codes = [code for code, _ in runtimes]; counts = runtimes[0][1]; assert all(other == counts for _, other in runtimes)
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = list(OWNERS)
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(), initial_storage={}) for owner, code in zip(OWNERS, codes)]
    prefix = f'mcopy-bulk-{layout}-l{LENGTH}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    return dict(sample_id=f'{prefix}-n{repetitions}', family='mcopy-bulk', corpus_id=CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_mcopy_bulk_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent byte intent, source, native/signature/receipt/code/account/empty-root/witness/original quote gates precede freeze and actuals.',
        scope='Finite whole-proposal comparison. Relative residues do not export heap alignment; native gas/status/routes and replay do not export final memory. No isolated MCOPY PGU or price claim.',
        parameters=dict(layout=layout, workload_level=level, repetitions=repetitions, full_repetitions=maximum,
            input_bytes=LENGTH, source_memory_offset=source, destination_memory_offset=destination, payload_extent_bytes=extent,
            owner_tags=list(range(USERS)), expected_counter=repetitions, counter_memory_offset=0, protected_memory_offset=32,
            payload_memory_offset=64, memory_words=(64 + extent + 31) // 32,
            guard_offsets=guard_offsets(layout), expected_guards=[[f'0x{v:064x}' for v in values] for values in guards],
            expected_final_memory_sha256=[digest(memory) for memory in memories],
            intent_commitment='Final memory SHA256 and sampled words are byte-intent/debug only, not native memory exports or protocol commitments.',
            seed_rule='A32=bytes(i|(tag<<7),i0..31);B32=inverse(A32);disjoint A then B;overlap A plus A[0];gap0xcd;padding0xee;protected0x7b',
            outer_calldata_bytes=0, account_count=USERS, aggregate_state_size=0, state_size_per_account=0,
            expected_initial_storage_entry_count=0, expected_final_storage_entry_count=0, expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            user_transactions_per_block=USERS, all_user_transactions=USERS, all_transactions_including_anchor=USERS + 1,
            block_count=1, visits_per_account=1, transaction_kind='call', transaction_gas_limit=envelope,
            transaction_envelope_reserve=1000, header_gas_limit=HEADER_GAS_LIMIT, charged_gas_budget=BLOCK_BUDGET,
            expected_anchor_gas=ANCHOR_GAS, gas_accounting_per_user=gas, expected_user_gross_gas=gas['gross_gas'],
            expected_user_refund_counter=0, expected_user_refund_applied=0, expected_user_charged_gas=gas['charged_gas'],
            expected_block_charged_gas=block_gas, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            recipient_schedule=list(OWNERS), expected_terminal='STOP', expected_user_success=True,
            expected_user_receipt_statuses=[True] * USERS, expected_anchor_success=True, expected_exceptional_instructions=0,
            execution_profile='evm-gas-only', preload_created_accounts_or_code=False, shared_runtime=False,
            runtime_bytes=len(codes[0]), runtime_sha256=[digest(code) for code in codes], unused_primary_code_hex='0x00',
            initial_storage_rule='Two distinct authenticated code owners; empty persistent storage. Sender contiguous nonces, fees and Anchor still change.',
            range_rule='Preallocate aligned payload once; seed logarithmically even R0; one snapshot-equivalent 65536-byte MCOPY per loop; counter and byte guards then STOP.'), request=request)


def source_modules():
    return {'initcode_preparation.py': initcode_preparation, 'spawn_memory.py': spawn_memory,
        'storage_fanout.py': storage_fanout, 'storage_mutation.py': mutation, 'top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'mcopy_bulk.{fn.__name__}': fn for fn in (_validate, guard_offsets, expected_memory, _assemble, _runtime, gas_accounting, _envelope, fits, full_repetitions, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for layout in LAYOUTS:
        for level in LEVELS:
            row = make_case(layout, level); raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
            write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 12
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='Twelve finite preallocated65536-byte MCOPY layout probes, own-zero/single/maximal uniform full.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        layouts={name: dict(source_offset=s, destination_offset=d, payload_extent=e) for name, (s, d, e) in LAYOUTS.items()}, levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False, primary_sources=['https://eips.ethereum.org/EIPS/eip-5656'],
        native_observability='ControlledNativeReceipt exports gas/status/routes; constructor compares full-vs-minimized execution_result. No final-memory export. Relative EVM offsets do not reveal guest heap base alignment.',
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'], cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
