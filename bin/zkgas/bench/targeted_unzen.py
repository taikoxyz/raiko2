#!/usr/bin/env python3
"""Read-only, explicitly profiled Unzen plus staged semantic-work reserves.

The original Unzen score retains its own unit. The other stages are counterfactual
PGU-equivalent scores, not marginal calibration, production tariffs or safety bounds.
No execution, source acquisition, coefficient selection or evaluation-set fitting occurs.
"""
import argparse
from decimal import Decimal, localcontext, ROUND_HALF_EVEN
from fractions import Fraction
import gzip
import hashlib
import io
import json
from pathlib import Path

from artifacts import MAX_INPUT, exclusive_json, require
import cap_frontier_report as cap
import gas_budget_report
import workload

STAGES = ('calibrated_unzen', 'ecmul', 'access_list_linear', 'access_list_pairs', 'container')
ECMUL_FIELDS = ('successful_calls', 'successful_native_gas', 'successful_scalar_work_units',
                'failed_calls', 'failed_native_gas')
SEMANTIC_DIAGNOSTICS = ('arithmetic_overflow', 'native_gas_accounting',
                        'unbalanced_call_hooks', 'call_hook_mismatch')
ACCESS_FIELDS = ('access_list_items', 'access_list_construction_pairs',
                 'access_list_construction_other_pairs', 'access_list_address_pairs')
CONTAINER_FIELDS = ('storage_journal_work_units', 'account_journal_work_units',
                    'account_cache_work_units', 'storage_transition_work_units')
TARIFFS = dict(ecmul=('ecmul_scalar_work_pgu',), access_list_linear=(
    'access_list_item_pgu', 'access_list_key_pgu'), access_list_pairs=(
    'access_list_same_low32_pair_pgu', 'access_list_other_pair_pgu', 'access_list_address_pair_pgu'), container=(
    'storage_journal_work_pgu', 'account_journal_work_pgu',
    'account_cache_work_pgu', 'storage_transition_work_pgu'))
MAX_INTEGER = (1 << 128) - 1
DEFAULT_RESOURCE_LIMITS = dict(source_bytes=256*1024*1024, rows=10000, blocks=100000)
CONVERSION_SCOPE = ('Whole-proposal training-sum shape conversion; not a marginal conversion, '
                    'direct PGU calibration or admission-safety certification. Additive reserves '
                    'may cover work already represented by Unzen; no operation credit is removed.')


def unsigned(value, label):
    """Reject booleans, floats and unbounded selector/coefficient integers."""
    require(type(value) is int and 0 <= value <= MAX_INTEGER,
            f'{label} must be a bounded nonnegative integer')
    return value


def _object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate JSON object key')
        result[key] = value
    return result


def _json(raw):
    return json.loads(raw, object_pairs_hook=_object,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError('nonfinite JSON value')))


class ResourceBudget:
    """Preflight every read reference; duplicate references are charged and checked again."""
    def __init__(self, overrides=None):
        require(overrides is None or isinstance(overrides, dict)
                and set(overrides) <= DEFAULT_RESOURCE_LIMITS.keys(), 'invalid resource limit fields')
        self.limits = DEFAULT_RESOURCE_LIMITS | (overrides or {})
        require(all(type(value) is int and value > 0 for value in self.limits.values()),
                'resource limits must be positive integers')
        self.used = dict(source_bytes=0, rows=0, blocks=0)
        self.plans = []

    def add(self, name, amount):
        unsigned(amount, name)
        value = self.used[name]+amount
        label = {'source_bytes': 'aggregate source byte', 'rows': 'total row', 'blocks': 'total block'}[name]
        require(value <= self.limits[name], label+' budget exceeded')
        self.used[name] = value

    def plan(self, path, expected=None, *, limit=MAX_INPUT, display=None):
        cap.regular(path)
        size = Path(path).stat().st_size
        require(size <= limit, 'artifact exceeds byte limit')
        if expected is not None:
            workload.sha(expected)
        self.add('source_bytes', size)
        plan = dict(path=Path(path), size=size, expected=expected,
                    display=str(path) if display is None else display, read=False)
        self.plans.append(plan)
        return plan

    def reference(self, base, record, *, limit=MAX_INPUT):
        require(isinstance(record['path'], str) and record['path'], 'missing artifact path')
        return self.plan(base/record['path'], record['sha256'], limit=limit, display=record['path'])

    def read(self, plan):
        require(not plan['read'], 'artifact read was not preflighted separately')
        cap.regular(plan['path'])
        require(plan['path'].stat().st_size == plan['size'], 'artifact size changed after preflight')
        with plan['path'].open('rb') as source:
            raw = source.read(plan['size']+1)
        require(len(raw) == plan['size'], 'artifact size changed during read')
        digest = hashlib.sha256(raw).hexdigest()
        require(plan['expected'] is None or digest == plan['expected'], 'artifact identity mismatch')
        plan['read'] = True
        return raw, dict(path=plan['display'], sha256=digest, byte_length=len(raw))

    def json(self, plan):
        raw, source = self.read(plan)
        return _json(raw), source

    def receipt(self):
        require(all(plan['read'] for plan in self.plans), 'preflighted artifact was not checked')
        return dict(limits=self.limits, used=self.used, source_read_references=len(self.plans),
            policy=('Each encoded source read reference is stat-bounded before reading and charged '
                    'in full, including repeated paths; every expected hash is rechecked. Canonical '
                    'gzip decoding remains independently bounded at 64MiB per input. Row/block '
                    'budgets include sidecar records; historical quotes are projected one at a time.'))


def fraction_record(value):
    return dict(numerator=value.numerator, denominator=value.denominator)


def ratio(value, label):
    require(isinstance(value, dict) and set(value) == {'numerator', 'denominator'},
            f'{label} must be an explicit rational')
    numerator = unsigned(value['numerator'], label+' numerator')
    denominator = unsigned(value['denominator'], label+' denominator')
    require(numerator > 0 and denominator > 0, f'{label} must be positive')
    return Fraction(numerator, denominator)


def ceil_fraction(value):
    return (value.numerator + value.denominator - 1) // value.denominator


def canonical_quote_sha256(quote):
    """Bind one quote object even when its source is a multi-row JSONL file."""
    raw = json.dumps(quote, sort_keys=True, separators=(',', ':'), ensure_ascii=False,
                     allow_nan=False).encode()
    return hashlib.sha256(raw).hexdigest()


def read_profile(path, budget):
    profile, source = budget.json(budget.plan(path, limit=1024*1024))
    required = {'schema_version', 'baseline_id', 'comparison_model_id', 'program', 'conversion',
                'partition', 'stages', 'tariffs', 'sources', 'limits'}
    require(isinstance(profile, dict) and set(profile) == required, 'invalid baseline profile fields')
    require(type(profile['schema_version']) is int and profile['schema_version'] == 1,
            'unsupported baseline profile schema')
    for name in ('baseline_id', 'comparison_model_id'):
        require(isinstance(profile[name], str) and bool(profile[name]), 'empty '+name)
    require(profile['stages'] in [list(STAGES[:end]) for end in range(1, len(STAGES)+1)],
            'stages must be a nonempty ordered prefix')
    require(set(profile['program']) == {'elf_sha256', 'sdk_version'}, 'invalid program identity fields')
    workload.sha(profile['program']['elf_sha256'])
    require(isinstance(profile['program']['sdk_version'], str) and profile['program']['sdk_version'],
            'missing SDK version')
    partition = profile['partition']
    require(set(partition) == {'training_inputs', 'evaluation_inputs', 'evaluation_status'},
            'invalid partition fields')
    for name in ('training_inputs', 'evaluation_inputs'):
        require(isinstance(partition[name], list) and len(partition[name]) <= 10000,
                'invalid '+name)
        for value in partition[name]:
            workload.sha(value)
        require(len(set(partition[name])) == len(partition[name]), 'duplicate '+name)
    require(not set(partition['training_inputs']) & set(partition['evaluation_inputs']),
            'training and evaluation inputs overlap')
    require(partition['evaluation_status'] in ('opened_regression', 'declared_unopened'),
            'unknown evaluation status')
    conversion = profile['conversion']
    require(conversion.get('kind') in ('training_sum_ratio', 'provided_ratio'), 'unknown conversion kind')
    if conversion['kind'] == 'training_sum_ratio':
        require(set(conversion) == {'kind'} and partition['training_inputs'],
                'training-sum conversion needs explicit training inputs')
    else:
        require(set(conversion) == {'kind', 'ratio', 'purpose'}, 'invalid provided conversion')
        ratio(conversion['ratio'], 'provided conversion')
        require(isinstance(conversion['purpose'], str) and conversion['purpose'],
                'provided conversion needs a provenance explanation')
    expected_tariffs = {name for stage in profile['stages'][1:] for name in TARIFFS[stage]}
    require(set(profile['tariffs']) == expected_tariffs, 'missing or unexpected tariff')
    for name, value in profile['tariffs'].items():
        unsigned(value, name)
    require(isinstance(profile['sources'], list) and 0 < len(profile['sources']) <= 128,
            'coefficient/conversion source bindings are required')
    sources = []
    for record in profile['sources']:
        require(set(record) == {'path', 'sha256', 'purpose'}, 'invalid profile source record')
        require(isinstance(record['purpose'], str) and record['purpose'], 'source purpose is required')
        sources.append((budget.reference(Path(path).parent, record), record['purpose']))
    require(set(profile['limits']) == {'raw_unzen', 'counterfactual_pgu'}, 'invalid limits')
    for name, value in profile['limits'].items():
        unsigned(value, name)
    return profile, dict(profile=source, sources=[]), sources


def validate_ecmul(value):
    """Qualify real precompile outcomes; failed inputs never imply scalar work."""
    require(isinstance(value, dict) and set(value) == set(ECMUL_FIELDS), 'invalid ECMUL counter fields')
    result = {key: unsigned(value[key], 'ECMUL '+key) for key in ECMUL_FIELDS}
    require(all(value <= (1 << 64)-1 for value in result.values()), 'ECMUL counters exceed u64')
    require(result['successful_native_gas'] == 6000*result['successful_calls'],
            'successful ECMUL native gas/count mismatch')
    require(result['successful_scalar_work_units'] <= 507*result['successful_calls'],
            'ECMUL scalar work/count mismatch')
    require(result['failed_calls'] != 0 or result['failed_native_gas'] == 0,
            'failed ECMUL native gas without a call')
    return result


def semantic_snapshot(value):
    """Absence is unobserved; a typed diagnostic cannot coexist with partial valid counters."""
    if value is None:
        return {}
    require(isinstance(value, dict) and set(value) <= {'ecmul', 'diagnostic', 'access_list_raw_storage_keys'},
            'invalid node semantic counters')
    diagnostic = value.get('diagnostic')
    counters = value.get('ecmul')
    if diagnostic is not None:
        require(diagnostic in SEMANTIC_DIAGNOSTICS, 'unknown semantic diagnostic')
        require(counters is None, 'partial semantic counters cannot coexist with diagnostic')
        result = dict(ecmul=None, diagnostic=diagnostic)
    else:
        result = dict(ecmul=validate_ecmul(counters)) if counters is not None else {}
    if 'access_list_raw_storage_keys' in value:
        keys = value['access_list_raw_storage_keys']
        require(keys is None or unsigned(keys, 'raw storage keys') <= (1 << 64)-1,
                'raw storage key count exceeds u64')
        result['access_list_raw_storage_keys'] = keys
    return result


def read_sidecar(plan, budget):
    if plan is None:
        return {}, None
    data, source = budget.json(plan)
    require(set(data) == {'schema_version', 'scope', 'producer', 'rows'}, 'invalid selector sidecar fields')
    require(type(data['schema_version']) is int and data['schema_version'] == 1,
            'unsupported selector sidecar schema')
    require(data['scope'] == 'node_committed', 'selector sidecar must describe committed node work')
    require(set(data['producer']) == {'sha256', 'source_revision'}, 'invalid selector producer')
    workload.sha(data['producer']['sha256'])
    require(isinstance(data['producer']['source_revision'], str) and data['producer']['source_revision'],
            'missing selector source revision')
    require(isinstance(data['rows'], list) and len(data['rows']) <= 10000, 'invalid selector rows')
    budget.add('rows', len(data['rows']))
    result = {}
    for row in data['rows']:
        require(set(row) == {'guest_input_sha256', 'canonical_quote_sha256', 'blocks'},
                'invalid selector row fields')
        key = workload.sha(row['guest_input_sha256']), workload.sha(row['canonical_quote_sha256'])
        require(key not in result, 'duplicate selector row')
        require(isinstance(row['blocks'], list) and len(row['blocks']) <= 10000, 'invalid selector blocks')
        budget.add('blocks', len(row['blocks']))
        blocks = []
        for index, block in enumerate(row['blocks']):
            require(set(block) == {'block_index', 'ecmul'} and type(block['block_index']) is int
                    and block['block_index'] == index, 'invalid selector block index')
            blocks.append(validate_ecmul(block['ecmul']))
        result[key] = blocks
    return result, dict(file=source, producer=data['producer'])


def _prepare(manifests, profile, budget):
    prepared, sources = [], []
    for path in manifests:
        manifest, source = budget.json(budget.plan(path))
        require(type(manifest['schema_version']) is int and manifest['schema_version'] == 1,
                'unsupported cap manifest schema')
        contract = manifest['contract']
        cap.validate_contract(contract)
        require(contract['model_id'] == profile['comparison_model_id'], 'comparison model mismatch')
        require(contract['identities']['elf'] == profile['program']['elf_sha256'], 'program ELF mismatch')
        require(isinstance(manifest['samples'], list), 'samples must be an array')
        budget.add('rows', len(manifest['samples']))
        sources.append(source)
        base = Path(path).parent
        samples, ids = [], set()
        for sample in manifest['samples']:
            sid = workload.sample_id(sample['sample_id'])
            require(sid not in ids, 'duplicate report sample ID')
            ids.add(sid)
            cap.quote_key(sample['quote_key'])
            require(isinstance(sample['role'], str) and sample['role'], 'empty sample role')
            require(sample['status'] in ('complete', 'screened', 'failed', 'not_attempted'),
                    'unknown sample status')
            measurement = sample.get('measurement', 'native_receipt')
            require(measurement in ('native_receipt', 'quote_actual_only'), 'unknown measurement scope')
            if measurement == 'quote_actual_only':
                require('native' not in sample, 'quote_actual_only cannot supply native receipts')
                require('headers' in sample, 'quote_actual_only requires original headers')
            plans = {}
            if sample['status'] == 'complete':
                count_source = 'headers' if measurement == 'quote_actual_only' else 'native'
                for name in ('input', count_source, 'quote', 'actual'):
                    record = sample[name]
                    if name == 'input':
                        require(record['compression'] in ('none', 'gzip'), 'unsupported input compression')
                        require(0 < unsigned(record['byte_length'], 'canonical input size') <= MAX_INPUT,
                                'decoded input exceeds byte limit')
                        record = dict(path=record['path'], sha256=record['file_sha256'])
                    plans[name] = budget.reference(base, record)
                # A separate bounded native-count pass checks the total block budget before
                # reading any canonical inputs/large quotes. The second read is charged too.
                plans['block_count'] = budget.reference(base, sample[count_source])
            samples.append((sample, plans))
        natural = None
        if 'natural' in manifest:
            entry = manifest['natural']
            natural_manifest, natural_source = budget.json(budget.reference(base, entry['input_manifest']))
            require(type(natural_manifest['schema_version']) is int and natural_manifest['schema_version'] == 1,
                    'unsupported natural input manifest schema')
            require(isinstance(natural_manifest['corpus_id'], str) and natural_manifest['corpus_id'],
                    'empty natural corpus ID')
            require(isinstance(natural_manifest['samples'], list), 'natural samples must be an array')
            budget.add('rows', len(natural_manifest['samples']))
            input_base = (base/entry['input_manifest']['path']).parent
            indexed = {}
            for sample in natural_manifest['samples']:
                sid = workload.sample_id(sample['sample_id'])
                require(sid not in indexed, 'duplicate natural sample ID')
                record = sample['guest_input']
                require(0 < unsigned(record['byte_length'], 'natural input size') <= MAX_INPUT,
                        'decoded input exceeds byte limit')
                indexed[sid] = (sample, budget.reference(input_base, record))
            natural = dict(manifest=natural_manifest, source=natural_source, indexed=indexed,
                           quotes=budget.reference(base, entry['quotes']))
        prepared.append((contract, samples, natural))
    return prepared, sources


def _input(record, plan, budget, *, natural=False):
    raw, source = budget.read(plan)
    compression = 'none' if natural else record['compression']
    if compression == 'gzip':
        with gzip.GzipFile(fileobj=io.BytesIO(raw)) as stream:
            data = stream.read(MAX_INPUT+1)
        require(len(data) <= MAX_INPUT, 'decoded input exceeds byte limit')
    else:
        data = raw
    canonical = hashlib.sha256(data).hexdigest(), len(data)
    require(canonical == (workload.sha(record['sha256']), record['byte_length']),
            'canonical input identity mismatch')
    return canonical, source


def _project_quote(quote):
    """Keep only numeric baseline selectors; never retain the full quote object."""
    blocks = []
    for block in quote['quote']['blocks']:
        pricing = block['node']['pricing']
        projection = {name: unsigned(pricing[name], name) for name in ACCESS_FIELDS+CONTAINER_FIELDS
                      if name in pricing}
        counters = semantic_snapshot(block['node'].get('semantic_counters'))
        blocks.append(dict(node=dict(pricing=projection, semantic_counters=counters)))
    return dict(canonical_quote_sha256=canonical_quote_sha256(quote), baseline_blocks=blocks)


def _measured(sample, plans, contract, profile, budget):
    canonical, _ = _input(sample['input'], plans['input'], budget)
    quote_actual_only = sample.get('measurement') == 'quote_actual_only'
    native = None if quote_actual_only else budget.json(plans['native'])[0]
    quote, _ = budget.json(plans['quote'])
    count = len(quote['quote']['blocks'])
    require(type(quote['proposal']['block_count']) is int
            and quote['proposal']['block_count'] == count > 0
            and count == plans['expected_blocks']
            and (native is None or count == native['block_count']), 'quote/native block count mismatch')
    actual, _ = budget.json(plans['actual'])
    per_row = dict(contract, corpus_id=sample['quote_key']['corpus_id'])
    if native is not None:
        workload.validate_native(native, canonical, {'kind': 'retained_input'}, per_row)
    public = native if native is not None else dict(public_output=quote['proposal']['public_output'])
    workload.validate_quote(quote, public, canonical, sample['quote_key']['sample_id'], per_row)
    require(quote['key'] == sample['quote_key'], 'manifest/quote key mismatch')
    workload.validate_sp1(actual, public, canonical, contract)
    require(type(actual['exit_code']) is int and actual['exit_code'] == 0, 'invalid actual exit code')
    raw_gas = workload.positive_integer(actual['execution_report']['gas'])
    require(workload.number(actual['prover_gas']) == raw_gas*10//191, 'actual PGU normalization mismatch')
    for name in ('sp1_sdk_version', 'sp1_core_executor_version', 'sp1_core_executor_runner_version'):
        require(actual[name] == profile['program']['sdk_version'], 'actual SDK/executor mismatch')
    metrics = workload.metrics(quote, actual)
    if native is not None:
        measured = gas_budget_report.classify_measurement(metrics, native)
    else:
        headers, _ = budget.json(plans['headers'])
        gas = _observed_headers(headers, quote)
        measured = dict(actual_proposal_sp1_pgu=metrics['actual_sp1_pgu'],
            node_literal_sum_pgu=metrics['node_known_pgu'], proposal_estimate_pgu=metrics['prove_total_pgu'],
            node_per_block_pgu=metrics['node_per_block_pgu'],
            node_aggregate_ceil_pgu=metrics['node_aggregate_ceil_pgu'],
            actual_over_node=metrics['actual_over_node'], actual_over_full=metrics['actual_over_full'],
            block_count=count, block_evm_gas_used=gas, total_evm_gas_used=sum(gas),
            ineligible_reason='quote_actual_only', priority=None,
            measurement_scope='Whole-proposal quote/actual with recorded header gas; no receipt qualification.')
    raw, total = cap.raw_unzen(quote)
    require(len(raw) == measured['block_count'], 'quote/native block count mismatch')
    return dict(measured, raw_unzen_per_block=raw, raw_unzen_sum=total, **_project_quote(quote))


def _observed_headers(headers, quote):
    """Check source-bound recorded headers; they never grant receipt-qualified capacity."""
    require(isinstance(headers, dict) and isinstance(headers.get('blocks'), list), 'missing header blocks')
    blocks = headers['blocks']
    require(len(blocks) == quote['proposal']['block_count'], 'header block count mismatch')
    gas, previous = [], None
    for header, block in zip(blocks, quote['quote']['blocks']):
        number = unsigned(header['number'], 'header number')
        require(number == block['block_number'] and (previous is None or number == previous+1),
                'header block number mismatch')
        previous = number
        used = unsigned(header['gas_used'], 'header gas')
        fields = header.get('header_fields')
        require(isinstance(fields, dict), 'missing original header fields')
        for name, expected, label in (('number', number, 'header number'), ('gasUsed', used, 'header gas')):
            value = fields.get(name)
            require(isinstance(value, str) and value.startswith('0x') and len(value) <= 66,
                    'invalid '+label+' hex')
            try:
                parsed = int(value, 16)
            except ValueError as error:
                raise ValueError('invalid '+label+' hex') from error
            require(parsed == expected, label+' field mismatch')
        gas.append(used)
    return gas


def _natural(prepared, contract, budget, seen):
    if prepared is None:
        return []
    raw, _ = budget.read(prepared['quotes'])
    rows, indexed = [], prepared['indexed']
    # BytesIO iterates one line at a time; no splitlines copy or full decoded quote map.
    for line in io.BytesIO(raw):
        if not line.strip():
            continue
        quote = _json(line)
        key = cap.quote_key(quote['key'])
        require(key[0] == prepared['manifest']['corpus_id'] and key[1] in indexed,
                'extra or duplicate natural quote key')
        sample, input_plan = indexed.pop(key[1])
        require(key[2] not in seen, 'duplicate historical canonical input')
        seen.add(key[2])
        count = unsigned(quote['proposal']['block_count'], 'natural block count')
        require(count > 0 and count == len(quote['quote']['blocks']), 'natural block count mismatch')
        budget.add('blocks', count)
        canonical, _ = _input(sample['guest_input'], input_plan, budget, natural=True)
        workload.validate_quote(quote, {'public_output': quote['proposal']['public_output']},
            canonical, key[1], dict(contract, corpus_id=key[0]))
        prices = workload.node_prices(quote)
        raw_unzen, total = cap.raw_unzen(quote)
        rows.append(dict(quote_key=quote['key'], sample_id=key[1], guest_input_sha256=canonical[0],
            status='quote_only', raw_unzen_per_block=raw_unzen, raw_unzen_sum=total,
            **prices, **_project_quote(quote)))
        del quote
    require(not indexed, 'missing natural quote key')
    return rows


def _rows(prepared, profile, budget):
    rows, histories, seen = [], [], set()
    for _, samples, _ in prepared:
        for sample, plans in samples:
            if sample['status'] == 'complete':
                data, _ = budget.json(plans['block_count'])
                require(isinstance(data.get('blocks'), list), 'missing counted block list')
                count = len(data['blocks'])
                require(count > 0 and (sample.get('measurement') == 'quote_actual_only'
                        or count == unsigned(data['block_count'], 'native block count')), 'native block count mismatch')
                budget.add('blocks', count)
                plans['expected_blocks'] = count
                del data
    for contract, samples, natural in prepared:
        for sample, plans in samples:
            key = sample['quote_key']['guest_input_sha256']
            require(key not in seen, 'duplicate canonical input across manifests')
            seen.add(key)
            row = dict(sample_id=sample['sample_id'], role=sample['role'], status=sample['status'],
                quote_key=sample['quote_key'], guest_input_sha256=key,
                measurement=sample.get('measurement', 'native_receipt'))
            if 'failure' in sample:
                row['failure'] = sample['failure']
            if sample['status'] == 'complete':
                row.update(_measured(sample, plans, contract, profile, budget))
            else:
                row.update(actual_proposal_sp1_pgu=None, ineligible_reason='unmeasured')
            rows.append(row)
        histories.extend(_natural(natural, contract, budget, seen))
    return rows, histories


def _conversion(rows, profile):
    indexed = {row['guest_input_sha256']: row for row in rows if row['status'] == 'complete'}
    for name in ('training_inputs', 'evaluation_inputs'):
        require(set(profile['partition'][name]) <= indexed.keys(), 'missing measured '+name)
    if profile['conversion']['kind'] == 'provided_ratio':
        value = ratio(profile['conversion']['ratio'], 'provided conversion')
    else:
        selected = [indexed[key] for key in profile['partition']['training_inputs']]
        numerator = sum(row['actual_proposal_sp1_pgu'] for row in selected)
        denominator = sum(row['raw_unzen_sum'] for row in selected)
        require(numerator > 0 and denominator > 0, 'training sums must be positive')
        value = Fraction(numerator, denominator)
    return value


def _stage_prices(row, scale, profile, selectors, used_selectors):
    blocks = row['baseline_blocks']
    key = row['guest_input_sha256'], row['canonical_quote_sha256']
    extra = selectors.get(key)
    if extra is not None:
        require(len(extra) == len(blocks), 'selector/quote block count mismatch')
        used_selectors.add(key)
    per_stage = {stage: [] for stage in profile['stages']}
    missing = {stage: [] for stage in profile['stages']}
    tariffs = profile['tariffs']
    for index, block in enumerate(blocks):
        value = scale*row['raw_unzen_per_block'][index]
        per_stage['calibrated_unzen'].append(ceil_fraction(value))
        absent = []
        for stage in profile['stages'][1:]:
            if stage == 'ecmul':
                internal = block['node'].get('semantic_counters', {})
                diagnostic = internal.get('diagnostic')
                internal = internal.get('ecmul')
                external = extra[index] if extra is not None else None
                require(diagnostic is None or external is None, 'sidecar cannot override incomplete quote semantics')
                require(internal is None or external is None or internal == external,
                        'quote and sidecar ECMUL counters disagree')
                counters = internal if internal is not None else external
                if counters is None:
                    absent.append('ecmul_semantic_counters'+(':'+diagnostic if diagnostic else ''))
                else:
                    value += tariffs['ecmul_scalar_work_pgu']*counters['successful_scalar_work_units']
            elif stage == 'access_list_linear':
                item = block['node']['pricing'].get('access_list_items')
                keys = block['node']['semantic_counters'].get('access_list_raw_storage_keys')
                for name, count, tariff in (
                        ('access_list_items', item, 'access_list_item_pgu'),
                        ('access_list_raw_storage_keys', keys, 'access_list_key_pgu')):
                    if count is None:
                        absent.append(name)
                    else:
                        value += unsigned(count, name)*tariffs[tariff]
            else:
                names = ACCESS_FIELDS[1:] if stage == 'access_list_pairs' else CONTAINER_FIELDS
                for field, tariff in zip(names, TARIFFS[stage]):
                    pricing = block['node']['pricing']
                    if field not in pricing:
                        absent.append(field)
                    else:
                        value += unsigned(pricing[field], field)*tariffs[tariff]
            missing[stage].extend(dict(block_index=index, selector=field) for field in absent)
            per_stage[stage].append(None if absent else ceil_fraction(value))
    return {stage: dict(status='not_available' if missing[stage] else 'complete',
        per_block_pgu_equivalent=per_stage[stage],
        sum_of_block_pgu_equivalent=None if missing[stage] else sum(per_stage[stage]),
        missing=missing[stage]) for stage in profile['stages']}


def _maximum(rows, scores, limit):
    admitted = [row for row, score in zip(rows, scores) if score <= limit]
    return dict(accepted_count=len(admitted),
        max_actual_pgu=max((row['actual_proposal_sp1_pgu'] for row in admitted), default=None),
        admitted_inputs=[row['guest_input_sha256'] for row in admitted])


def _exact_stage_score(row, stage, index, scale):
    # Every reserve coefficient is an integer PGU value. Therefore the difference
    # between the two ceiled quotes is exactly the additive integer reserve.
    reserve = row['stages'][stage]['per_block_pgu_equivalent'][index] \
        - row['stages']['calibrated_unzen']['per_block_pgu_equivalent'][index]
    return scale*row['raw_unzen_per_block'][index]+reserve


def _inherited_budget(scale, profile):
    exact = scale*profile['limits']['raw_unzen']
    return dict(raw_unzen_limit=profile['limits']['raw_unzen'],
        floor_threshold_pgu_equivalent=exact.numerator//exact.denominator,
        exact_threshold_pgu_equivalent=fraction_record(exact),
        scope=('The same add-only counterfactual score under the converted old raw budget. '
               'The exact rational comparison preserves raw Unzen acceptance at the unpatched '
               'stage. Comparing ceiled quotes to the floor threshold is additionally conservative '
               'at fractional boundaries; it is reported separately. The conversion remains '
               'training-sum shape normalization, not marginal or safety calibration.'))


def _capacity(rows, stage, profile, scale):
    eligible = [row for row in rows if row['status'] == 'complete' and row['ineligible_reason'] is None]
    matched = [row for row in eligible if row['stages'][stage]['status'] == 'complete']
    available = {row['guest_input_sha256'] for row in matched}
    limit = profile['limits']['counterfactual_pgu']
    inherited = _inherited_budget(scale, profile)
    inherited.update(integer_ceiling_frontier=_maximum(matched,
        [row['stages'][stage]['sum_of_block_pgu_equivalent'] for row in matched],
        inherited['floor_threshold_pgu_equivalent']),
        exact_budget_frontier=_maximum(matched,
            [_exact_stage_score(row, stage, 0, scale) for row in matched], scale*profile['limits']['raw_unzen']))
    return dict(eligible=dict(count=len(eligible), available=len(matched),
        unavailable_inputs=[row['guest_input_sha256'] for row in eligible
                            if row['guest_input_sha256'] not in available]),
        matched=dict(unzen=_maximum(matched, [row['raw_unzen_sum'] for row in matched], profile['limits']['raw_unzen']),
            patched=_maximum(matched, [row['stages'][stage]['sum_of_block_pgu_equivalent'] for row in matched], limit),
            new_node=_maximum(matched, [row['node_literal_sum_pgu'] for row in matched], limit)),
        inherited_unzen_budget_patched=inherited)


def _shape(rows, stage, profile):
    indexed = {row['guest_input_sha256']: row for row in rows if row['status'] == 'complete'}
    partition = profile['partition']
    result = dict(evaluation_status=partition['evaluation_status'],
        evaluation_status_authority='Caller declaration; tool does not certify that observations were unopened.',
        training_count=len(partition['training_inputs']), evaluation_count=len(partition['evaluation_inputs']))
    required = partition['training_inputs'] + partition['evaluation_inputs']
    unavailable = [key for key in required if indexed[key]['stages'][stage]['status'] != 'complete']
    if not partition['training_inputs'] or not partition['evaluation_inputs'] or unavailable:
        return dict(result, status='not_available', unavailable_inputs=unavailable)
    selected = lambda name: [indexed[key] for key in partition[name]]
    training, evaluation = selected('training_inputs'), selected('evaluation_inputs')
    scores = dict(unzen=lambda row: row['raw_unzen_sum'],
        patched=lambda row: row['stages'][stage]['sum_of_block_pgu_equivalent'],
        new_node=lambda row: row['node_literal_sum_pgu'],
        new_full=lambda row: row['proposal_estimate_pgu'], constant=lambda row: 1)
    metrics = {}
    actual_sum = sum(row['actual_proposal_sp1_pgu'] for row in evaluation)
    require(actual_sum > 0, 'evaluation actual sum must be positive')
    for name, score in scores.items():
        training_sum = sum(score(row) for row in training)
        if training_sum == 0:
            metrics[name] = dict(status='not_available', reason='zero training score sum')
            continue
        multiplier = Fraction(sum(row['actual_proposal_sp1_pgu'] for row in training), training_sum)
        error = sum(abs(multiplier*score(row)-row['actual_proposal_sp1_pgu']) for row in evaluation)
        values = [score(row) for row in evaluation]
        metrics[name] = dict(status='complete', training_only_scale=fraction_record(multiplier),
            wape=fraction_record(error/actual_sum),
            raw_spearman=spearman(values, [row['actual_proposal_sp1_pgu'] for row in evaluation]))
        if name in ('new_node', 'new_full'):
            metrics[name].update(native_sum_over_actual=fraction_record(Fraction(sum(values), actual_sum)),
                native_underquote_count=sum(row['actual_proposal_sp1_pgu'] > score(row) for row in evaluation),
                maximum_positive_native_gap_pgu=max(
                    max(0, row['actual_proposal_sp1_pgu']-score(row)) for row in evaluation))
    return dict(result, status='complete', metrics=metrics)


def spearman(predictions, actuals):
    """Average-tie ranks with exact squared correlation and six-place half-even display."""
    require(len(predictions) == len(actuals), 'rank vector length mismatch')
    size = len(predictions)
    if size < 2:
        return dict(status='not_available', reason='fewer than two observations')

    def ranks(values):
        ordered = sorted(range(size), key=values.__getitem__)
        result, start = [0]*size, 0
        while start < size:
            end = start+1
            while end < size and values[ordered[end]] == values[ordered[start]]:
                end += 1
            # Twice the average one-based rank is an exact integer.
            for position in range(start, end):
                result[ordered[position]] = start+end+1
            start = end
        return [value-(size+1) for value in result]

    left, right = ranks(predictions), ranks(actuals)
    covariance = sum(a*b for a, b in zip(left, right))
    product = sum(a*a for a in left)*sum(b*b for b in right)
    if product == 0:
        return dict(status='not_available', reason='zero rank variance')
    with localcontext() as context:
        context.prec = 64
        value = (Decimal(covariance)/Decimal(product).sqrt()).quantize(Decimal('0.000001'),
                                                                    rounding=ROUND_HALF_EVEN)
    return dict(status='complete', value=format(abs(value) if value.is_zero() else value, '.6f'),
                squared=fraction_record(Fraction(covariance*covariance, product)),
                sign=(covariance > 0)-(covariance < 0))


def _historical(rows, stage, profile, scale):
    available = [row for row in rows if row['stages'][stage]['status'] == 'complete']
    available_inputs = {row['guest_input_sha256'] for row in available}
    raw_limit = profile['limits']['raw_unzen']
    pgu_limit = profile['limits']['counterfactual_pgu']
    inherited = _inherited_budget(scale, profile)
    inherited_floor = inherited_exact = 0
    count = raw_passing = patched_passing = node_passing = 0
    for row in available:
        for index, (raw, patched, node) in enumerate(zip(row['raw_unzen_per_block'],
                row['stages'][stage]['per_block_pgu_equivalent'], row['node_per_block_pgu'])):
            count += 1
            raw_passing += raw <= raw_limit
            patched_passing += patched <= pgu_limit
            node_passing += node <= pgu_limit
            inherited_floor += patched <= inherited['floor_threshold_pgu_equivalent']
            inherited_exact += _exact_stage_score(row, stage, index, scale) <= scale*raw_limit
    inherited.update(integer_ceiling_passing_blocks=inherited_floor, exact_budget_passing_blocks=inherited_exact)
    return dict(proposals=len(rows), available_proposals=len(available),
        unavailable_inputs=[row['guest_input_sha256'] for row in rows
                            if row['guest_input_sha256'] not in available_inputs],
        matched_blocks=count, raw_unzen_passing=raw_passing,
        patched_passing=patched_passing, new_node_passing=node_passing,
        inherited_unzen_budget_patched=inherited,
        scope='Existing quote-only block packing. No actual PGU, transaction repacking or rejection rate.')


def build_report(manifest_paths, profile_path, *, selectors_path=None, resource_limits=None):
    """Validate artifacts, preserve unavailable rows, then compare identical subsets."""
    require(0 < len(manifest_paths) <= 128, 'need 1..=128 manifests')
    budget = ResourceBudget(resource_limits)
    profile, profile_source, authorities = read_profile(profile_path, budget)
    prepared, manifests = _prepare(manifest_paths, profile, budget)
    selector_plan = budget.plan(selectors_path) if selectors_path is not None else None
    # Every source graph is size-preflighted before coefficient/sample bytes are read.
    for plan, purpose in authorities:
        _, source = budget.read(plan)
        profile_source['sources'].append(dict(source, purpose=purpose))
    selectors, selector_source = read_sidecar(selector_plan, budget)
    rows, histories = _rows(prepared, profile, budget)
    scale = _conversion(rows, profile)
    used_selectors = set()
    for row in rows + histories:
        if row['status'] in ('complete', 'quote_only'):
            row['stages'] = _stage_prices(row, scale, profile, selectors, used_selectors)
    require(used_selectors == selectors.keys(), 'unmatched or stale selector sidecar row')
    stages = {}
    for index, stage in enumerate(profile['stages']):
        selectors_used = ['raw_unzen_zkgas']
        prices_used = ['whole_proposal_training_conversion']
        for patch in profile['stages'][1:index+1]:
            if patch == 'ecmul':
                selectors_used += ['ecmul.successful_scalar_work_units']
            elif patch == 'access_list_linear':
                selectors_used += ['access_list_items', 'access_list_raw_storage_keys']
            else:
                selectors_used += list(ACCESS_FIELDS[1:] if patch == 'access_list_pairs' else CONTAINER_FIELDS)
            prices_used += list(TARIFFS[patch])
        stages[stage] = dict(capacity=_capacity(rows, stage, profile, scale), shape=_shape(rows, stage, profile),
            historical=_historical(histories, stage, profile, scale),
            complexity=dict(selectors=selectors_used, price_parameters=prices_used,
                scope='Explicit feature/parameter inventory; not a CPU or total maintenance-cost measurement.'))
    projected = []
    for row in rows + histories:
        projected.append({key: row[key] for key in (
            'sample_id', 'guest_input_sha256', 'quote_key', 'status', 'measurement', 'measurement_scope', 'ineligible_reason', 'failure',
            'block_count', 'block_evm_gas_used', 'total_evm_gas_used', 'user_evm_gas_used_total',
            'raw_unzen_per_block', 'raw_unzen_sum', 'node_literal_sum_pgu',
            'actual_proposal_sp1_pgu', 'proposal_estimate_pgu', 'stages') if key in row})
        if 'canonical_quote_sha256' in row:
            projected[-1]['canonical_quote_sha256'] = row['canonical_quote_sha256']
    return dict(schema_version=1, baseline_id=profile['baseline_id'],
        profile=profile, sources=dict(profile=profile_source, manifests=manifests, selectors=selector_source),
        conversion=dict(kind=profile['conversion']['kind'], ratio=fraction_record(scale), scope=CONVERSION_SCOPE),
        scope=('Finite offline counterfactual with add-only reserves. Raw Unzen remains unzen_zkgas; '
               'patched scores are PGU-equivalent diagnostics, not production tariffs. Capacity maxima '
               'use complete single-block proposals within 30M charged gas; multi-block actuals stay whole. '
               'Missing semantic observations remain unavailable, never zero-filled or silently excluded. '
               'Coefficient provenance bindings are retained, not independently certified by this tool.'),
        stages=stages, rows=projected, resources=budget.receipt())


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, action='append', required=True)
    parser.add_argument('--profile', type=Path, required=True)
    parser.add_argument('--selectors', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--max-source-bytes', type=int, default=DEFAULT_RESOURCE_LIMITS['source_bytes'])
    parser.add_argument('--max-rows', type=int, default=DEFAULT_RESOURCE_LIMITS['rows'])
    parser.add_argument('--max-blocks', type=int, default=DEFAULT_RESOURCE_LIMITS['blocks'])
    args = parser.parse_args(argv)
    exclusive_json(args.output, build_report(args.manifest, args.profile, selectors_path=args.selectors,
        resource_limits=dict(source_bytes=args.max_source_bytes, rows=args.max_rows, blocks=args.max_blocks)))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
