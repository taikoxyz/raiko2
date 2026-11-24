#!/usr/bin/env python3
"""Generate18 guarded, finite BN254 pairing inputs without a compiler."""
import argparse
import inspect
import json
from pathlib import Path
import re

import initcode_preparation
from initcode_preparation import _Factory
import precompile_workload
from precompile_workload import FIELD_MODULUS, GROUP_ORDER, _point_add
import spawn_memory
from spawn_memory import digest, encoded, write
import storage_fanout
from storage_fanout import BASE_ADDRESS, MODEL
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-bn254-pairing-dense-20261103'
KINDS = ('identity', 'nonidentity')
PAIR_COUNTS = (2, 16, 64)
LEVELS = ('zero', 'single', 'full')
USERS = 2
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
OWNER = f'0x{BASE_ADDRESS:040x}'
# EIP-197 encoding: Fq2 imaginary coefficient first, then real coefficient.
G2 = (
    11559732032986387107991004021392285783925812861821192530917403151452391805634,
    10857046999023057135944570762232829481370756359578518086990519993285655852781,
    4082367875863433681332203403145435568316851327593401208105741076214120093531,
    8495653923123431417604973247489272438418190587263600148770280649306958101930,
)


def _validate(kind, pairs):
    if type(kind) is not str or kind not in KINDS: raise ValueError('kind must be identity or nonidentity')
    if type(pairs) is not int or pairs not in PAIR_COUNTS: raise ValueError('pairs must be2,16or64')


def pairing_input(kind, pairs):
    """Use the existing G1 affine helper; no independent production EC generator."""
    _validate(kind, pairs); point = None; values = []
    for _ in range(pairs // 2 if kind == 'identity' else pairs):
        point = _point_add(point, (1, 2)); x, y = point
        values.extend((x, y, *G2))
        if kind == 'identity': values.extend((x, (-y) % FIELD_MODULUS, *G2))
    return b''.join(value.to_bytes(32, 'big') for value in values)


def _runtime(kind, pairs, repetitions):
    """All data is unreachable; declared counts describe one successful user."""
    _validate(kind, pairs); original.integer(repetitions, 0, 65536, 'repetitions')
    asm = _Factory(); size = 192 * pairs; output = 32 + size

    def store(offset, value, times=1):
        asm.push(value, times=times); asm.push(offset, times=times); asm.op(0x52, times)

    def load(offset, times=1):
        asm.push(offset, times=times); asm.op(0x51, times)

    asm.push(size); asm.target('data'); asm.push(32); asm.op(0x39)
    store(output, 2); store(0, 0)
    visits = repetitions + 1; asm.label('loop', visits)
    load(0, visits); asm.push(repetitions, times=visits); asm.op(0x14, visits)
    asm.target('done', visits); asm.op(0x57, visits)
    store(output, 2, repetitions)
    asm.push(32, times=repetitions); asm.push(output, times=repetitions)
    asm.push(size, times=repetitions); asm.push(32, times=repetitions)
    asm.push(8, times=repetitions); asm.op(0x5a, repetitions); asm.op(0xfa, repetitions)
    mutation._check(asm, 1, repetitions)
    asm.op(0x3d, repetitions); mutation._check(asm, 32, repetitions)
    load(output, repetitions); mutation._check(asm, int(kind == 'identity'), repetitions)
    load(0, repetitions); asm.push(1, times=repetitions); asm.op(1, repetitions)
    asm.push(0, times=repetitions); asm.op(0x52, repetitions)
    asm.target('loop', repetitions); asm.op(0x56, repetitions)
    asm.label('done'); load(0); mutation._check(asm, repetitions); asm.op(0)
    asm.label('failure', 0); asm.op(0xfe, 0)
    # A CODECOPY source offset is a label but does not need an executed JUMPDEST.
    asm.labels['data'] = len(asm.code)
    return asm.finish() + pairing_input(kind, pairs), asm.counts


def gas_accounting(kind, pairs, repetitions):
    _validate(kind, pairs); original.integer(repetitions, 0, 65536, 'repetitions')
    _, counts = _runtime(kind, pairs, repetitions)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x39: 0, 0x3d: 2, 0x51: 3,
             0x52: 3, 0x56: 8, 0x57: 10, 0x5a: 2, 0x5b: 1, 0xfa: 0, 0xfe: 0}
    ordinary = sum(n * (3 if 0x60 <= int(op[-2:], 16) <= 0x7f else costs[int(op[-2:], 16)]) for op, n in counts.items())
    words = 2 + 6 * pairs; memory = 3 * words + words * words // 512
    copy_gas = 3 + 18 * pairs; native = 45000 + 34000 * pairs
    precompile = repetitions * (100 + native); gross = 21000 + ordinary + memory + copy_gas + precompile
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=memory,
        codecopy_gas=copy_gas, native_gas_per_call=native, warm_access_gas_per_call=100,
        precompile_gas=precompile, gross_gas=gross, refund_counter=0, refund_applied=0, charged_gas=gross)


def _envelope(gas):
    # Conservative fixture reserve for EIP-150 and continuation; no storage sentry.
    return gas['gross_gas'] + (gas['native_gas_per_call'] + 62) // 63 + 2301


def fits(kind, pairs, repetitions):
    _validate(kind, pairs); original.integer(repetitions, 0, 65536, 'repetitions')
    gas = gas_accounting(kind, pairs, repetitions); envelope = _envelope(gas)
    code, _ = _runtime(kind, pairs, repetitions)
    return (len(code) <= 24576 and envelope <= min(TRANSACTION_GAS_CAP, EIP7825_GAS_CAP)
        and ANCHOR_GAS + USERS * gas['charged_gas'] <= BLOCK_BUDGET
        and all(HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] >= envelope for j in range(USERS)))


def full_repetitions(kind, pairs):
    _validate(kind, pairs)
    if not fits(kind, pairs, 0): raise ValueError('zero control does not fit constraints')
    low, high = 0, 65536
    while low < high:
        middle = (low + high + 1) // 2
        if fits(kind, pairs, middle): low = middle
        else: high = middle - 1
    assert 0 < low < 65536 and fits(kind, pairs, low) and not fits(kind, pairs, low + 1)
    return low


def make_case(kind='nonidentity', pairs=2, level='full'):
    _validate(kind, pairs)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero, single or full')
    maximum = full_repetitions(kind, pairs); repetitions = {'zero': 0, 'single': 1, 'full': maximum}[level]
    gas = gas_accounting(kind, pairs, repetitions); envelope = _envelope(gas_accounting(kind, pairs, maximum))
    code, counts = _runtime(kind, pairs, repetitions); payload = pairing_input(kind, pairs)
    request = original.make_case('call', 'stop', count=USERS, gas_limit=envelope)['request']
    program = request['program']; routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update({op: count * USERS for op, count in counts.items()}); program['expected_route_counts'] = routes
    program['transactions']['recipients'] = [OWNER] * USERS
    program['prestate_contracts'] = [dict(address=OWNER, code_hex='0x' + code.hex(), initial_storage={})]
    prefix = f'bn254-pairing-dense-{kind}-k{pairs}'; block_gas = ANCHOR_GAS + USERS * gas['charged_gas']
    return dict(sample_id=f'{prefix}-n{repetitions}', family='bn254-pairing-dense', corpus_id=CORPUS,
        role='control' if repetitions == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_bn254_pairing_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent exact-source review and all18 native/signature/receipt/code/account/empty-root/witness/operand/wrapper/counter/original-real-quote gates precede freeze and actuals.',
        scope='Finite whole-input frozen-pricing comparison; actual same-proposal SP1 PGU, literal node block estimates/sum and complete-proposal quotes remain separate. No isolated pairing PGU, refit, arbitrary operand/failure/cache bound, global, IID, proof, GPU or production claim.',
        parameters=dict(kind=kind, pairs=pairs, workload_level=level, repetitions=repetitions, full_repetitions=maximum,
            precompile_address='0x08', expected_pairing_result=int(kind == 'identity'), expected_precompile_success=True,
            expected_precompile_calls=USERS * repetitions, expected_precompile_calls_per_user=repetitions,
            pairing_input_hex='0x' + payload.hex(), pairing_input_sha256=digest(payload), pairing_input_bytes=len(payload),
            pairing_element_bytes=192, all_points_finite=True, outer_calldata_bytes=0,
            input_memory_offset=32, output_memory_offset=32 + len(payload), counter_memory_offset=0,
            memory_words=2 + 6 * pairs, output_sentinel=2, expected_counter=repetitions,
            account_count=1, aggregate_state_size=0, state_size_per_account=0, expected_initial_storage_entry_count=0,
            expected_final_storage_entry_count=0, expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            user_transactions_per_block=USERS, all_user_transactions=USERS, all_transactions_including_anchor=USERS + 1,
            block_count=1, visits_per_account=USERS, transaction_kind='call', transaction_gas_limit=envelope,
            transaction_envelope_reserve=_envelope(gas) - gas['gross_gas'], header_gas_limit=HEADER_GAS_LIMIT,
            charged_gas_budget=BLOCK_BUDGET, expected_anchor_gas=ANCHOR_GAS, gas_accounting_per_user=gas,
            expected_user_gross_gas=gas['gross_gas'], expected_user_refund_counter=0, expected_user_refund_applied=0,
            expected_user_charged_gas=gas['charged_gas'], expected_block_charged_gas=block_gas,
            near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS - j * gas['charged_gas'] for j in range(USERS)],
            recipient_schedule=[OWNER] * USERS, expected_terminal='STOP', expected_user_success=True,
            expected_user_receipt_statuses=[True] * USERS, expected_anchor_success=True, expected_exceptional_instructions=0,
            execution_profile='evm-gas-only', preload_created_accounts_or_code=False, shared_runtime=True,
            runtime_bytes=len(code), runtime_sha256=digest(code), unused_primary_code_hex='0x00',
            initial_storage_rule='One authenticated code account with empty persistent storage; whole-world fees, nonces and Anchor still change.',
            range_rule='Nonempty CODECOPY for every level. Reset output sentinel2 every call; require status1,32-byte returndata,exact0/1, counter and STOP.',
            point_schedule='(jP,Q),(-jP,Q) for j1..K/2' if kind == 'identity' else '(jP,Q) for j1..K; exponent K(K+1)/2 nonzero modulo r'), request=request)


def source_modules():
    return {'initcode_preparation.py': initcode_preparation, 'precompile_workload.py': precompile_workload,
            'spawn_memory.py': spawn_memory, 'storage_fanout.py': storage_fanout,
            'storage_mutation.py': mutation, 'top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'precompile_workload._point_add': _point_add,
        'storage_mutation._check': mutation._check, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        'bn254_pairing_dense._validate': _validate, 'bn254_pairing_dense.pairing_input': pairing_input,
        'bn254_pairing_dense._runtime': _runtime, 'bn254_pairing_dense.gas_accounting': gas_accounting,
        'bn254_pairing_dense._envelope': _envelope, 'bn254_pairing_dense.fits': fits,
        'bn254_pairing_dense.full_repetitions': full_repetitions, 'bn254_pairing_dense.make_case': make_case,
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for kind in KINDS:
        for pairs in PAIR_COUNTS:
            for level in LEVELS:
                row = make_case(kind, pairs, level); raw = encoded(row.pop('request'))
                path = f"requests/{row['sample_id']}.json"; write(output / path, raw)
                rows.append(row | {'request_path': path, 'request_sha256': digest(raw)})
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 18
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='18 finite dense pairing inputs: identity/nonidentity, K2/16/64, own-zero/single/maximal-uniform-full.',
        generator_sha256=digest(Path(__file__).read_bytes()),
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(helper).encode()) for name, helper in helpers().items()},
        kinds=list(KINDS), pairs=list(PAIR_COUNTS), levels=list(LEVELS), users=USERS,
        holdout_only=True, actuals_used_for_fitting=False,
        primary_sources=['https://eips.ethereum.org/EIPS/eip-197', 'https://eips.ethereum.org/EIPS/eip-1108'],
        required_comparison_quantities=['actual same-proposal SP1 PGU', 'node per-block estimates and literal sum', 'complete proposal estimate'], cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
