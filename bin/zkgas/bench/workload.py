#!/usr/bin/env python3
"""Finite, sequential workload screening and execute-only SP1 measurements (stdlib only)."""
import argparse
from contextlib import contextmanager
from fractions import Fraction
import fcntl
import json
import math
import os
from pathlib import Path
import re
import shutil
import sys
import time
import uuid

from artifacts import (MAX_INPUT, MIB, compress, copy_exclusive, exclusive_json, identity,
                       read_json, receipt, require, restore, safe_child, validate_artifacts)
from process_guard import StageFailure, interruption_handlers, run

DEFAULT_LIMITS = {'pss_mib': 6144, 'min_free_mib': 256, 'output_mib': 192,
                  'log_mib': 16, 'timeout_seconds': 600}
SUCCESS = ('complete', 'screened')
FAILURES = ('native_rejected', 'quote_failed', 'measurement_failed', 'guard_failed',
            'validation_failed', 'stage_failed', 'interrupted', 'retention_failed')
EXECUTION_PROFILES = ('legacy-unzen', 'evm-gas-only')


class RetainedFailure(ValueError):
    """A failed attempt whose artifacts, source and contract have all been validated."""
    def __init__(self, folder, status):
        self.folder, self.status = folder, status
        super().__init__('failed attempt retained; use --retry-failed to create a new attempt')


def sha(value):
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value),
            'SHA256 must be 64 lowercase hexadecimal digits')
    return value


def sample_id(value):
    require(isinstance(value, str) and re.fullmatch('[A-Za-z0-9][A-Za-z0-9_.-]{0,127}', value),
            'sample ID must be a portable single path component')
    return value


def positive_integer(value):
    require(type(value) is int and value > 0, 'expected positive integer')
    return value


def number(value):
    require(type(value) is int or isinstance(value, str) and re.fullmatch('[0-9]+', value),
            'expected unsigned decimal integer')
    result = int(value)
    require(result >= 0, 'negative quantity')
    return result


def public_bytes(value):
    if isinstance(value, str):
        raw = bytes.fromhex(value.removeprefix('0x'))
    else:
        require(isinstance(value, list) and all(type(v) is int and 0 <= v <= 255 for v in value),
                'invalid public values')
        raw = bytes(value)
    require(len(raw) == 32, 'Shasta public output must have 32 bytes')
    return raw.hex()


def validate_program_manifest(manifest, profile, elf_sha256):
    require(type(manifest.get('schema_version')) is int and manifest['schema_version'] == 1,
            'unsupported program manifest schema')
    require(manifest.get('execution_profile') == profile,
            'program manifest execution profile mismatch')
    require(manifest.get('elf_sha256') == elf_sha256, 'program manifest ELF identity mismatch')
    require(manifest.get('sp1_sdk_version') == '6.3.0', 'program manifest SDK identity mismatch')


def validate_report_profile(report, contract):
    expected = contract.get('execution_profile', 'legacy-unzen')
    require(expected in EXECUTION_PROFILES, 'unknown execution profile in contract')
    actual = report.get('execution_profile')
    if actual is None:
        require(expected == 'legacy-unzen', 'missing execution profile in report')
    else:
        require(actual == expected, 'report execution profile mismatch')


def load_config(path):
    config = read_json(path)
    require(config['schema_version'] == 1, 'unsupported configuration schema')
    require(isinstance(config['corpus_id'], str) and config['corpus_id'], 'empty corpus ID')
    require(isinstance(config['model_id'], str) and config['model_id'], 'empty model ID')
    frozen = {'schema_version': 1, 'corpus_id': config['corpus_id'], 'model_id': config['model_id'],
              'identities': {}, 'measurement_kind': 'local execute-only SP1; not a proof',
              'production_sdk_parity_verified': False}
    if 'execution_profile' in config:
        require(config['execution_profile'] in EXECUTION_PROFILES, 'unknown execution profile')
        frozen['execution_profile'] = config['execution_profile']
    has_manifest = 'program_manifest' in config['artifacts']
    require(frozen.get('execution_profile') != 'evm-gas-only' or has_manifest,
            'experimental execution profile requires a bound program manifest')
    require(not has_manifest or 'execution_profile' in frozen,
            'program manifest requires an explicit execution profile')
    paths = {}
    names = ('native', 'quote', 'sp1', 'elf', 'model') + (('program_manifest',) if has_manifest else ())
    for name in names:
        item = config['artifacts'][name]
        source = (path.parent / item['path']).resolve(strict=True)
        expected = sha(item['sha256'])
        require(identity(source)[0] == expected, f'frozen {name} hash mismatch')
        if name in ('native', 'quote', 'sp1'):
            require(os.access(source, os.X_OK), f'{name} is not executable')
        if name == 'elf':
            require(identity(source)[1] <= MAX_INPUT, 'ELF exceeds 64 MiB')
        if name == 'model':
            require(read_json(source)['artifact_sha256'] == config['model_id'],
                    'model artifact metadata ID differs from configured model ID')
        if name == 'program_manifest':
            require(identity(source)[1] <= MIB, 'program manifest exceeds 1 MiB')
            validate_program_manifest(read_json(source), frozen['execution_profile'],
                                      frozen['identities']['elf'])
        paths[name] = source
        frozen['identities'][name] = expected
    limits = dict(DEFAULT_LIMITS, **config.get('limits', {}))
    require(set(limits) == set(DEFAULT_LIMITS), 'unknown resource option')
    for value in limits.values():
        require(type(value) in (int, float) and math.isfinite(value) and value > 0,
                'resource limits must be finite and positive')
    for key in ('pss_mib', 'min_free_mib', 'output_mib', 'log_mib'):
        positive_integer(limits[key])
    require(limits['pss_mib'] <= 6144 and limits['min_free_mib'] >= 256,
            'limits cannot weaken 6 GiB PSS / 256 MiB disk floor')
    require(limits['output_mib'] <= 192 and limits['log_mib'] <= 16,
            'output/log limits exceed supported bounds')
    frozen['limits'] = limits
    # Preserve source provenance supplied by the operator without interpreting it as authority.
    frozen['provenance'] = config.get('provenance', {})
    return paths, frozen


def recheck(paths, contract):
    for name, path in paths.items():
        require(identity(path)[0] == contract['identities'][name], f'{name} changed during run')


def load_catalog(path):
    catalog = read_json(path)
    require(catalog['schema_version'] == 1, 'unsupported catalog schema')
    require(isinstance(catalog['cases'], list) and catalog['cases'], 'empty catalog')
    ids = set()
    for case in catalog['cases']:
        sid = sample_id(case['sample_id'])
        require(sid not in ids, 'duplicate sample ID')
        ids.add(sid)
        relative = Path(case['request_path'])
        require(not relative.is_absolute() and '..' not in relative.parts,
                'catalog request path must stay relative to the catalog')
        source = (path.parent / relative).resolve(strict=True)
        require(source.is_relative_to(path.parent.resolve()), 'request escapes catalog directory')
        require(identity(source, limit=MAX_INPUT)[0] == sha(case['request_sha256']),
                f'request identity mismatch: {sid}')
    return catalog['cases']


def validate_native(native, input_identity, source, contract=None):
    require(native['schema_version'] == 1, 'unsupported native report schema')
    validate_report_profile(native, contract or {})
    require(native['guest_input_sha256'] == input_identity[0] and
            native['guest_input_bincode_length'] == input_identity[1], 'native/input identity mismatch')
    require(public_bytes(native['public_values']) == public_bytes(native['public_output']),
            'native public values mismatch')
    if source['kind'] == 'request':
        require(native['construction_request_sha256'] == source['case']['request_sha256'],
                'native construction request mismatch')


def validate_quote(quote, native, input_identity, sid, contract):
    require(quote['status'] == 'complete', 'quote rejected input')
    require(quote['schema_version'] == 1, 'unsupported quote schema')
    validate_report_profile(quote, contract)
    validate_report_profile(quote['quote'], contract)
    require(quote['key'] == {'corpus_id': contract['corpus_id'], 'sample_id': sid,
                           'guest_input_sha256': input_identity[0]}, 'quote key mismatch')
    require(quote['guest_input_byte_length'] == input_identity[1], 'quote input length mismatch')
    require(quote['executable']['sha256'] == contract['identities']['quote'], 'quote executable mismatch')
    require(quote['calibration_profile'] == quote['quote']['model_id'] == contract['model_id'],
            'quote model mismatch')
    require(public_bytes(quote['proposal']['public_output']) == public_bytes(native['public_output']),
            'native/quote output mismatch')
    require(number(quote['quote']['node']['block_local_known_pgu']) > 0 and
            number(quote['quote']['prove']['total_pgu']) > 0, 'zero pricing denominator')


def validate_sp1(report, native, input_identity, contract):
    require(report['schema_version'] == 1 and report['exit_code'] == 0, 'SP1 guest did not complete')
    manifest_sha256 = contract['identities'].get('program_manifest')
    require(contract.get('execution_profile') != 'evm-gas-only' or manifest_sha256,
            'experimental SP1 validation requires a bound program manifest')
    validate_report_profile(report, contract)
    require(report.get('program_manifest_sha256') == manifest_sha256,
            'SP1 program manifest identity mismatch')
    require(report['measurement_policy'] == 'sp1-6.3-streaming-v1' and
            report['sp1_execution_engine'] == 'gas-estimator', 'unexpected measurement policy')
    for key in ('sp1_sdk_version', 'sp1_core_executor_version', 'sp1_core_executor_runner_version'):
        require(report[key] == '6.3.0', 'unexpected SDK identity')
    opts = report['executor_opts']
    require(opts['memory_limit'] == 24 * 1024**3 and opts['gas_trace_chunk_threshold'] == 134217728 and
            opts['gas_trace_chunk_slots'] == 2 and opts['shard_size'] == 1 << 24 and
            opts['minimal_trace_chunk_threshold'] == 16777216 and opts['trace_chunk_slots'] == 5 and
            opts['sharding_threshold'] == {'element_threshold': 402653184, 'height_threshold': 4194304},
            'unexpected SP1 executor options')
    require(report['guest_input_sha256'] == input_identity[0] and
            report['guest_input_bincode_length'] == input_identity[1], 'SP1/input identity mismatch')
    require(report['elf_sha256'] == contract['identities']['elf'] and
            report['executable_sha256'] == contract['identities']['sp1'], 'SP1 executable/program mismatch')
    require(public_bytes(report['public_values']) == public_bytes(native['public_output']),
            'SP1/native public output mismatch: program/input qualification failed')
    require(number(report['prover_gas']) > 0, 'missing/zero actual PGU')
    for key in ('total_instruction_count', 'total_syscall_count'):
        number(report[key])


def metrics(quote, report):
    actual = number(report['prover_gas'])
    node = number(quote['quote']['node']['block_local_known_pgu'])
    full = number(quote['quote']['prove']['total_pgu'])
    return {'actual_sp1_pgu': actual, 'node_known_pgu': node, 'prove_total_pgu': full,
            'actual_over_node': {'numerator': actual, 'denominator': node},
            'actual_over_full': {'numerator': actual, 'denominator': full},
            'scope': 'proposal actual PGU / sum of node block-local known PGU; not per-block actual'}


def read_attempt(folder):
    require(folder.is_dir() and not folder.is_symlink(), 'invalid attempt directory')
    status = read_json(folder / 'status.json')
    require(status['schema_version'] == 1 and isinstance(status['artifacts'], list), 'invalid attempt status')
    require(status['sample_id'] == folder.parent.name and status['attempt_id'] == folder.name,
            'attempt/sample directory identity mismatch')
    require(read_json(folder / 'started.json') == {'schema_version': 1, 'source': status['source'],
                                                'contract': status['contract']}, 'started/status mismatch')
    validate_artifacts(folder, status['artifacts'])
    manifest_sha256 = status['contract']['identities'].get('program_manifest')
    manifest_path = folder / 'program-manifest.json'
    if manifest_sha256 and (status['status'] in SUCCESS or manifest_path.exists()):
        require(identity(manifest_path, limit=MIB)[0] == manifest_sha256,
                'retained program manifest identity mismatch')
        validate_program_manifest(read_json(manifest_path), status['contract']['execution_profile'],
                                  status['contract']['identities']['elf'])
    if status['status'] in SUCCESS:
        record = next(r for r in status['artifacts'] if r['path'] == 'input.bin.gz')
        input_identity = record['uncompressed_sha256'], record['uncompressed_length']
        native = read_json(folder / 'native.json')
        quote = read_json(folder / 'quote.jsonl')
        validate_native(native, input_identity, status['source'], status['contract'])
        validate_quote(quote, native, input_identity, status['sample_id'], status['contract'])
        if status['status'] == 'complete':
            report = read_json(folder / 'sp1.json')
            validate_sp1(report, native, input_identity, status['contract'])
            require(status['metrics'] == metrics(quote, report), 'stored metrics mismatch')
            require(read_json(folder / 'observation.json') == observation(status, quote, report),
                    'stored observation mismatch')
    elif (folder / 'failure-observation.json').exists():
        require(read_json(folder / 'failure-observation.json') == failed_observation(status),
                'stored failed observation mismatch')
    return status


def observation(status, quote, report):
    result = {'status': 'complete', 'observation_id': status['sample_id'],
            'attempt_id': status['attempt_id'], 'quote_key': quote['key'], 'backend': 'sp1',
            'workload': {'value': str(number(report['prover_gas'])), 'unit': 'sp1_pgu'},
            'measurement_stage': 'execute', 'program': {'elf_sha256': report['elf_sha256']},
            'backend_version': report['sp1_sdk_version'],
            'software_version': report['executable_sha256'],
            'source': {'measurement_policy': report['measurement_policy'],
                       'production_sdk_parity_verified': False}}
    if 'execution_profile' in status['contract']:
        result['source']['execution_profile'] = status['contract']['execution_profile']
    if 'program_manifest' in status['contract']['identities']:
        result['source']['program_manifest_sha256'] = status['contract']['identities']['program_manifest']
    return result


def failed_observation(status):
    failure = status.get('failure') or {'stage': 'retention', 'message': '; '.join(status['retention_errors'])}
    return {'status': 'failed', 'observation_id': status['sample_id'],
            'attempt_id': status['attempt_id'],
            'failure': {'code': status['status'], 'message': failure['message']},
            'source': {'stage': failure.get('stage', 'unknown')}}


@contextmanager
def output_lock(root):
    require(not root.is_symlink(), 'refusing symlink output root')
    root.mkdir(parents=True, exist_ok=True)
    path = safe_child(root, '.workload.lock')
    with path.open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def previous(sample, source, contract, retry):
    results = []
    for folder in sorted(sample.glob('attempt-*')):
        try:
            status = read_attempt(folder)
        except (OSError, ValueError, KeyError, StopIteration):
            # Corrupt completed artifacts are never silently retried; only incomplete attempts
            # without a readable success declaration may be explicitly retried.
            try:
                declared = read_json(folder / 'status.json').get('status')
            except (OSError, ValueError):
                declared = None
            require(declared not in SUCCESS, 'corrupt successful attempt; investigate before resuming')
            require(retry, 'partial/corrupt attempt retained; use --retry-failed')
            continue
        require(status['source'] == source and status['contract'] == contract,
                'sample source/contract changed; use a fresh output root')
        results.append((folder, status))
    completed = [result for result in results if result[1]['status'] == 'complete']
    require(len(completed) <= 1, 'multiple completed attempts require investigation')
    if completed:
        return completed[0]
    screened = [result for result in results if result[1]['status'] == 'screened']
    failures = [result for result in results if result[1]['status'] != 'screened']
    if failures and not retry:
        require(all(result[1]['status'] in FAILURES for result in failures),
                'unknown retained attempt status; investigate before resuming')
        raise RetainedFailure(*failures[-1])
    if screened:
        return screened[-1]
    return None


def artifact_receipts(folder, compressed_records):
    records = {r['path']: r for r in compressed_records}
    for path in sorted(folder.iterdir()):
        if path.name not in ('started.json', 'status.json') and path.name not in records:
            records[path.name] = receipt(path)
    return list(records.values())


def execute_source(root, source, paths, contract, through, retry=False, catalog_dir=None):
    sid = sample_id(source['case']['sample_id'] if source['kind'] == 'request' else source['sample_id'])
    sample = safe_child(root, sid)
    sample.mkdir(exist_ok=True)
    prior = previous(sample, source, contract, retry)
    if prior and (prior[1]['status'] == 'complete' or through == 'screen'):
        return {'sample_id': sid, 'status': 'validated_resume', 'attempt': prior[0].name}
    folder = sample / (f'attempt-{time.time_ns():020d}-' + uuid.uuid4().hex)
    folder.mkdir()
    status = {'schema_version': 1, 'sample_id': sid, 'attempt_id': folder.name,
              'source': source, 'contract': contract, 'status': 'running', 'artifacts': []}
    exclusive_json(folder / 'started.json', {'schema_version': 1, 'source': source, 'contract': contract})
    records, phase = [], 'setup'
    input_path, native_path, quote_path = folder / 'input.bin', folder / 'native.json', folder / 'quote.jsonl'
    profile_args = (['--execution-profile', contract['execution_profile']]
                    if 'execution_profile' in contract else [])
    try:
        require(shutil.disk_usage(folder).free >= contract['limits']['min_free_mib'] * MIB + 5 * MAX_INPUT,
                'insufficient disk for bounded input/output plus verified retention')
        recheck(paths, contract)
        manifest_path = None
        if 'program_manifest' in paths:
            manifest_path = folder / 'program-manifest.json'
            copy_exclusive(paths['program_manifest'], manifest_path,
                           contract['identities']['program_manifest'])
        if prior:
            origin, old = prior
            record = next(r for r in old['artifacts'] if r['path'] == 'input.bin.gz')
            restore(origin / 'input.bin.gz', input_path, record)
            copy_exclusive(origin / 'native.json', native_path)
            copy_exclusive(origin / 'quote.jsonl', quote_path)
        elif source['kind'] == 'request':
            request = folder / 'request.json'
            copy_exclusive(catalog_dir / source['case']['request_path'], request,
                           source['case']['request_sha256'])
            phase = 'native'
            run([str(paths['native']), '--input', str(request), '--output', str(folder / 'guest-input.json'),
                 '--output-bincode', str(input_path), '--report', str(native_path)] + profile_args, folder, phase,
                contract['limits'], [input_path, native_path, folder / 'guest-input.json'])
        else:
            copy_exclusive(Path(source['input_path']), input_path, source['input_sha256'])
            copy_exclusive(Path(source['native_report_path']), native_path, source['native_report_sha256'])
        phase = 'validation'
        input_identity = identity(input_path, limit=MAX_INPUT)
        native = read_json(native_path)
        validate_native(native, input_identity, source, contract)
        if not prior:
            phase = 'quote'
            run([str(paths['quote']), '--input-bincode', str(input_path), '--corpus-id', contract['corpus_id'],
                 '--sample-id', sid, '--output', str(quote_path)] + profile_args,
                folder, phase, contract['limits'], [quote_path])
        phase = 'validation'
        quote = read_json(quote_path)
        validate_quote(quote, native, input_identity, sid, contract)
        if through == 'measure':
            phase = 'sp1'
            report_path = folder / 'sp1.json'
            resource_path = folder / 'sp1-resources.json'
            manifest_args = (['--program-manifest', str(manifest_path)]
                             if manifest_path is not None else [])
            run([str(paths['sp1']), '--input-bincode', str(input_path), '--elf', str(paths['elf']),
                 '--output', str(report_path), '--resource-receipt', str(resource_path)] + manifest_args, folder, phase,
                contract['limits'], [report_path, resource_path], resource_receipt=resource_path)
            phase = 'validation'
            report = read_json(report_path)
            validate_sp1(report, native, input_identity, contract)
            status['metrics'] = metrics(quote, report)
            exclusive_json(folder / 'observation.json', observation(status, quote, report))
            status['status'] = 'complete'
        else:
            status['status'] = 'screened'
        recheck(paths, contract)
    except BaseException as error:
        if isinstance(error, (KeyboardInterrupt, InterruptedError)):
            status['status'] = 'interrupted'
        elif isinstance(error, StageFailure):
            status['status'] = ('guard_failed' if error.failure != 'nonzero_exit' else
                                {'native': 'native_rejected', 'quote': 'quote_failed',
                                 'sp1': 'measurement_failed'}[error.stage])
        else:
            status['status'] = 'validation_failed' if phase == 'validation' else 'stage_failed'
        status['failure'] = {'stage': phase, 'code': type(error).__name__, 'message': str(error)}
        interrupted = isinstance(error, (KeyboardInterrupt, InterruptedError))
    else:
        interrupted = False
    # Preserve every partial artifact. Compress the large owned inputs only after verification;
    # failed compression retains originals and records its own error.
    for name in ('input.bin', 'guest-input.json'):
        path = folder / name
        if path.exists():
            try:
                records.append(compress(path))
            except Exception as error:
                status.setdefault('retention_errors', []).append(f'{type(error).__name__}: {error}')
                if status['status'] in SUCCESS:
                    status['status'] = 'retention_failed'
    status['artifacts'] = artifact_receipts(folder, records)
    if status['status'] not in SUCCESS:
        exclusive_json(folder / 'failure-observation.json', failed_observation(status))
        status['artifacts'].append(receipt(folder / 'failure-observation.json'))
    exclusive_json(folder / 'status.json', status)
    read_attempt(folder)
    if interrupted:
        raise InterruptedError('run cancelled; owned descendants terminated and attempt retained')
    return {'sample_id': sid, 'status': status['status'], 'attempt': folder.name}


def validate_root(root, strict=True):
    contract = read_json(root / 'contract.json')
    results = []
    for sample in sorted(root.iterdir()):
        if sample.is_dir():
            sample_id(sample.name)
            require(not sample.is_symlink(), 'refusing symlink sample')
            for folder in sorted(sample.glob('attempt-*')):
                try:
                    status = read_attempt(folder)
                except (OSError, ValueError, KeyError, StopIteration) as error:
                    if strict:
                        raise
                    # Report corrupt/partial evidence explicitly, and never rank its measurements.
                    results.append((folder, {'sample_id': sample.name, 'attempt_id': folder.name,
                        'status': 'invalid_or_incomplete', 'source': {}, 'contract': contract,
                        'failure': {'code': type(error).__name__, 'message': str(error)}}))
                    continue
                require(status['sample_id'] == sample.name and status['attempt_id'] == folder.name and
                        status['contract'] == contract, 'attempt/root identity mismatch')
                results.append((folder, status))
    return results


def summary(root):
    attempts = validate_root(root, strict=False)
    completed, rows = {}, []
    for folder, status in attempts:
        row = {'sample_id': status['sample_id'], 'attempt_id': folder.name, 'status': status['status'],
               'source': status['source']}
        if status['status'] == 'complete':
            sid = status['sample_id']
            require(sid not in completed, 'duplicate complete sample')
            completed[sid] = status['metrics']
            row['metrics'] = status['metrics']
        if 'failure' in status:
            row['failure'] = status['failure']
        rows.append(row)
    contract = read_json(root / 'contract.json')
    attempted = {row['sample_id'] for row in rows}
    rows.extend({'sample_id': case['sample_id'], 'status': 'not_attempted',
                 'source': {'kind': 'request', 'case': case}}
                for case in contract.get('cases', []) if case['sample_id'] not in attempted)
    ranked = sorted(completed, key=lambda sid: Fraction(completed[sid]['actual_sp1_pgu'],
                                                       completed[sid]['node_known_pgu']), reverse=True)
    observations = []
    for folder, status in attempts:
        if status['status'] == 'complete':
            observations.append(read_json(folder / 'observation.json'))
        elif status['status'] not in SUCCESS:
            observations.append(failed_observation(status))
    return {'schema_version': 1, 'objective': 'actual SP1 PGU / node-known PGU',
            'ranked_sample_ids': ranked, 'attempts': rows,
            'observations': {'schema_version': 1, 'observation_set_id': root.name,
                             'observations': observations}}


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest='action', required=True)
    for action in ('screen', 'run', 'measure'):
        sub = commands.add_parser(action)
        sub.add_argument('--config', type=Path, required=True)
        sub.add_argument('--output-root', type=Path, required=True)
        sub.add_argument('--retry-failed', action='store_true')
        if action != 'measure':
            sub.add_argument('--catalog', type=Path, required=True)
            sub.add_argument('--continue-after-failure', action='store_true',
                             help='attempt the remaining catalog after a retained failure; still exit nonzero')
        else:
            sub.add_argument('--input-bincode', type=Path, required=True)
            sub.add_argument('--native-report', type=Path, required=True)
            sub.add_argument('--sample-id', required=True)
    for action in ('verify', 'report'):
        sub = commands.add_parser(action)
        sub.add_argument('--output-root', type=Path, required=True)
        if action == 'report':
            sub.add_argument('--output', type=Path, required=True)
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    require(not args.output_root.is_symlink(), 'refusing symlink output root')
    root = args.output_root.resolve()
    if args.action in ('verify', 'report'):
        with output_lock(root):
            if args.action == 'report':
                exclusive_json(args.output, summary(root))
            else:
                print(json.dumps({'verified_attempts': len(validate_root(root))}))
        return 0
    paths, contract = load_config(args.config.resolve())
    catalog_dir = None
    if args.action == 'measure':
        input_path, native_path = args.input_bincode.resolve(), args.native_report.resolve()
        sources = [{'kind': 'retained_input', 'sample_id': sample_id(args.sample_id),
                    'input_path': str(input_path), 'input_sha256': identity(input_path, limit=MAX_INPUT)[0],
                    'native_report_path': str(native_path), 'native_report_sha256': identity(native_path)[0]}]
    else:
        catalog = args.catalog.resolve()
        catalog_dir = catalog.parent
        cases = load_catalog(catalog)
        sources = [{'kind': 'request', 'case': case} for case in cases]
        contract['catalog_sha256'] = identity(catalog)[0]
        contract['cases'] = cases
    failed = False
    with interruption_handlers(), output_lock(root):
        manifest = root / 'contract.json'
        if manifest.exists():
            require(read_json(manifest) == contract, 'output root contract changed; choose a fresh root')
        else:
            exclusive_json(manifest, contract)
        for source in sources:
            try:
                outcome = execute_source(root, source, paths, contract,
                                         'screen' if args.action == 'screen' else 'measure',
                                         args.retry_failed, catalog_dir)
            except RetainedFailure as error:
                if not getattr(args, 'continue_after_failure', False):
                    raise
                outcome = {'sample_id': error.status['sample_id'], 'status': 'retained_failure',
                           'retained_status': error.status['status'], 'attempt': error.folder.name}
            print(json.dumps(outcome), flush=True)
            if outcome['status'] not in (*SUCCESS, 'validated_resume'):
                failed = True
                # Fail fast. The frozen catalog still retains all unattempted cases.
                if not getattr(args, 'continue_after_failure', False):
                    break
    return int(failed)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, KeyError, InterruptedError) as error:
        print(f'{type(error).__name__}: {error}', file=sys.stderr)
        sys.exit(2)
