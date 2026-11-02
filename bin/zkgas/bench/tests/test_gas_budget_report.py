"""Gas receipts and absolute-priority behavior, independent of benchmark producers."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

BENCH = Path(__file__).resolve().parents[1]


def load():
    path = BENCH/'gas_budget_report.py'
    assert path.is_file(), 'gas-budget report tool is missing'
    sys.path.insert(0, str(BENCH))
    spec = importlib.util.spec_from_file_location('budget_report_test_subject', path)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def native(gas=30_000_000, blocks=1):
    return {'block_count': blocks,
        'blocks': [{'block_index': i, 'block_gas_used': gas, 'user_transaction_gas_used': gas-1000} for i in range(blocks)],
        'user_transaction_gas_used_total': blocks*(gas-1000),
        'receipts_by_block': [[{'transaction_index': 0, 'gas_used': 1000, 'cumulative_gas_used': 1000, 'success': True},
            {'transaction_index': 1, 'gas_used': gas-1000, 'cumulative_gas_used': gas, 'success': True}] for _ in range(blocks)]}


def metrics(actual=2_000_000_100, node=100, full=3_000_000_000):
    return {'actual_sp1_pgu': actual, 'node_known_pgu': node, 'prove_total_pgu': full,
        'node_aggregate_ceil_pgu': node, 'node_per_block_pgu': [node],
        'actual_over_node': {'numerator': actual, 'denominator': node},
        'actual_over_full': {'numerator': actual, 'denominator': full}}


class GasBudgetReportTests(unittest.TestCase):
    def test_empty_root_has_no_maximum_and_invalid_policy_still_rejects(self):
        m = load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); (root/'contract.json').write_text('{}')
            report = m.build_report(root)
            self.assertEqual(report['counts']['complete'], 0)
            self.assertIsNone(report['max_observed_node_shortfall_pgu'])
            self.assertIsNone(report['max_observed_proposal_shortfall_pgu'])
            for kwargs in [{'gas_budget':0}, {'threshold':-1}, {'node_limit':-1}]:
                with self.subTest(kwargs=kwargs), self.assertRaises(ValueError): m.build_report(root, **kwargs)

    def test_failed_attempt_stays_unmeasured_and_cli_never_overwrites(self):
        m = load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'evidence'; root.mkdir(); contract = {'identities':{}}
            (root/'contract.json').write_text(json.dumps(contract))
            folder = root/'rejected'/'attempt-1'; folder.mkdir(parents=True)
            (folder/'started.json').write_text(json.dumps({'schema_version':1,'source':{},'contract':contract}))
            status = {'schema_version':1,'sample_id':'rejected','attempt_id':'attempt-1','source':{},'contract':contract,
                'status':'stage_failed','artifacts':[],'failure':{'stage':'sp1','message':'resource guard stopped execution','code':'guard'}}
            (folder/'status.json').write_text(json.dumps(status))
            before = {p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()}
            output = Path(temporary)/'report.json'
            self.assertEqual(m.main(['--output-root',str(root),'--output',str(output)]), 0)
            report = json.loads(output.read_text()); self.assertEqual(report['counts']['eligible'], 0)
            row = report['rows'][0]; self.assertIsNone(row['priority']); self.assertIsNone(row['actual_proposal_sp1_pgu'])
            self.assertEqual(row['failure'], status['failure'])
            with self.assertRaises(FileExistsError): m.main(['--output-root',str(root),'--output',str(output)])
            self.assertEqual(before,{p.relative_to(root):p.read_bytes() for p in root.rglob('*') if p.is_file()})

    def test_strict_two_billion_boundary_uses_missing_node_work(self):
        m = load(); row = m.classify_measurement(metrics(), native())
        self.assertEqual(row['node_shortfall_pgu'], 2_000_000_000)
        self.assertEqual(row['priority'], 'low')
        self.assertEqual(row['proposal_shortfall_pgu'], 0)
        self.assertEqual(m.classify_measurement(metrics(actual=2_000_000_101), native())['priority'], 'high')
        self.assertEqual(m.classify_measurement(metrics(actual=2_100_000_000, node=2_000_000_000), native())['priority'], 'low')

    def test_underfilled_high_case_counts_but_over_budget_and_multiblock_do_not(self):
        m = load(); high = metrics(actual=2_000_000_101)
        self.assertEqual(m.classify_measurement(high, native(gas=1_000_000))['priority'], 'high')
        over = m.classify_measurement(high, native(gas=30_000_001))
        self.assertEqual((over['priority'], over['ineligible_reason']), (None, 'over_gas_budget'))
        multi_metrics = dict(high, node_known_pgu=200, node_per_block_pgu=[100, 100])
        multi = m.classify_measurement(multi_metrics, native(gas=20_000_000, blocks=2))
        self.assertEqual((multi['priority'], multi['ineligible_reason']), (None, 'multi_block_proposal'))
        self.assertEqual(multi['actual_proposal_sp1_pgu'], high['actual_sp1_pgu'])

    def test_multiblock_node_limit_is_not_applied_to_proposal_sum(self):
        m = load(); per_block = dict(metrics(node=180), node_per_block_pgu=[90,90])
        row = m.classify_measurement(per_block, native(gas=1_000_000, blocks=2), node_limit=100)
        self.assertIsNone(row['admitted_at_node_limit'])
        self.assertIsNone(row['priority'])

    def test_node_block_partition_must_match_native_and_literal_sum(self):
        m = load()
        for partition in [[], [99], [50,50]]:
            with self.subTest(partition=partition), self.assertRaises(ValueError):
                m.classify_measurement(dict(metrics(), node_per_block_pgu=partition), native())

    def test_receipt_accounting_and_optional_node_limit_remain_separate(self):
        m = load(); high = metrics(actual=2_000_000_101)
        row = m.classify_measurement(high, native(), node_limit=99)
        self.assertEqual(row['priority'], 'high'); self.assertFalse(row['admitted_at_node_limit'])
        self.assertTrue(row['near_full']); self.assertEqual(row['user_evm_gas_used_total'], 29_999_000)
        self.assertEqual(row['block_evm_gas_used'], [30_000_000])
        self.assertIsNone(m.classify_measurement(high, native())['admitted_at_node_limit'])
        self.assertTrue(m.classify_measurement(high, native(), node_limit=100)['admitted_at_node_limit'])
        # Failed receipts are valid workload; their charged gas still counts.
        n = native(); n['receipts_by_block'][0][1]['success'] = False
        self.assertEqual(m.classify_measurement(high, n)['priority'], 'high')

    def test_gas_join_errors_and_invalid_policy_values_reject(self):
        m = load()
        for field in ['block_total', 'receipt_delta', 'user_total', 'index', 'blocks']:
            n = native()
            if field == 'block_total': n['blocks'][0]['block_gas_used'] -= 1
            elif field == 'receipt_delta': n['receipts_by_block'][0][1]['gas_used'] -= 1
            elif field == 'user_total': n['user_transaction_gas_used_total'] -= 1
            elif field == 'index': n['receipts_by_block'][0][1]['transaction_index'] = 0
            elif field == 'blocks': n['blocks'] = []
            with self.subTest(field=field), self.assertRaises(ValueError):
                m.classify_measurement(metrics(), n)
        for key,value in [('gas_budget', 0), ('gas_budget', True), ('threshold', -1), ('threshold', True), ('node_limit', -1)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                m.classify_measurement(metrics(), native(), **{key:value})

    def test_overquotes_have_zero_shortfall_and_preserve_both_prices(self):
        m = load(); row = m.classify_measurement(metrics(actual=10, node=20, full=5), native(gas=10_000))
        self.assertEqual((row['node_shortfall_pgu'], row['proposal_shortfall_pgu']), (0, 5))
        self.assertEqual((row['node_literal_sum_pgu'], row['proposal_estimate_pgu']), (20, 5))
        self.assertEqual(row['priority'], 'low'); self.assertFalse(row['near_full'])

    @unittest.skipUnless(os.environ.get('ZKGAS_WORKLOAD_ROOT'), 'opt in retained workload evidence; no backend execution')
    def test_retained_workload_read_only_integration(self):
        m = load(); root = Path(os.environ['ZKGAS_WORKLOAD_ROOT'])
        report = m.build_report(root)
        complete = [r for r in report['rows'] if r['status'] == 'complete']
        self.assertGreater(len(complete), 0)
        self.assertEqual(report['counts']['complete'], len(complete))
        eligible = [r for r in complete if r['priority'] is not None]
        self.assertEqual(report['max_observed_node_shortfall_pgu'], max((r['node_shortfall_pgu'] for r in eligible), default=None))


if __name__ == '__main__': unittest.main()
