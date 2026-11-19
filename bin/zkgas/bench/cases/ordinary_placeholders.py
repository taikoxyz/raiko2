#!/usr/bin/env python3
"""Emit a finite placeholder follow-up using the existing native/sampling tools."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
import precompile_boundaries as precompiles
import storage_mutation as mutation
import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL = '80160649d7d29a85879cd9524b35c271a4b8638f64de94eef91d016e56121cf8'
OPERATIONS = ('origin', 'call', 'caught-revert', 'ecrecover')
CHILD = '0x3000000000000000000000000000000000010001'
RUNTIME_BYTES = 1024
CHILD_RUNTIME_BYTES = 32
TRANSACTION_GAS_LIMIT = 15_000_000


def _call(asm, times, expected_status, *, scaffold=False):
    # CALL's seven inputs, with zero value and empty input/output in independent frames.
    for _ in range(5):
        asm.push(0, times=times)
    asm.push(int(CHILD, 16), 20, times)
    asm.push(1000, times=times)
    if scaffold:
        for _ in range(7):
            asm.op(0x50, times)
        asm.push(expected_status, times=times)
    else:
        asm.op(0xf1, times)
    mutation._check(asm, expected_status, times)
    asm.op(0x3d, times)
    mutation._check(asm, 0, times)


def _runtime(operation, repetitions, lane):
    asm = _Factory()
    target = lane == 'target'
    failed_child = operation == 'caught-revert'
    if operation != 'origin':
        _call(asm, 1, int(not failed_child))
    asm.push(0)
    visits = repetitions+1
    asm.label('loop', visits)
    asm.op(0x80, visits)
    asm.push(repetitions, times=visits)
    asm.op(0x14, visits)
    asm.target('done', visits)
    asm.op(0x57, visits)
    if operation == 'origin':
        # At this ordinary top-level call ORIGIN must equal CALLER. The independent
        # route counts distinguish the target from the same-valued scaffold.
        asm.op(0x32 if target else 0x33, repetitions)
        asm.op(0x33, repetitions)
        asm.op(0x14, repetitions)
        mutation._require(asm, repetitions)
    else:
        _call(asm, repetitions, int(not failed_child), scaffold=not target)
    asm.push(1, times=repetitions)
    asm.op(0x01, repetitions)
    asm.target('loop', repetitions)
    asm.op(0x56, repetitions)
    asm.label('done')
    mutation._check(asm, repetitions)
    asm.op(0x00)
    asm.label('failure', 0)
    asm.op(0xfe, 0)
    code = asm.finish()
    if len(code) > RUNTIME_BYTES:
        raise ValueError('runtime exceeds fixed padding')
    counts = dict(asm.counts)
    calls = 0 if operation == 'origin' else 1+(repetitions if target else 0)
    if calls:
        child = bytes.fromhex('6200000062000000fd') if failed_child else b'\x00'
        if failed_child:
            counts['opcode:0x62'] += 2*calls
            counts['opcode:0xfd'] = calls
        else:
            counts['opcode:0x00'] += calls
        child = child.ljust(CHILD_RUNTIME_BYTES, b'\x00')
    else:
        child = None
    return code.ljust(RUNTIME_BYTES, b'\x00'), child, counts, calls


def _gas(counts, calls):
    static = {0x00: 0, 0x01: 3, 0x14: 3, 0x15: 3, 0x32: 2, 0x33: 2,
              0x3d: 2, 0x50: 2, 0x56: 8, 0x57: 10, 0x5b: 1,
              0x80: 3, 0xf1: 0, 0xfd: 0, 0xfe: 0}
    ordinary = sum(n*(3 if 0x60 <= int(key[-2:], 16) <= 0x7f else static[int(key[-2:], 16)])
                   for key, n in counts.items())
    access = 2600+100*(calls-1) if calls else 0
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary,
        call_access_gas=access, memory_expansion_gas=0, refund_applied=0,
        charged_gas=21000+ordinary+access)


def _sid(operation, repetitions, lane):
    return f'ordinary-placeholder-{operation}-n{repetitions}-{lane}'


def make_case(operation, *, repetitions, lane='target'):
    """Build one bounded request; preserve ECRECOVER's existing request and identities."""
    if type(operation) is not str or operation not in OPERATIONS:
        raise ValueError('unknown placeholder operation')
    original.integer(repetitions, 0, 65536, 'repetitions')
    if type(lane) is not str or lane not in ('target', 'scaffold'):
        raise ValueError('lane must be target or scaffold')
    if operation == 'ecrecover':
        if lane != 'target':
            raise ValueError('ECRECOVER reuses the existing guarded precompile target')
        return precompiles.make_case('ecrecover-valid', repetitions)
    code, child, counts, calls = _runtime(operation, repetitions, lane)
    gas = _gas(counts, calls)
    if gas['charged_gas']+1000 > TRANSACTION_GAS_LIMIT:
        raise ValueError('recipe exceeds fixed transaction envelope')
    if mutation.ANCHOR_GAS+gas['charged_gas'] > 30_000_000:
        raise ValueError('recipe exceeds charged block budget')
    row = original.make_case('call', 'stop', gas_limit=TRANSACTION_GAS_LIMIT)
    program = row['request']['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0)
    routes.update(counts)
    program.update(code_hex='0x'+code.hex(), expected_route_counts=routes)
    if child is not None:
        program['prestate_contracts'] = [dict(address=CHILD,
            code_hex='0x'+child.hex(), initial_storage={})]
    sid = _sid(operation, repetitions, lane)
    row.update(sample_id=sid, family='ordinary-placeholder-followup',
        role='control' if repetitions == 0 or lane == 'scaffold' else 'candidate',
        baseline_sample_id=_sid(operation, 0, lane),
        paired_sample_id=_sid(operation, repetitions, 'scaffold' if lane == 'target' else 'target'),
        data_role='model_development_probe', holdout_only=False, actuals_used_for_fitting=False,
        completion_gate='Successful outer receipt; actual ORIGIN/CALLER and CALL opcode counts; '
            'each child status/empty returndata; normal child REVERT rather than exceptional halt; '
            'exact gas and existing full/minimized native replay equality.',
        scope='Finite controlled whole-proposal measurements. Target count increments include '
            'loop and guard work; scaffold differences are diagnostic, not a subtraction credit. '
            'The synthetic ORIGIN720 quote-exclusion control is not historical application evidence.')
    row['parameters'] = dict(operation=operation, repetitions=repetitions, lane=lane,
        runtime_bytes=len(code), runtime_sha256=digest(code), child_runtime_bytes=len(child) if child else 0,
        child_runtime_sha256=digest(child) if child else None,
        expected_completed_iterations=repetitions,
        expected_target_events=repetitions if lane == 'target' else 0,
        expected_origin_events=repetitions if operation == 'origin' and lane == 'target' else 0,
        expected_confirmed_call_events=calls, expected_call_warmup_events=int(operation != 'origin'),
        expected_child_stop_events=calls if operation == 'call' else 0,
        expected_child_revert_events=calls if operation == 'caught-revert' else 0,
        requested_child_gas=1000 if calls else None, child_call_value=0,
        expected_child_returndata_bytes=0, expected_exceptional_instructions=0,
        transaction_kind='call', count_per_block=1, block_count=1,
        transaction_gas_limit=TRANSACTION_GAS_LIMIT, header_gas_limit=mutation.HEADER_GAS_LIMIT,
        charged_gas_budget=30_000_000, expected_anchor_gas=mutation.ANCHOR_GAS,
        expected_user_charged_gas=gas['charged_gas'],
        expected_block_charged_gas=mutation.ANCHOR_GAS+gas['charged_gas'],
        gas_accounting_per_user=gas, expected_user_success=True, expected_terminal='STOP',
        outer_calldata_bytes=0, execution_profile='evm-gas-only',
        origin_guard='At ordinary top-level CALL, ORIGIN equals CALLER; native opcode counts disambiguate the scaffold.',
        availability_role='constructed_high_quote_control' if operation == 'origin' and repetitions == 720
            else 'synthetic_unit_probe',
        witness_policy='existing native actual-read minimization and full/minimized replay equality')
    return row


def prepare(output_dir, *, model_id=MODEL):
    """Freeze a create-only 23-row catalog without executing a guest or choosing prices."""
    if not isinstance(output_dir, Path):
        raise ValueError('output_dir must be a Path')
    if type(model_id) is not str or re.fullmatch(r'[0-9a-f]{64}', model_id) is None:
        raise ValueError('model_id must be lowercase sha256')
    plans = [make_case(operation, repetitions=n, lane=lane)
             for operation in ('origin', 'call', 'caught-revert')
             for n in ((0, 1, 8, 720) if operation == 'origin' else (0, 1, 8))
             for lane in ('target', 'scaffold')]
    plans += [make_case('ecrecover', repetitions=n) for n in (0, 1, 8)]
    output_dir.mkdir(parents=True, exist_ok=False)
    (output_dir/'requests').mkdir()
    rows = []
    for plan in plans:
        raw = encoded(plan.pop('request'))
        relative = f"requests/{plan['sample_id']}.json"
        write(output_dir/relative, raw)
        rows.append(plan | dict(request_path=relative, request_sha256=digest(raw)))
    sources = ('ordinary_placeholders.py', 'initcode_preparation.py', 'top_level_create.py',
               'storage_mutation.py', 'spawn_memory.py', 'precompile_boundaries.py',
               'precompile_boundary_vectors.json')
    write(output_dir/'catalog.json', encoded(dict(schema_version=1, frozen_model_id=model_id,
        purpose='Finite placeholder follow-up with independent result/count/gas guards; no price changes.',
        source_sha256={name: digest(Path(__file__).with_name(name).read_bytes()) for name in sources},
        interpretation='Keep all N0/scaffold/control rows. Prefer same-target count increments with '
            'no scaffold credit. ECRECOVER requests and sample IDs are retained verbatim from the '
            'existing precompile panel; reuse qualified actuals instead of resampling. '
            'All rows are development probes, not unopened validation or normal-application evidence.',
        cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args()
    print(json.dumps(dict(prepared=prepare(args.output_dir, model_id=args.model_id))))
