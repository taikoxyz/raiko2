#!/usr/bin/env python3
"""Finite ordinary-opcode probes; generation does not execute or choose tariffs."""
import argparse
import json
from pathlib import Path
import re

from initcode_preparation import _Factory
import top_level_create as original
import storage_mutation as mutation
from spawn_memory import digest, encoded, write

MODEL = 'eea6f1da87ee129d2d9f759ae6cef01cb8ac93988376a86efb290398e6db5da6'
OWNER = '0x3000000000000000000000000000000000010000'
CHILD = '0x3000000000000000000000000000000000010001'
RUNTIME_BYTES = 1024
PAYLOAD_OFFSET = 896
PAYLOAD = bytes(range(1, 256)) + b'\x01'
OPCODES = {'codesize': 0x38, 'codecopy': 0x39, 'log3': 0xa3, 'staticcall': 0xfa}
COUNTS = {'codesize': ((0,4096,65536),(8192,32768)),
          'codecopy': ((0,4096,65536),(8192,32768)),
          'log3': ((0,1,2048),(256,512)),
          'staticcall': ((0,256,4096),(1024,16384))}
LENGTHS = {'codesize': (0,), 'codecopy': (0,32,64,96), 'log3': (0,32,256), 'staticcall': (0,)}


def _staticcall(asm, times, *, scaffold=False):
    for _ in range(4): asm.push(0, times=times)
    asm.push(int(CHILD,16),20,times); asm.push(1000,times=times)
    if scaffold:
        for _ in range(6): asm.op(0x50,times)
        asm.push(1,times=times)
    else: asm.op(0xfa,times)
    mutation._check(asm,1,times)
    asm.op(0x3d,times); mutation._check(asm,0,times)


def _runtime(operation, n, length, lane):
    asm = _Factory(); target = lane == 'target'
    # Identical memory setup at N0, including the untouched-boundary guard area.
    asm.push(0); asm.push(320); asm.op(0x52)
    if operation == 'log3':
        for offset in range(0,256,32):
            asm.push(int.from_bytes(PAYLOAD[offset:offset+32],'big'),32); asm.push(offset); asm.op(0x52)
    if operation == 'staticcall': _staticcall(asm,1)
    asm.push(0); asm.label('loop',n+1)
    asm.op(0x80,n+1); asm.push(n,times=n+1); asm.op(0x14,n+1)
    asm.target('done',n+1); asm.op(0x57,n+1)
    if operation == 'codesize':
        if target: asm.op(0x38,n)
        else: asm.push(RUNTIME_BYTES,times=n)
        mutation._check(asm,RUNTIME_BYTES,n)
    elif operation == 'codecopy':
        asm.push(length,times=n); asm.push(PAYLOAD_OFFSET,times=n); asm.push(0,times=n)
        if target: asm.op(0x39,n)
        else:
            for _ in range(3): asm.op(0x50,n)
    elif operation == 'log3':
        asm.push(0x33,times=n); asm.push(0x22,times=n); asm.op(0x82,n)
        asm.push(length,times=n); asm.push(0,times=n)
        if target: asm.op(0xa3,n)
        else:
            for _ in range(5): asm.op(0x50,n)
    else: _staticcall(asm,n,scaffold=not target)
    asm.push(1,times=n); asm.op(1,n); asm.target('loop',n); asm.op(0x56,n)
    asm.label('done'); mutation._check(asm,n)
    if operation == 'codecopy':
        expected = PAYLOAD[:length] + bytes(128-length) if target and n else bytes(128)
        # Check every copied byte and untouched suffix, independently of CODECOPY.
        for offset in range(0,128,32):
            asm.push(offset); asm.op(0x51)
            asm.push(int.from_bytes(expected[offset:offset+32],'big'),32); asm.op(0x14); mutation._require(asm)
        asm.push(128); asm.op(0x51); mutation._check(asm,0)
    asm.op(0); asm.label('failure',0); asm.op(0xfe,0)
    code=asm.finish()
    if len(code) > PAYLOAD_OFFSET: raise ValueError('runtime exceeds reserved code payload boundary')
    code += bytes(PAYLOAD_OFFSET-len(code)) + PAYLOAD[:128]
    counts = dict(asm.counts)
    if operation=='staticcall': counts['opcode:0x00'] += 1 + (n if target else 0)
    return code, counts


def _gas(operation, n, length, lane, counts):
    static = {0:0, 1:3, 0x14:3, 0x15:3, 0x38:2, 0x39:0, 0x3d:2,
              0x50:2, 0x51:3, 0x52:3, 0x56:8, 0x57:10, 0x5b:1,
              0x80:3, 0x82:3, 0xa3:0, 0xfa:0, 0xfe:0}
    ordinary = sum(count*(3 if 0x60 <= int(key[-2:],16) <= 0x7f else static[int(key[-2:],16)]) for key,count in counts.items())
    target = lane=='target'
    copy = n*(3+3*((length+31)//32)) if operation=='codecopy' and target else 0
    logs = n*(1500+8*length) if operation=='log3' and target else 0
    calls = 2600 + (100*n if target else 0) if operation=='staticcall' else 0
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=33,
                codecopy_gas=copy, log_gas=logs, call_access_gas=calls,
                charged_gas=21000+ordinary+33+copy+logs+calls, refund_applied=0)


def _sid(operation,n,length,lane): return f'ordinary-{operation}-l{length}-n{n}-{lane}'


def make_case(operation, *, repetitions, length=0, lane='target'):
    if type(operation) is not str or operation not in OPCODES: raise ValueError('unknown ordinary operation')
    n=original.integer(repetitions,0,65536,'repetitions')
    original.integer(length,0,256,'length')
    if type(lane) is not str or lane not in ('target','scaffold'): raise ValueError('lane must be target or scaffold')
    if length not in LENGTHS[operation] + ((97,) if operation=='codecopy' else ()): raise ValueError('unsupported length for operation')
    code,counts=_runtime(operation,n,length,lane); gas=_gas(operation,n,length,lane,counts)
    # No cap inflation: keep the constructor TX envelope and block budget fixed.
    if gas['charged_gas']+1000 > 15_000_000 or mutation.ANCHOR_GAS+gas['charged_gas'] > 30_000_000:
        raise ValueError('recipe exceeds fixed transaction/block gas capacity')
    row=original.make_case('call','stop',count=1,block_count=1,gas_limit=15_000_000)
    program=row['request']['program']; routes=dict.fromkeys(program['expected_route_counts'],0); routes.update(counts)
    program['expected_route_counts']=routes; program['transactions']['recipients']=[OWNER]
    program['prestate_contracts']=[dict(address=OWNER,code_hex='0x'+code.hex(),initial_storage={})]
    if operation=='staticcall': program['prestate_contracts'].append(dict(address=CHILD,code_hex='0x00',initial_storage={}))
    sid=_sid(operation,n,length,lane)
    reserved=length==97 or n in COUNTS[operation][1]
    row.update(sample_id=sid,family='ordinary-tariffs',role='control' if n==0 or lane=='scaffold' else 'candidate',
        baseline_sample_id=_sid(operation,0,length,lane), paired_sample_id=_sid(operation,n,length,'scaffold' if lane=='target' else 'target'),
        data_role='reserved_validation' if reserved else 'fit_candidate' if n in COUNTS[operation][0] else 'engineering_only',
        holdout_only=reserved,actuals_used_for_fitting=False,
        completion_gate='Exact completed/attempted routes, successful receipts, literal result guards, charged gas and full/minimized native execution equality; independent ordered log payload/topics check.',
        scope='Finite marginal whole-proposal observations with matched scaffolding; no assigned tariff, per-transaction actual PGU, universal capacity or throughput claim.')
    row['parameters']=dict(operation=operation,repetitions=n,length=length,lane=lane,
        runtime_bytes=len(code),runtime_sha256=digest(code),embedded_payload_offset=PAYLOAD_OFFSET,
        embedded_payload_sha256=digest(PAYLOAD[:128]),expected_codesize=RUNTIME_BYTES if operation=='codesize' else None,
        expected_completed_iterations=n,expected_target_events=n if lane=='target' else 0,
        expected_staticcall_warmup_events=1 if operation=='staticcall' else 0,
        expected_child_stop_events=(1+(n if lane=='target' else 0)) if operation=='staticcall' else 0,
        expected_log_count=n if operation=='log3' and lane=='target' else 0,
        log_topic_rule='topic0=iteration index; topic1=0x22; topic2=0x33' if operation=='log3' else None,
        log_payload_sha256=digest(PAYLOAD[:length]) if operation=='log3' else None,
        codecopy_raw_gas=3+3*((length+31)//32) if operation=='codecopy' else None,
        codecopy_envelope='reserved_fallback_boundary' if length==97 else 'ordinary_raw_gas_at_most12' if operation=='codecopy' else None,
        transaction_kind='call',count_per_block=1,block_count=1,transaction_gas_limit=15_000_000,
        header_gas_limit=mutation.HEADER_GAS_LIMIT,charged_gas_budget=30_000_000,expected_anchor_gas=mutation.ANCHOR_GAS,
        expected_user_charged_gas=gas['charged_gas'],expected_block_charged_gas=mutation.ANCHOR_GAS+gas['charged_gas'],
        gas_accounting_per_user=gas,expected_user_success=True,expected_exceptional_instructions=0,
        outer_calldata_bytes=0,execution_profile='evm-gas-only',witness_policy='native actual-read minimization with full/minimized replay equality',
        scaffold_policy='Loop checks/jumps scale with N. Scaffold substitutes result PUSH for CODESIZE, operand POPs for COPY/LOG, or operand POPs+success PUSH for STATICCALL; exact routes/gas disclose differences.')
    return row


def prepare(output, *, model_id=MODEL):
    if type(model_id) is not str or re.fullmatch(r'[0-9a-f]{64}',model_id) is None: raise ValueError('model_id must be lowercase sha256')
    plans=[make_case(op,repetitions=n,length=length,lane=lane) for op in OPCODES for length in LENGTHS[op] for counts in COUNTS[op] for n in counts for lane in ('target','scaffold')]
    # First fallback boundary is deliberately excluded from the ordinary fit.
    plans += [make_case('codecopy',repetitions=n,length=97,lane=lane) for n in (0,1) for lane in ('target','scaffold')]
    output=Path(output); output.mkdir(parents=True,exist_ok=False); (output/'requests').mkdir()
    rows=[]
    for row in plans:
        raw=encoded(row.pop('request')); path=f"requests/{row['sample_id']}.json"; write(output/path,raw)
        rows.append(row | dict(request_path=path,request_sha256=digest(raw)))
    sources=('ordinary_tariffs.py','initcode_preparation.py','top_level_create.py','storage_mutation.py','spawn_memory.py')
    write(output/'catalog.json',encoded(dict(schema_version=1,frozen_model_id=model_id,
        purpose='Frozen finite ordinary-opcode fit/reserved catalog; native qualification precedes actual measurement.',
        source_sha256={name:digest(Path(__file__).with_name(name).read_bytes()) for name in sources},
        count_roles={op:dict(fit=list(fit),reserved_validation=list(reserved)) for op,(fit,reserved) in COUNTS.items()},
        interpretation='Keep N0 and both lanes. Delta includes disclosed operand/result checks and loop scaffolding. Reserved inputs cannot enter fit. Retain native failures explicitly; do not raise gas caps or retry SP1 automatically.',
        cases=rows)))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('--output-dir',type=Path,required=True); parser.add_argument('--model-id',default=MODEL)
    args=parser.parse_args(); print(json.dumps(dict(prepared=prepare(args.output_dir,model_id=args.model_id))))
