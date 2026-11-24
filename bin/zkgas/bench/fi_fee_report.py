#!/usr/bin/env python3
"""Offline FI fee sensitivity arithmetic. No execution or market observations."""
import argparse
from fractions import Fraction
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile

UINT64 = (1 << 64) - 1
UINT128 = (1 << 128) - 1
UINT256 = (1 << 256) - 1
MAX_CONFIG_BYTES = 65536
MODELS = ('linear', 'doubling_interpolated')
FIELDS = {'schema_version', 'evidence_kind', 'base_fee_wei', 'doubling_thresholds',
          'initial_pending', 'enqueue_count', 'pending_samples',
          'sp1_usd_per_billion_pgu', 'eth_usd', 'reserve_multiplier',
          'fi_extra_pgu_scenarios'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def exact_keys(value, keys, name):
    require(type(value) is dict and value.keys() == keys, f'{name}: fields must be {sorted(keys)}')


def integer(value, maximum, name, minimum=0):
    require(type(value) is int and minimum <= value <= maximum,
            f'{name}: expected integer in [{minimum}, {maximum}]')
    return value


def integer_string(value, maximum, name, minimum=0):
    require(type(value) is str and len(value) <= len(str(maximum))
            and re.fullmatch(r'0|[1-9][0-9]*', value) is not None,
            f'{name}: expected canonical decimal integer string')
    return integer(int(value), maximum, name, minimum)


def decimal_ratio(value, name, minimum=None):
    require(type(value) is str and len(value) <= 39
            and re.fullmatch(r'(0|[1-9][0-9]*)(\.[0-9]*[1-9])?', value) is not None,
            f'{name}: expected canonical decimal string (no exponent or trailing fractional zero)')
    require(len(value.replace('.', '')) <= 38 and
            ('.' not in value or len(value.split('.')[1]) <= 18),
            f'{name}: at most 38 digits and 18 fractional digits')
    result = Fraction(value)
    require(result > 0 and (minimum is None or result >= minimum), f'{name}: invalid rate')
    return result


def unique_integers(value, limit, name, minimum=0):
    require(type(value) is list and len(value) <= limit, f'{name}: expected array with at most {limit} entries')
    for item in value:
        integer(item, UINT64, name, minimum)
    require(len(set(value)) == len(value), f'{name}: duplicate value')


def validate(config):
    exact_keys(config, FIELDS, 'config')
    require(type(config['schema_version']) is int and config['schema_version'] == 1,
            'schema_version: expected 1')
    require(config['evidence_kind'] == 'sensitivity', 'evidence_kind: expected sensitivity')
    integer_string(config['base_fee_wei'], UINT256, 'base_fee_wei', 1)
    unique_integers(config['doubling_thresholds'], 16, 'doubling_thresholds', 1)
    require(config['doubling_thresholds'], 'doubling_thresholds: must not be empty')
    initial = integer(config['initial_pending'], UINT64, 'initial_pending')
    count = integer(config['enqueue_count'], 10000, 'enqueue_count')
    require(initial + count <= UINT64, 'enqueue span: final pending exceeds uint64')
    unique_integers(config['pending_samples'], 32, 'pending_samples')
    for key in ('sp1_usd_per_billion_pgu', 'eth_usd', 'reserve_multiplier'):
        decimal_ratio(config[key], key, 1 if key == 'reserve_multiplier' else None)
    scenarios = config['fi_extra_pgu_scenarios']
    require(type(scenarios) is list and 1 <= len(scenarios) <= 32,
            'fi_extra_pgu_scenarios: expected 1..32 explicit scenarios')
    names = set()
    for row in scenarios:
        exact_keys(row, {'name', 'pgu'}, 'scenario')
        name = row['name']
        require(type(name) is str and 0 < len(name) <= 64 and name == name.strip()
                and name.isprintable() and name not in names, 'scenario: invalid or duplicate name')
        names.add(name)
        integer_string(row['pgu'], UINT128, 'scenario.pgu')
    return config


def no_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f'duplicate JSON key: {key}')
        result[key] = value
    return result


def reject_constant(value):
    raise ValueError(f'nonfinite JSON number: {value}')


def read_config(path):
    path = Path(path)
    require(stat.S_ISREG(path.lstat().st_mode), 'config must be a regular file')
    with path.open('rb') as source:
        raw = source.read(MAX_CONFIG_BYTES + 1)
    require(len(raw) <= MAX_CONFIG_BYTES, 'config exceeds 64KiB')
    try:
        config = json.loads(raw, object_pairs_hook=no_duplicate_keys, parse_constant=reject_constant)
    except RecursionError as error:
        raise ValueError('config JSON nesting exceeds parser bound') from error
    return validate(config), hashlib.sha256(raw).hexdigest()


def fee_wei(model, base, threshold, pending):
    """Full-precision floor arithmetic, with only the final fee bounded to uint256."""
    integer(base, UINT256, 'base', 1)
    integer(threshold, UINT64, 'threshold', 1)
    integer(pending, UINT64, 'pending')
    require(model in MODELS, 'unknown fee model')
    if model == 'linear':
        result = base * (threshold + pending) // threshold
    else:
        quotient, remainder = divmod(pending, threshold)
        # The interpolation factor is >=1. Reject before shifting by untrusted q.
        if quotient > 256 - base.bit_length():
            raise OverflowError('fee exceeds uint256')
        result = (base << quotient) * (threshold + remainder) // threshold
    if result > UINT256:
        raise OverflowError('fee exceeds uint256')
    return result


def rational(value):
    return {'numerator': str(value.numerator), 'denominator': str(value.denominator)}


def eth(wei):
    whole, fraction = divmod(wei, 10**18)
    return str(whole) + ('.' + f'{fraction:018d}'.rstrip('0') if fraction else '')


def cost_scenarios(config):
    price = decimal_ratio(config['sp1_usd_per_billion_pgu'], 'price')
    exchange = decimal_ratio(config['eth_usd'], 'exchange')
    reserve = decimal_ratio(config['reserve_multiplier'], 'reserve', 1)
    result = []
    for scenario in config['fi_extra_pgu_scenarios']:
        cost = int(scenario['pgu']) * price / 10**9
        adjusted = cost * reserve
        required = adjusted / exchange * 10**18
        required_wei = -(-required.numerator // required.denominator)
        result.append(dict(scenario, evidence_kind='hypothetical_sensitivity',
                           incremental_sp1_usd=rational(cost),
                           reserve_adjusted_sp1_usd=rational(adjusted),
                           required_fee_wei=str(required_wei),
                           required_fee_eth=eth(required_wei)))
    return result


def coverage(revenue, costs, count=1):
    return {row['name']: revenue >= int(row['required_fee_wei']) * count for row in costs}


def build_report(path):
    config, digest = read_config(path)
    costs = cost_scenarios(config)
    samples, batches = [], []
    base, initial, count = int(config['base_fee_wei']), config['initial_pending'], config['enqueue_count']
    for threshold in config['doubling_thresholds']:
        for model in MODELS:
            for pending in config['pending_samples']:
                row = {'model': model, 'threshold': threshold, 'pending': pending,
                       'status': 'fee_overflow', 'fee_wei': None, 'fee_eth': None,
                       'covers_reserve_adjusted_sp1_cost': None}
                try:
                    fee = fee_wei(model, base, threshold, pending)
                    row.update(status='available', fee_wei=str(fee), fee_eth=eth(fee),
                               covers_reserve_adjusted_sp1_cost=coverage(fee, costs))
                except OverflowError:
                    pass
                samples.append(row)
            total, priced = 0, 0
            for offset in range(count):
                try:
                    fee = fee_wei(model, base, threshold, initial + offset)
                except OverflowError:
                    break
                total += fee
                priced += 1
            complete = priced == count
            batches.append({'model': model, 'threshold': threshold,
                            'initial_pending': initial, 'final_pending': initial + count,
                            'requested_count': count, 'priced_count': priced,
                            'status': 'complete' if complete else 'fee_overflow',
                            'first_unavailable_pending': None if complete else initial + priced,
                            'total_fee_wei': str(total) if complete else None,
                            'total_fee_eth': eth(total) if complete else None,
                            'priced_prefix_fee_wei': str(total), 'priced_prefix_fee_eth': eth(total),
                            'required_batch_fee_wei': {row['name']: str(int(row['required_fee_wei']) * count)
                                                       for row in costs},
                            'covers_reserve_adjusted_sp1_cost': coverage(total, costs, count) if complete else None})
    partial = any(row['status'] == 'fee_overflow' for row in samples + batches)
    return {'schema_version': 1, 'evidence_kind': 'sensitivity', 'sensitivity_only': True,
            'status': 'partial' if partial else 'complete',
            'config': config, 'config_sha256': digest,
            'calculator_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'assumptions': {
                'pending': 'tail - head, including consumed but unlanded entries',
                'batch': 'sequential enqueue at initial_pending+i; no landing or head advancement',
                'linear_fee_wei': 'floor(F0*(T+p)/T)',
                'doubling_interpolated_fee_wei': 'q,r=divmod(p,T); floor(F0*2^q*(T+r)/T)',
                'arithmetic': 'full precision intermediates; each fee must fit uint256; exact unbounded revenue sums',
                'required_fee_wei': 'ceil(FI_extra_PGU/1e9*SP1_USD_per_billion_PGU*reserve/ETH_USD*1e18)',
                'batch_cost': 'same hypothetical incremental PGU for every entry; sum of per-entry ceiling fees',
                'coverage': 'protocol revenue versus assumed reserve-adjusted incremental SP1 cost only',
                'limits': 'not measured Etna FI costs, full cost coverage, proof feasibility, or capacity acceptance',
                'prices': 'explicit sensitivity inputs; existing zkGas prices are unchanged'},
            'excluded_costs': ['unmeasured second proof', 'landing', 'DA', 'validator and operator operations',
                               'sender L1 submission gas (not protocol revenue)'],
            'cost_scenarios': costs, 'samples': samples, 'batches': batches}


def publish(path, report):
    """Write fully before atomic, no-replacement publication in the destination directory."""
    path = Path(path)
    encoded = (json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + '\n').encode('utf-8')
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.fi-fee-', delete=False) as target:
            temporary = Path(target.name)
            target.write(encoded)
            target.flush()
            os.fsync(target.fileno())
        os.link(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        report = build_report(args.config)
        publish(args.output, report)
    except (OSError, ValueError) as error:
        print(f'fi-fee-report: {error}', file=sys.stderr)
        return 1
    return 2 if report['status'] == 'partial' else 0


if __name__ == '__main__':
    sys.exit(main())
