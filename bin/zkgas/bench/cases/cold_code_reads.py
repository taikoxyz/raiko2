#!/usr/bin/env python3
"""Generate12 bounded cold-account code-read probes without compiler or new builds."""
import argparse
import inspect
import json
from pathlib import Path
import re

import initcode_preparation
from initcode_preparation import _Factory
import spawn_memory
from spawn_memory import digest, encoded, write
import storage_fanout
from storage_fanout import MODEL
import storage_mutation as mutation
import top_level_create as original

CORPUS = 'unzen-cold-code-reads-20261104'
OPERATIONS = {'balance': 0x31, 'size': 0x3b}
FOOTPRINTS = (16, 128, 512)
LEVELS = ('zero', 'selected')
BASE_ADDRESS = 0x4000000000000000000000000000000000010000
# Constructor overhead_prestate/candidate_transactions and call_recipient default.
PRIMARY = '0x' + '66' * 20
CODE_BYTES = 24576
ANCHOR_GAS = 167355
BLOCK_BUDGET = 30_000_000
HEADER_GAS_LIMIT = 31_000_000
TRANSACTION_GAS_CAP = 15_000_000
EIP7825_GAS_CAP = 16_777_216
REQUEST_BYTES_CAP = 32 * 1024 * 1024


def _validate(operation, footprint, visits=0):
    if type(operation) is not str or operation not in OPERATIONS: raise ValueError('operation must be balance or size')
    if type(footprint) is not int or footprint not in FOOTPRINTS: raise ValueError('footprint must be16,128 or512')
    original.integer(visits, 0, footprint, 'visits')


def target_code(index):
    original.integer(index, 0, 511, 'target index')
    return b'\x7f' + (index + 1).to_bytes(32, 'big') + b'\0' + b'\x5b' * (CODE_BYTES - 34)


def _runtime(operation, footprint, visits):
    """One read site, fixed immediate widths, memory counter and exact result guards."""
    _validate(operation, footprint, visits); asm = _Factory()
    def load(times=1): asm.push(0, times=times); asm.op(0x51, times)
    def require(value, times=1):
        asm.push(value, 32, times); asm.op(0x14, times); mutation._require(asm, times)
    asm.push(0, 32); asm.push(0); asm.op(0x52)
    checks = visits + 1; asm.label('loop', checks)
    load(checks); asm.push(visits, 32, checks); asm.op(0x14, checks); asm.target('done', checks); asm.op(0x57, checks)
    load(visits); asm.push(BASE_ADDRESS, 20, visits); asm.op(1, visits); asm.op(OPERATIONS[operation], visits)
    require(0 if operation == 'balance' else CODE_BYTES, visits)
    load(visits); asm.push(1, times=visits); asm.op(1, visits); asm.push(0, times=visits); asm.op(0x52, visits)
    asm.target('loop', visits); asm.op(0x56, visits)
    asm.label('done'); load(); require(visits)
    # All checks consume their complete stack; independent byte execution verifies emptiness.
    asm.op(0); asm.label('failure', 0); asm.op(0xfe, 0)
    return asm.finish(), asm.counts


def gas_accounting(operation, footprint, visits):
    """Frozen pre-Amsterdam EIP2929 cold gas; witness preloading does not warm accounts."""
    _validate(operation, footprint, visits); code, counts = _runtime(operation, footprint, visits)
    costs = {0: 0, 1: 3, 0x14: 3, 0x15: 3, 0x31: 0, 0x3b: 0, 0x51: 3,
        0x52: 3, 0x56: 8, 0x57: 10, 0x5b: 1, 0xfe: 0}
    ordinary = sum(count * (3 if 0x60 <= (op := int(key[-2:], 16)) <= 0x7f else costs[op]) for key, count in counts.items())
    cold = 2600 * visits; gross = 21000 + ordinary + 3 + cold
    return dict(intrinsic_gas=21000, ordinary_opcode_gas=ordinary, memory_expansion_gas=3,
        cold_account_gas=cold, warm_account_gas=0, gross_gas=gross, charged_gas=gross,
        refund_counter=0, refund_applied=0, runtime_bytes=len(code))


def make_case(operation='size', footprint=128, level='selected'):
    _validate(operation, footprint)
    if type(level) is not str or level not in LEVELS: raise ValueError('level must be zero or selected')
    visits = 0 if level == 'zero' else footprint; gas = gas_accounting(operation, footprint, visits)
    envelope = max(gas_accounting(op, footprint, footprint)['gross_gas'] for op in OPERATIONS) + 1000
    if not (envelope <= TRANSACTION_GAS_CAP and envelope <= EIP7825_GAS_CAP
        and ANCHOR_GAS + gas['charged_gas'] <= BLOCK_BUDGET and HEADER_GAS_LIMIT - ANCHOR_GAS >= envelope):
        raise ValueError('case exceeds transaction, header prefix or charged block gas cap')
    code, counts = _runtime(operation, footprint, visits)
    request = original.make_case('call', 'stop', count=1, gas_limit=envelope)['request']; program = request['program']
    routes = dict.fromkeys(program['expected_route_counts'], 0); routes.update(counts)
    contracts = [dict(address=f'0x{BASE_ADDRESS + index:040x}', code_hex='0x' + target_code(index).hex(), initial_storage={}) for index in range(footprint)]
    program.update(code_hex='0x' + code.hex(), prestate_contracts=contracts, expected_route_counts=routes)
    prefix = f'cold-code-read-{operation}-k{footprint}'; block_gas = ANCHOR_GAS + gas['charged_gas']
    return dict(sample_id=f'{prefix}-n{visits}', family='cold-code-reads', corpus_id=CORPUS,
        role='control' if visits == 0 else 'candidate', baseline_sample_id=f'{prefix}-n0',
        data_role='unfitted_cold_code_reads_validation', holdout_only=True, actuals_used_for_fitting=False,
        completion_gate='Independent byte intent, canonical witness minimality, signatures, account/code/root, native receipts and original quote gates precede actuals.',
        scope='Selected footprint unsaturated pilot. K512 is a fixture cap, not a gas maximum or protocol bound. Whole inputs differ in code, account proofs and witness; no isolated-byte PGU or price conclusion.',
        parameters=dict(operation=operation, footprint=footprint, visits=visits, workload_level=level,
            prestate_target_count=footprint, visited_target_count=visits, target_base_address=f'0x{BASE_ADDRESS:040x}',
            target_code_bytes=CODE_BYTES, target_code_pattern='push-tag-stop-jumpdest',
            target_code_sha256=[digest(bytes.fromhex(c['code_hex'][2:])) for c in contracts],
            primary_code_owner_address=PRIMARY, primary_code_owner_count=1, all_user_transactions=1,
            user_transactions_per_block=1, all_transactions_including_anchor=2, transaction_kind='call',
            block_count=1, outer_calldata_bytes=0, aggregate_state_size=0, state_size_per_account=0,
            expected_persistent_storage_reads=0, expected_persistent_storage_writes=0,
            expected_anchor_gas=ANCHOR_GAS, gas_accounting_per_user=gas, expected_user_gross_gas=gas['gross_gas'],
            expected_user_refund_counter=0, expected_user_refund_applied=0, expected_user_charged_gas=gas['charged_gas'],
            expected_block_charged_gas=block_gas, transaction_gas_limit=envelope, header_gas_limit=HEADER_GAS_LIMIT,
            charged_gas_budget=BLOCK_BUDGET, transaction_envelope_reserve=1000, near_full=block_gas >= 29_700_000,
            header_prefix_remaining_gas=[HEADER_GAS_LIMIT - ANCHOR_GAS], runtime_bytes=len(code), runtime_sha256=digest(code),
            expected_counter=visits, counter_memory_offset=0, memory_words=1,
            expected_terminal='STOP', expected_user_success=True, expected_user_receipt_statuses=[True],
            expected_anchor_success=True, expected_exceptional_instructions=0, execution_profile='evm-gas-only',
            initial_storage_rule='Primary program and configured target accounts have empty persistent storage. Sender/system accounts are separate domains.',
            target_account_rule='Consecutive unique deployed code owners, balance0 and empty persistent storage; setup is outside measured block, not free over lifecycle.',
            range_rule='Own zero retains all configured targets with no target reads; selected reads each cold target once, without executing target frames.'), request=request)


def source_modules():
    return {'cases/initcode_preparation.py': initcode_preparation, 'cases/spawn_memory.py': spawn_memory,
        'cases/storage_fanout.py': storage_fanout, 'cases/storage_mutation.py': mutation, 'cases/top_level_create.py': original}


def helpers():
    return {'_Factory': _Factory, 'storage_mutation._require': mutation._require,
        'top_level_create.make_case': original.make_case, 'top_level_create.integer': original.integer,
        **{f'cold_code_reads.{fn.__name__}': fn for fn in (_validate, target_code, _runtime, gas_accounting, make_case)},
        'spawn_memory.digest': digest, 'spawn_memory.encoded': encoded, 'spawn_memory.write': write}


def prepare(output: Path, *, model_id=MODEL):
    if not isinstance(output, Path): raise ValueError('output must be a Path')
    if type(model_id) is not str or re.fullmatch('[0-9a-f]{64}', model_id) is None: raise ValueError('model_id must be lowercase SHA256')
    output.mkdir(parents=True, exist_ok=False); (output / 'requests').mkdir(); rows = []
    for operation in OPERATIONS:
        for footprint in FOOTPRINTS:
            for level in LEVELS:
                row = make_case(operation, footprint, level); raw = encoded(row.pop('request'))
                if len(raw) >= REQUEST_BYTES_CAP: raise ValueError('encoded request exceeds32MiB fixture cap')
                path = f"requests/{row['sample_id']}.json"; write(output / path, raw)
                rows.append(row | {'request_path': path, 'request_sha256': digest(raw)}); del raw
    assert len({r['sample_id'] for r in rows}) == len({r['request_sha256'] for r in rows}) == 12
    write(output / 'catalog.json', encoded(dict(schema_version=1, corpus_id=CORPUS, frozen_model_id=model_id,
        purpose='Finite12 selected cold-account code reads with own-zero configured-footprint controls.',
        generator_sha256=digest(Path(__file__).read_bytes()), source_root='bin/zkgas/bench',
        source_sha256={name: digest(Path(module.__file__).read_bytes()) for name, module in source_modules().items()},
        helper_source_sha256={name: digest(inspect.getsource(fn).encode()) for name, fn in helpers().items()},
        operations=list(OPERATIONS), footprints=list(FOOTPRINTS), levels=list(LEVELS), users=1,
        holdout_only=True, actuals_used_for_fitting=False,
        primary_sources=['https://eips.ethereum.org/EIPS/eip-2929', 'https://ethereum.github.io/yellowpaper/paper.pdf'],
        native_observability='Native gas/status/routes and full/minimized execution-result replay; per-read values/order/content use independent byte intent, not direct native exports.', cases=rows)))
    return len(rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, required=True); parser.add_argument('--model-id', default=MODEL)
    args = parser.parse_args(); print(json.dumps({'prepared': prepare(args.output_dir, model_id=args.model_id)}))
