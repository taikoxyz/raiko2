#!/usr/bin/env python3
"""Generate27 finite unique-message Ethereum KECCAK padding/bulk probes."""
import argparse
import importlib.util
import inspect
import json
from pathlib import Path
import re
import sys

import initcode_preparation
from initcode_preparation import _Factory
import spawn_memory
from spawn_memory import digest, encoded, write
import storage_fanout
from storage_fanout import BASE_ADDRESS, MODEL
import storage_mutation as mutation
import top_level_create as original

# The maintained stdlib reference is a benchmark dependency, never guest code.
_HASH_PATH = Path(__file__).resolve().parents[1] / 'tests' / 'ethereum_hash.py'
_HASH_NAME = '_zkgas_keccak_bulk_ethereum_reference'
if _HASH_NAME not in sys.modules:
    _spec = importlib.util.spec_from_file_location(_HASH_NAME, _HASH_PATH)
    _module = importlib.util.module_from_spec(_spec)
    sys.modules[_HASH_NAME] = _module
    _spec.loader.exec_module(_module)
ethereum_hash = sys.modules[_HASH_NAME]

CORPUS = 'unzen-keccak-bulk-20261104'
LENGTHS = (0, 135, 136, 137, 65536)
PATTERNS = {'zero': 0, 'a5': 165}
DOMAINS = ((0, 'zero'),) + tuple((length, pattern) for length in LENGTHS[1:] for pattern in PATTERNS)
LEVELS = ('zero', 'single', 'full')
MAX_REPETITIONS = 1_000_000
USERS = 2
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
OWNERS = tuple(f'0x{BASE_ADDRESS + tag:040x}' for tag in range(USERS))


def _validate(length, pattern, repetitions=0, tag=0):
    if type(length) is not int or length not in LENGTHS: raise ValueError('unknown input length')
    if type(pattern) is not str or pattern not in PATTERNS or (length == 0 and pattern != 'zero'): raise ValueError('unknown length/pattern domain')
    original.integer(repetitions, 0, MAX_REPETITIONS, 'repetitions')
    original.integer(tag, 0, USERS - 1, 'owner tag')


def _hash(message):
    """Bypass reference cache: no retained corpus of unique hash inputs."""
    return ethereum_hash.keccak256.__wrapped__(message)


def expected_message(length, pattern, index, tag=0):
    _validate(length, pattern, index, tag)
    return (((tag << 128) | index).to_bytes(32, 'big') + bytes([PATTERNS[pattern]]) * (length - 32)) if length else b''


def guard_offsets(length, pattern):
    _validate(length, pattern)
    extent = max(32, 32 * ((length + 31) // 32))
    return sorted({32, 64, 64 + extent - 32})


def _guards(length, pattern, repetitions, tag):
    """Only the selected final message is hashed; sizing uses placeholders."""
    extent = max(32, 32 * ((length + 31) // 32)); seed = bytes([PATTERNS[pattern]]) * 32
    memory = bytearray(64 + extent); memory[64:] = seed * (extent // 32)
    if length and length % 32: memory[64 + length:] = b'\xee' * (extent - length)
    memory[:32] = repetitions.to_bytes(32, 'big')
    if repetitions:
        memory[64:96] = ((tag << 128) | (repetitions - 1)).to_bytes(32, 'big')
        memory[32:64] = _hash(expected_message(length, pattern, repetitions - 1, tag))
    return [int.from_bytes(memory[offset:offset + 32], 'big') for offset in guard_offsets(length, pattern)]


def _assemble(length, pattern, repetitions, tag, values):
    """Fixed-width placeholders are private to gas-only sizing."""
    _validate(length, pattern, repetitions, tag)
    asm = _Factory(); extent = max(32, 32 * ((length + 31) // 32)); seed = bytes([PATTERNS[pattern]]) * 32
    def store(offset, value, times=1):
        asm.push(value, 32, times); asm.push(offset, times=times); asm.op(0x52, times)
    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)
    store(64 + extent - 32, int.from_bytes(b'\xee' * 32, 'big'))
    store(0, 0); store(32, 0); store(64, int.from_bytes(seed, 'big'))
    filled = 32
    while filled < extent:
        size = min(filled, extent - filled)
        asm.push(size); asm.push(64); asm.push(64 + filled); asm.op(0x5e); filled += size
    if length and length % 32:
        tail = bytes([PATTERNS[pattern]]) * (length % 32) + b'\xee' * (32 - length % 32)
        store(64 + extent - 32, int.from_bytes(tail, 'big'))
    visits = repetitions + 1; asm.label('loop', visits)
    load(0, visits); asm.push(repetitions, 32, visits); asm.op(0x14, visits); asm.target('done', visits); asm.op(0x57, visits)
    load(0, repetitions); asm.push(tag << 128, 32, repetitions); asm.op(1, repetitions); asm.push(64, times=repetitions); asm.op(0x52, repetitions)
    asm.push(length, times=repetitions); asm.push(64, times=repetitions); asm.op(0x20, repetitions)
    asm.push(32, times=repetitions); asm.op(0x52, repetitions)
    load(0, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions); asm.push(0, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); load(0); asm.push(repetitions, 32); asm.op(0x14); mutation._require(asm)
    for offset, value in zip(guard_offsets(length, pattern), values):
        load(offset); asm.push(value, 32); asm.op(0x14); mutation._require(asm)
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def _runtime(length, pattern, repetitions, tag=0):
    _validate(length, pattern, repetitions, tag)
    return _assemble(length, pattern, repetitions, tag, _guards(length, pattern, repetitions, tag))


def gas_accounting(length, pattern, repetitions):
    _validate(length, pattern, repetitions)
    code, counts = _assemble(length, pattern, repetitions, 0, [0] * len(guard_offsets(length, pattern)))
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x20: 0, 0x51: 3, 0x52: 3, 0x56: 8, 0x57: 10, 0x5b: 1, 0x5e: 0, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)]) for op, n in counts.items())
    extent = max(32, 32 * ((length + 31) // 32)); words = 2 + extent // 32
    memory = 3 * words + words * words // 512; preparation = 0; filled = 32
    while filled < extent:
        size = min(filled, extent - filled); preparation += 3 + 3 * ((size + 31) // 32); filled += size
    per_hash = 30 + 6 * ((length + 31) // 32); hashes = repetitions * per_hash
    gross = 21000 + ordinary + memory + preparation + hashes
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=memory,
        preparation_mcopy_gas=preparation, keccak_gas_per_call=per_hash, keccak_gas=hashes,
        gross_gas=gross, refund_counter=0, refund_applied=0, charged_gas=gross, runtime_bytes=len(code))


def _envelope(gas): return gas['gross_gas'] + 1000


def fits(length, pattern, repetitions):
    gas = gas_accounting(length, pattern, repetitions); envelope = _envelope(gas)
    return (gas['runtime_bytes'] < 24576 and envelope <= TRANSACTION_GAS_CAP and envelope <= EIP7825_GAS_CAP
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_repetitions(length, pattern):
    _validate(length, pattern)
    if not fits(length, pattern, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, MAX_REPETITIONS
    while low < high:
        middle = (low + high + 1) // 2
        if fits(length, pattern, middle): low = middle
        else: high = middle - 1
    assert 0 < low < MAX_REPETITIONS and fits(length, pattern, low) and not fits(length, pattern, low + 1)
    return low


def make_case(length=136, pattern='a5', level='full'):
    _validate(length, pattern)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero, single or full')
    maximum = full_repetitions(length, pattern); repetitions = {'zero': 0, 'single': 1, 'full': maximum}[level]
    gas = gas_accounting(length, pattern, repetitions); envelope = _envelope(gas_accounting(length, pattern, maximum))
    guards = [_guards(length, pattern, repetitions, tag) for tag in range(USERS)]
    runtimes = [_assemble(length, pattern, repetitions, tag, guards[tag]) for tag in range(USERS)]
    codes = [code for code, _ in runtimes]; counts = runtimes[0][1]; assert all(other == counts for _, other in runtimes)
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = list(OWNERS)
    program['prestate_contracts'] = [dict(address=owner, code_hex='0x' + code.hex(), initial_storage={}) for owner, code in zip(OWNERS, codes)]
    prefix = f'keccak-bulk-l{length}-{pattern}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']; extent = max(32, 32 * ((length + 31) // 32))
    guard_hex = [[f'0x{value:064x}' for value in values] for values in guards]
    return dict(sample_id=f'{prefix}-n{repetitions}', family='keccak-bulk', corpus_id=CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_keccak_bulk_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent byte intent/source/native/signature/receipt/code/account/empty-root/witness/original quote gates precede freeze and actuals.',
        scope='Finite whole-proposal comparison. Empty EVM hashing uses its constant shortcut. No user-only per-hash PGU attribution, tariff or production claim.',
        parameters=dict(input_bytes=length, pattern=pattern, workload_level=level, repetitions=repetitions, full_repetitions=maximum,
            owner_tags=list(range(USERS)), expected_counter=repetitions, counter_memory_offset=0, last_digest_memory_offset=32,
            input_memory_offset=64, payload_extent_bytes=extent, memory_words=2 + extent // 32,
            hash_permutations_per_call=0 if not length else length // 136 + 1,
            guard_offsets=guard_offsets(length, pattern), expected_guards=guard_hex,
            expected_last_digest=[values[0] for values in guard_hex], expected_last_input_word=[values[1] for values in guard_hex],
            expected_last_nonce_word_hex=[values[1] for values in guard_hex],
            expected_last_input_sha256=[digest(expected_message(length, pattern, repetitions - 1, tag)) if repetitions else None for tag in range(USERS)],
            message_rule='For nonempty L: BE256((tag<<128)|i) followed by pattern byte repeated L-32. Empty input is not unique; payload nonce backing is outside the hash range.',
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
            initial_storage_rule='Two distinct tagged code owners; empty persistent storage; one signer contiguous nonces0/1.',
            range_rule='Allocate once; seed full aligned extent logarithmically even R0; restore partial tail; nonce then KECCAK each iteration; exact final guards then STOP.'), request=request)


def source_modules():
    return {'cases/initcode_preparation.py': initcode_preparation, 'cases/spawn_memory.py': spawn_memory,
        'cases/storage_fanout.py': storage_fanout, 'cases/storage_mutation.py': mutation,
        'cases/top_level_create.py': original, 'tests/ethereum_hash.py': ethereum_hash}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        'ethereum_hash.keccak256': ethereum_hash.keccak256, 'ethereum_hash._permute': ethereum_hash._permute, 'ethereum_hash._rotate': ethereum_hash._rotate,
        **{f'keccak_bulk.{fn.__name__}': fn for fn in (_validate, _hash, expected_message, guard_offsets, _guards, _assemble, _runtime, gas_accounting, _envelope, fits, full_repetitions, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for length, pattern in DOMAINS:
        for level in LEVELS:
            row = make_case(length, pattern, level); raw = encoded(row.pop('request')); path = f"requests/{row['sample_id']}.json"
            write(output / path, raw); rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({row['sample_id'] for row in rows}) == len({row['request_sha256'] for row in rows}) == 27
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='Finite nine KECCAK length/pattern domains, own-zero/single/maximal uniform full.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        domains=[dict(input_bytes=length, pattern=pattern) for length, pattern in DOMAINS], levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False, primary_sources=['https://keccak.team/keccak_specs_summary.html'],
        native_observability='Native receipt gas/status/routes plus full/minimized outcome binding; no per-call memory/input/digest exports. Anchor may hash too.',
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'], cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
