#!/usr/bin/env python3
"""Read-only receipt-gas and absolute-shortfall report for validated workload outputs."""
import argparse
from pathlib import Path

from artifacts import exclusive_json, read_json
import workload

GAS_BUDGET = 30_000_000
PRIORITY_THRESHOLD = 2_000_000_000


def validate_policy(gas_budget, threshold, node_limit):
    for name, value, minimum in [('gas budget', gas_budget, 1), ('threshold', threshold, 0)]:
        workload.require(type(value) is int and value >= minimum, f'invalid {name}')
    workload.require(node_limit is None or type(node_limit) is int and node_limit >= 0,
                     'invalid node limit')


def classify_measurement(metrics, native, *, gas_budget=GAS_BUDGET,
                         threshold=PRIORITY_THRESHOLD, node_limit=None):
    validate_policy(gas_budget, threshold, node_limit)
    blocks = native['blocks']; receipts = native['receipts_by_block']
    workload.require(isinstance(blocks, list) and blocks and
                     len(blocks) == native['block_count'] == len(receipts), 'native block count mismatch')
    gas, user_gas = [], 0
    for index, (block, items) in enumerate(zip(blocks, receipts)):
        workload.require(type(block['block_index']) is int and block['block_index'] == index,
                         'native block index mismatch')
        workload.require(isinstance(items, list) and items, 'missing charged-gas receipts')
        previous, users = 0, 0
        for tx_index, item in enumerate(items):
            workload.require(type(item['transaction_index']) is int and item['transaction_index'] == tx_index,
                             'native transaction index mismatch')
            used = workload.number(item['gas_used']); cumulative = workload.number(item['cumulative_gas_used'])
            workload.require(cumulative-previous == used, 'receipt gas delta mismatch')
            previous = cumulative
            if tx_index: users += used
        workload.require(previous == workload.number(block['block_gas_used']) and
                         users == workload.number(block['user_transaction_gas_used']), 'native gas total mismatch')
        gas.append(previous); user_gas += users
    workload.require(user_gas == workload.number(native['user_transaction_gas_used_total']), 'native user gas mismatch')
    actual = workload.number(metrics['actual_sp1_pgu']); node = workload.number(metrics['node_known_pgu'])
    full = workload.number(metrics['prove_total_pgu'])
    per_block = metrics['node_per_block_pgu']
    workload.require(isinstance(per_block, list) and len(per_block) == len(blocks) and
                     sum(workload.number(value) for value in per_block) == node,
                     'node block partition mismatch')
    eligible = len(blocks) == 1 and gas[0] <= gas_budget
    shortfall, full_shortfall = max(0, actual-node), max(0, actual-full)
    return dict(actual_proposal_sp1_pgu=actual, node_literal_sum_pgu=node, proposal_estimate_pgu=full,
        node_per_block_pgu=metrics['node_per_block_pgu'], node_aggregate_ceil_pgu=metrics['node_aggregate_ceil_pgu'],
        actual_over_node=metrics['actual_over_node'], actual_over_full=metrics['actual_over_full'],
        block_count=len(blocks), block_evm_gas_used=gas, total_evm_gas_used=sum(gas), user_evm_gas_used_total=user_gas,
        node_shortfall_pgu=shortfall, proposal_shortfall_pgu=full_shortfall,
        priority=('high' if shortfall > threshold else 'low') if eligible else None,
        ineligible_reason=None if eligible else 'multi_block_proposal' if len(blocks) != 1 else 'over_gas_budget',
        near_full=eligible and gas[0]*100 >= gas_budget*99,
        admitted_at_node_limit=None if node_limit is None or len(blocks) != 1 else node <= node_limit)


def build_report(root, *, gas_budget=GAS_BUDGET, threshold=PRIORITY_THRESHOLD, node_limit=None):
    validate_policy(gas_budget, threshold, node_limit)
    # summary validates checksums, inputs, quote/actual joins and old metric shapes;
    # it recomputes the canonical literal node sum without changing original files.
    source = workload.summary(root); rows = []
    for entry in source['attempts']:
        row = {key: entry[key] for key in ['sample_id', 'status']}
        if 'attempt_id' in entry: row['attempt_id'] = entry['attempt_id']
        if 'failure' in entry: row['failure'] = entry['failure']
        if entry['status'] == 'complete':
            native = read_json(root/entry['sample_id']/entry['attempt_id']/'native.json')
            row.update(classify_measurement(entry['metrics'], native, gas_budget=gas_budget,
                                           threshold=threshold, node_limit=node_limit))
            row['guest_input_sha256'] = native['guest_input_sha256']
        else:
            row.update(priority=None, ineligible_reason='unmeasured', actual_proposal_sp1_pgu=None,
                       node_shortfall_pgu=None, proposal_shortfall_pgu=None)
        rows.append(row)
    eligible = [r for r in rows if r['priority'] is not None]
    admitted = [r for r in eligible if r['admitted_at_node_limit'] is True]
    return dict(schema_version=1, gas_budget_evm=gas_budget, high_priority_shortfall_pgu=threshold,
        node_limit_pgu=node_limit, counts=dict(attempts=len(rows), complete=sum(r['status'] == 'complete' for r in rows),
            eligible=len(eligible), near_full=sum(r['near_full'] for r in eligible),
            high_priority=sum(r['priority'] == 'high' for r in eligible), low_priority=sum(r['priority'] == 'low' for r in eligible)),
        max_observed_node_shortfall_pgu=max((r['node_shortfall_pgu'] for r in eligible), default=None),
        max_observed_proposal_shortfall_pgu=max((r['proposal_shortfall_pgu'] for r in eligible), default=None),
        max_admitted_node_shortfall_pgu=None if node_limit is None else max((r['node_shortfall_pgu'] for r in admitted), default=None),
        source_contract=read_json(root/'contract.json'), rows=rows,
        scope='Finite same-program whole-proposal SP1 execute-only observations; single-block gas-budget priority. Sampled maxima are not upper bounds. Node-limit admissibility is separate; no actual PGU allocated to blocks.')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--gas-budget', type=int, default=GAS_BUDGET)
    parser.add_argument('--high-priority-shortfall', type=int, default=PRIORITY_THRESHOLD)
    parser.add_argument('--node-limit', type=int)
    args = parser.parse_args(argv)
    exclusive_json(args.output, build_report(args.output_root, gas_budget=args.gas_budget,
                                           threshold=args.high_priority_shortfall, node_limit=args.node_limit))
    return 0


if __name__ == '__main__': raise SystemExit(main())
