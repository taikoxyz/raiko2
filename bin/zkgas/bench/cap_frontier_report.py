#!/usr/bin/env python3
"""Read-only, identity-checked Unzen and offline node-cap sensitivity report."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path

from artifacts import MAX_INPUT, exclusive_json, regular, require
import gas_budget_report
import workload

DEFAULT_NODE_LIMITS = [0, 100_000_000, 250_000_000, 500_000_000, 1_000_000_000,
                       2_000_000_000, 3_600_000_000, 5_000_000_000, 10_000_000_000,
                       20_000_000_000, 30_000_000_000]
DEFAULT_UNZEN_LIMIT = 100_000_000


def unsigned(value, label):
    require(type(value) is int and value >= 0, f'{label} must be a nonnegative integer')
    return value


def read_bytes(path):
    regular(path)
    with Path(path).open('rb') as source:
        data = source.read(MAX_INPUT + 1)
    require(len(data) <= MAX_INPUT, 'artifact exceeds byte limit')
    return data


def file_record(path, raw, display_path=None):
    return dict(path=str(path) if display_path is None else display_path,
                sha256=hashlib.sha256(raw).hexdigest(), byte_length=len(raw))


def checked_file(base, record):
    require(isinstance(record['path'], str) and record['path'], 'missing artifact path')
    path = base / record['path']
    raw = read_bytes(path)
    source = file_record(path, raw, record['path'])
    require(source['sha256'] == workload.sha(record['sha256']), 'artifact identity mismatch')
    return raw, source


def checked_json(base, record):
    raw, source = checked_file(base, record)
    return json.loads(raw), source


def checked_input(base, record, *, natural=False):
    # Compression is explicit, not inferred from the filename. The compressed and
    # decoded identities both remain bound to the original source file.
    compression = 'none' if natural else record['compression']
    require(compression in ('none', 'gzip'), 'unsupported input compression')
    file_hash = record['sha256'] if natural else record['file_sha256']
    raw, source = checked_file(base, dict(path=record['path'], sha256=file_hash))
    if compression == 'gzip':
        # GzipFile reads a bounded decoded stream, including concatenated members.
        import io
        with gzip.GzipFile(fileobj=io.BytesIO(raw)) as decoded:
            data = decoded.read(MAX_INPUT + 1)
        require(len(data) <= MAX_INPUT, 'decoded input exceeds byte limit')
    else:
        data = raw
    length = unsigned(record['byte_length'], 'input byte length')
    require(length > 0, 'empty canonical input')
    canonical = hashlib.sha256(data).hexdigest(), len(data)
    require(canonical == (workload.sha(record['sha256']), length), 'canonical input identity mismatch')
    source.update(compression=compression, canonical_sha256=canonical[0], canonical_byte_length=canonical[1])
    return canonical, source


def validate_contract(contract):
    require(contract['execution_profile'] in workload.EXECUTION_PROFILES, 'unknown execution profile')
    require(isinstance(contract['model_id'], str) and contract['model_id'], 'empty model ID')
    for name in ('quote', 'sp1', 'elf', 'program_manifest'):
        workload.sha(contract['identities'][name])


def quote_key(key):
    require(set(key) == {'corpus_id', 'sample_id', 'guest_input_sha256'}, 'invalid quote key fields')
    require(isinstance(key['corpus_id'], str) and key['corpus_id'], 'empty corpus ID')
    workload.sample_id(key['sample_id'])
    workload.sha(key['guest_input_sha256'])
    return key['corpus_id'], key['sample_id'], key['guest_input_sha256']


def raw_unzen(quote):
    blocks = quote['quote']['blocks']
    count = unsigned(quote['proposal']['block_count'], 'quote block count')
    require(count == len(blocks) and count > 0, 'quote block count mismatch')
    values = [unsigned(block['node']['unzen_zkgas'], 'per-block raw Unzen') for block in blocks]
    total = workload.number(quote['quote']['node']['unzen_zkgas_sum'])
    require(sum(values) == total, 'raw Unzen sum mismatch')
    return values, total


def complete_row(base, sample, contract):
    canonical, input_source = checked_input(base, sample['input'])
    native, native_source = checked_json(base, sample['native'])
    quote, quote_source = checked_json(base, sample['quote'])
    actual, actual_source = checked_json(base, sample['actual'])
    per_row_contract = dict(contract, corpus_id=sample['quote_key']['corpus_id'])
    workload.validate_native(native, canonical, {'kind': 'retained_input'}, per_row_contract)
    workload.validate_quote(quote, native, canonical, sample['quote_key']['sample_id'], per_row_contract)
    require(quote['key'] == sample['quote_key'], 'manifest/quote key mismatch')
    workload.validate_sp1(actual, native, canonical, contract)
    require(type(actual['exit_code']) is int and actual['exit_code'] == 0, 'invalid actual exit code')
    raw_gas = workload.positive_integer(actual['execution_report']['gas'])
    require(workload.number(actual['prover_gas']) == raw_gas*10//191, 'actual PGU normalization mismatch')
    raw, total = raw_unzen(quote)
    require(type(native['block_count']) is int, 'invalid native block count')
    measured = gas_budget_report.classify_measurement(workload.metrics(quote, actual), native)
    require(len(raw) == measured['block_count'], 'quote/native block count mismatch')
    return dict(measured, guest_input_sha256=canonical[0], raw_unzen_per_block=raw,
        raw_unzen_sum=total, raw_unzen_unit='unzen_zkgas (raw Unzen, not SP1 PGU)',
        actual_minus_node_pgu=measured['actual_proposal_sp1_pgu']-measured['node_literal_sum_pgu'],
        actual_minus_full_pgu=measured['actual_proposal_sp1_pgu']-measured['proposal_estimate_pgu'],
        quote=quote, actual=actual, native=native,
        sources=dict(input=input_source, native=native_source, quote=quote_source, actual=actual_source))


def frontier(rows):
    """Summarize already validated, complete rows after caller-selected cap filtering."""
    maximum = max((r['actual_proposal_sp1_pgu'] for r in rows), default=None)
    return dict(accepted_count=len(rows), max_actual_pgu=maximum,
        max_actual_sample_ids=[r['sample_id'] for r in rows if r['actual_proposal_sp1_pgu'] == maximum],
        max_positive_node_gap_pgu=max((r['node_shortfall_pgu'] for r in rows), default=None))


def acceptance(blocks, proposals, accepted_blocks, accepted_proposals):
    # An empty denominator is explicitly unavailable rather than a fabricated 0/0.
    return dict(accepted_blocks=accepted_blocks, accepted_proposals=accepted_proposals,
        block_acceptance_ratio=None if not blocks else dict(numerator=accepted_blocks, denominator=blocks),
        proposal_acceptance_ratio=None if not proposals else dict(numerator=accepted_proposals, denominator=proposals))


def natural_rows(base, natural, contract):
    manifest, manifest_source = checked_json(base, natural['input_manifest'])
    require(type(manifest['schema_version']) is int and manifest['schema_version'] == 1,
            'unsupported natural input manifest schema')
    corpus = manifest['corpus_id']
    require(isinstance(corpus, str) and corpus, 'empty natural corpus ID')
    samples = manifest['samples']
    require(isinstance(samples, list), 'natural samples must be an array')
    raw, quotes_source = checked_file(base, natural['quotes'])
    indexed = {}
    for line in raw.splitlines():
        if not line.strip():
            continue
        quote = json.loads(line)
        key = quote_key(quote['key'])
        require(key not in indexed, 'duplicate natural quote key')
        indexed[key] = quote
    result, seen = [], set()
    input_base = (base / natural['input_manifest']['path']).parent
    for sample in samples:
        sid = workload.sample_id(sample['sample_id'])
        require(sid not in seen, 'duplicate natural sample ID')
        seen.add(sid)
        canonical, source = checked_input(input_base, sample['guest_input'], natural=True)
        key = corpus, sid, canonical[0]
        require(key in indexed, 'missing natural quote key')
        quote = indexed.pop(key)
        # Natural sensitivity joins canonical bytes to quote identities and checks
        # public-output agreement within the quote. It consumes no actual report.
        workload.validate_quote(quote, {'public_output': quote['proposal']['public_output']},
                                canonical, sid, dict(contract, corpus_id=corpus))
        prices = workload.node_prices(quote)
        raw_values, total = raw_unzen(quote)
        result.append(dict(quote_key=quote['key'], input_source=source, quote=quote,
            raw_unzen_per_block=raw_values, raw_unzen_sum=total, **prices))
    require(not indexed, 'extra natural quote key')
    return result, dict(input_manifest=manifest_source, quotes=quotes_source)


def sensitivity(rows, sources, unzen_limit, limits, covering):
    if not rows:
        return None
    blocks = [(raw, node) for row in rows
              for raw, node in zip(row['raw_unzen_per_block'], row['node_per_block_pgu'])]
    block_count, proposal_count = len(blocks), len(rows)
    old = lambda row: all(raw <= unzen_limit for raw in row['raw_unzen_per_block'])
    unzen = acceptance(block_count, proposal_count, sum(raw <= unzen_limit for raw, _ in blocks),
                       sum(old(row) for row in rows))
    sweep = []
    for limit in limits:
        fits = lambda row: all(node <= limit for node in row['node_per_block_pgu'])
        sweep.append(dict(node_limit_pgu=limit,
            **acceptance(block_count, proposal_count, sum(node <= limit for _, node in blocks),
                         sum(fits(row) for row in rows)),
            old_admitted_blocks_retained=sum(raw <= unzen_limit and node <= limit for raw, node in blocks),
            old_admitted_proposals_retained=sum(old(row) and fits(row) for row in rows)))
    return dict(block_count=block_count, proposal_count=proposal_count, unzen=unzen,
        node_frontiers=sweep, minimum_node_threshold_covering_old_admitted_blocks_pgu=covering,
        covering_threshold_scope='Minimum threshold covering this retained old-admitted block set; not a production recommendation.',
        sources=sources, rows=rows,
        scope='Existing historical block/proposal sensitivity; no transaction repacking or transaction rejection rate; no historical actuals consumed.')


def build_report(manifest_path, *, unzen_limit=DEFAULT_UNZEN_LIMIT, node_limits=None):
    unsigned(unzen_limit, 'Unzen limit')
    limits = list(DEFAULT_NODE_LIMITS if node_limits is None else node_limits)
    for limit in limits:
        unsigned(limit, 'node limit')
    manifest_path = Path(manifest_path)
    manifest_raw = read_bytes(manifest_path)
    manifest = json.loads(manifest_raw)
    require(type(manifest['schema_version']) is int and manifest['schema_version'] == 1,
            'unsupported cap manifest schema')
    contract = manifest['contract']
    validate_contract(contract)
    require(isinstance(manifest['samples'], list), 'samples must be an array')
    base = manifest_path.parent
    rows, seen_ids, complete_hashes = [], set(), set()
    for sample in manifest['samples']:
        sid = workload.sample_id(sample['sample_id'])
        require(sid not in seen_ids, 'duplicate report sample ID')
        seen_ids.add(sid)
        quote_key(sample['quote_key'])
        require(isinstance(sample['role'], str) and sample['role'], 'empty sample role')
        status = sample['status']
        require(status in ('complete', 'screened', 'failed', 'not_attempted'), 'unknown sample status')
        row = dict(sample_id=sid, role=sample['role'], status=status, quote_key=sample['quote_key'],
                   guest_input_sha256=sample['quote_key']['guest_input_sha256'])
        if 'failure' in sample:
            row['failure'] = sample['failure']
        if status == 'complete':
            row.update(complete_row(base, sample, contract))
            require(row['guest_input_sha256'] not in complete_hashes, 'duplicate complete canonical input')
            complete_hashes.add(row['guest_input_sha256'])
        else:
            row.update(actual=None, actual_proposal_sp1_pgu=None, ineligible_reason='unmeasured',
                       priority=None, node_shortfall_pgu=None, proposal_shortfall_pgu=None)
        rows.append(row)
    natural, sources = natural_rows(base, manifest['natural'], contract) if 'natural' in manifest else ([], None)
    covering = max((node for row in natural for raw, node in
                    zip(row['raw_unzen_per_block'], row['node_per_block_pgu']) if raw <= unzen_limit), default=None)
    if covering is not None:
        limits.append(covering)
    limits = sorted(set(limits))
    eligible = [row for row in rows if row['status'] == 'complete' and row['ineligible_reason'] is None]
    unzen = frontier([row for row in eligible if row['raw_unzen_per_block'][0] <= unzen_limit])
    sweep = [dict(node_limit_pgu=limit, **frontier([row for row in eligible
              if row['node_literal_sum_pgu'] <= limit])) for limit in limits]
    return dict(schema_version=1, contract=contract, source_manifest=file_record(manifest_path, manifest_raw),
        gas_budget_evm=gas_budget_report.GAS_BUDGET, unzen_limit_raw=unzen_limit,
        unzen_frontier=unzen, node_frontiers=sweep, rows=rows,
        counts=dict(samples=len(rows), complete=len(complete_hashes), eligible_single_block=len(eligible)),
        natural_sensitivity=sensitivity(natural, sources, unzen_limit, limits, covering),
        scope='Finite same-program execute-only observations, not proofs or upper bounds. Block actual maxima include only single-block proposals with receipt gas <=30000000. Multi-block quotes preserve whole-proposal actual PGU and give no per-block actual PGU. Raw Unzen and node PGU are distinct units; offline thresholds are not a production recommendation.')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--unzen-limit', type=int, default=DEFAULT_UNZEN_LIMIT)
    parser.add_argument('--node-limit', type=int, action='append')
    args = parser.parse_args(argv)
    exclusive_json(args.output, build_report(args.manifest, unzen_limit=args.unzen_limit, node_limits=args.node_limit))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
