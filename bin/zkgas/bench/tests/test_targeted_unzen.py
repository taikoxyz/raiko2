"""Counterfactual Unzen reserves from explicit profiles and authenticated receipts."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest import mock

BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))
import test_cap_frontier_report as cap_fixtures


def load():
    spec = importlib.util.spec_from_file_location('targeted_unzen_subject', BENCH/'targeted_unzen.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class TargetedUnzenTests(unittest.TestCase):
    def setUp(self):
        self.fixture = cap_fixtures.CapFrontierTests(methodName='runTest')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.train = self.case('train', raw=100, actual=200, work=0)
        self.evaluation = self.case('evaluation', raw=200, actual=500, work=1, access=1, keys=2, container=1)
        evidence = self.root/'calibration.json'
        evidence.write_text('{"purpose":"synthetic test-only coefficient authority"}\n')
        self.profile = dict(schema_version=1, baseline_id='synthetic-targeted-unzen',
            comparison_model_id='model-v1', program=dict(elf_sha256='3'*64, sdk_version='6.3.0'),
            conversion=dict(kind='training_sum_ratio'),
            partition=dict(training_inputs=[self.train['input']['sha256']],
                evaluation_inputs=[self.evaluation['input']['sha256']], evaluation_status='opened_regression'),
            stages=['calibrated_unzen', 'ecmul', 'access_list_linear', 'access_list_pairs', 'container'],
            tariffs=dict(ecmul_scalar_work_pgu=100, access_list_item_pgu=10,
                access_list_key_pgu=5, access_list_same_low32_pair_pgu=20, access_list_other_pair_pgu=30,
                access_list_address_pair_pgu=40, storage_journal_work_pgu=20,
                account_journal_work_pgu=30, account_cache_work_pgu=40,
                storage_transition_work_pgu=50),
            sources=[dict(path='calibration.json', sha256=hashlib.sha256(evidence.read_bytes()).hexdigest(),
                          purpose='Explicit synthetic coefficients, not measured prices')],
            limits=dict(raw_unzen=150, counterfactual_pgu=520))

    def case(self, sid, *, raw, actual, work, access=0, keys=0, container=0, blocks=1):
        row = self.fixture.case(sid, raw=(raw,)*blocks, nodes=(90,)*blocks,
                                actual=actual, gas=1_000_000)
        quote_path = self.root/row['quote']['path']
        quote = json.loads(quote_path.read_text())
        for block in quote['quote']['blocks']:
            block['node']['semantic_counters'] = dict(ecmul=dict(
                successful_calls=int(work > 0), successful_native_gas=6000*int(work > 0),
                successful_scalar_work_units=work, failed_calls=0, failed_native_gas=0),
                access_list_raw_storage_keys=keys)
            block['node']['pricing'].update(access_list_items=access,
                access_list_construction_pairs=0, access_list_construction_other_pairs=0,
                access_list_address_pairs=0, storage_journal_work_units=container,
                account_journal_work_units=0, account_cache_work_units=0, storage_transition_work_units=0)
        quote_path.write_text(json.dumps(quote))
        row['quote']['sha256'] = hashlib.sha256(quote_path.read_bytes()).hexdigest()
        return row

    def write_inputs(self):
        self.fixture.path.write_text(json.dumps(self.fixture.manifest))
        path = self.root/'profile.json'
        path.write_text(json.dumps(self.profile))
        return path

    def build(self, selectors=None, **kwargs):
        return load().build_report([self.fixture.path], self.write_inputs(), selectors_path=selectors, **kwargs)

    def mutate_quote(self, row, update):
        path = self.root/row['quote']['path']
        quote = json.loads(path.read_text())
        update(quote)
        path.write_text(json.dumps(quote))
        row['quote']['sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
        return quote

    def sidecar(self, row, counters=None):
        module = load()
        quote = json.loads((self.root/row['quote']['path']).read_text())
        if counters is None:
            counters = dict(successful_calls=1, successful_native_gas=6000,
                successful_scalar_work_units=1, failed_calls=0, failed_native_gas=0)
        path = self.root/'selectors.json'
        path.write_text(json.dumps(dict(schema_version=1, scope='node_committed',
            producer=dict(sha256='a'*64, source_revision='synthetic-source'), rows=[dict(
                guest_input_sha256=row['input']['sha256'],
                canonical_quote_sha256=module.canonical_quote_sha256(quote),
                blocks=[dict(block_index=0, ecmul=counters)])])))
        return path

    def test_training_only_conversion_and_incremental_additions(self):
        report = self.build()
        self.assertEqual(report['conversion']['ratio'], dict(numerator=2, denominator=1))
        row = report['rows'][1]
        self.assertEqual(row['raw_unzen_per_block'], [200])
        self.assertEqual([row['stages'][stage]['per_block_pgu_equivalent'] for stage in self.profile['stages']],
                         [[400], [500], [520], [520], [540]])
        self.assertEqual(report['stages']['access_list_pairs']['capacity']['eligible']['count'], 2)
        self.assertEqual(report['stages']['access_list_pairs']['capacity']['matched']['unzen']['accepted_count'], 1)
        self.assertEqual(report['stages']['access_list_pairs']['capacity']['matched']['patched']['accepted_count'], 2)
        self.assertEqual(report['stages']['ecmul']['shape']['evaluation_count'], 1)
        self.assertIn('not a marginal', report['conversion']['scope'])

    def test_missing_scalar_observation_propagates_without_zero_fill(self):
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node'].pop('semantic_counters'))
        report = self.build()
        row = report['rows'][1]
        self.assertEqual(row['stages']['calibrated_unzen']['status'], 'complete')
        for stage in self.profile['stages'][1:]:
            self.assertEqual(row['stages'][stage]['status'], 'not_available')
            self.assertEqual(row['stages'][stage]['per_block_pgu_equivalent'], [None])
            self.assertIsNone(row['stages'][stage]['sum_of_block_pgu_equivalent'])
            self.assertEqual(report['stages'][stage]['capacity']['eligible']['available'], 1)
            self.assertEqual(report['stages'][stage]['capacity']['eligible']['unavailable_inputs'],
                             [self.evaluation['input']['sha256']])
            self.assertEqual(report['stages'][stage]['capacity']['matched']['new_node']['accepted_count'], 1)
            self.assertEqual(report['stages'][stage]['shape']['status'], 'not_available')

    def test_failed_call_gas_never_becomes_scalar_work(self):
        counters = dict(successful_calls=0, successful_native_gas=0,
            successful_scalar_work_units=0, failed_calls=1, failed_native_gas=200000)
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node'].update(
            semantic_counters=dict(ecmul=counters)))
        row = self.build()['rows'][1]
        self.assertEqual(row['stages']['ecmul']['per_block_pgu_equivalent'], [400])
        counters['failed_scalar_work_units'] = 100
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node'].update(
            semantic_counters=dict(ecmul=counters)))
        with self.assertRaisesRegex(ValueError, 'counter fields'):
            self.build()

    def test_hash_bound_sidecar_completes_old_quote(self):
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node'].pop('semantic_counters'))
        report = self.build(self.sidecar(self.evaluation))
        self.assertEqual(report['rows'][1]['stages']['ecmul']['per_block_pgu_equivalent'], [500])
        self.assertEqual(report['sources']['selectors']['producer']['sha256'], 'a'*64)

    def test_stale_or_conflicting_sidecar_is_rejected(self):
        path = self.sidecar(self.evaluation)
        self.mutate_quote(self.evaluation, lambda quote: quote.update(metadata={'changed': True}))
        with self.assertRaisesRegex(ValueError, 'stale selector'):
            self.build(path)
        counters = dict(successful_calls=1, successful_native_gas=6000,
            successful_scalar_work_units=2, failed_calls=0, failed_native_gas=0)
        with self.assertRaisesRegex(ValueError, 'disagree'):
            self.build(self.sidecar(self.evaluation, counters))

    def test_training_evaluation_overlap_is_rejected(self):
        self.profile['partition']['evaluation_inputs'].append(self.train['input']['sha256'])
        with self.assertRaisesRegex(ValueError, 'overlap'):
            self.build()

    def test_evaluation_actual_does_not_change_scale_or_quotes(self):
        before = self.build()
        path = self.root/self.evaluation['actual']['path']
        actual = json.loads(path.read_text())
        actual['prover_gas'] = 2000
        actual['execution_report']['gas'] = 38200
        path.write_text(json.dumps(actual))
        self.evaluation['actual']['sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
        after = self.build()
        self.assertEqual(before['conversion'], after['conversion'])
        self.assertEqual(before['rows'][1]['stages'], after['rows'][1]['stages'])
        self.assertNotEqual(before['stages']['ecmul']['shape'], after['stages']['ecmul']['shape'])

    def test_missing_prices_or_unverified_source_are_not_defaulted(self):
        original = copy.deepcopy(self.profile)
        del self.profile['tariffs']['ecmul_scalar_work_pgu']
        with self.assertRaisesRegex(ValueError, 'tariff'):
            self.build()
        self.profile = original
        self.profile['sources'][0]['sha256'] = 'b'*64
        with self.assertRaisesRegex(ValueError, 'identity mismatch'):
            self.build()

    def test_unmeasured_controls_remain_visible_without_numeric_success(self):
        self.fixture.manifest['samples'].append(dict(sample_id='screened', role='control', status='screened',
            quote_key=dict(corpus_id='original-corpus', sample_id='screened', guest_input_sha256='b'*64),
            failure=dict(message='native-only control')))
        row = self.build()['rows'][-1]
        self.assertEqual(row['status'], 'screened')
        self.assertEqual(row['failure']['message'], 'native-only control')
        self.assertIsNone(row['actual_proposal_sp1_pgu'])
        self.assertNotIn('stages', row)

    def test_multiblock_actuals_are_whole_and_excluded_from_block_capacity(self):
        multi = self.case('multi', raw=200, actual=4000, work=1, blocks=2)
        self.profile['partition']['evaluation_inputs'].append(multi['input']['sha256'])
        report = self.build()
        row = report['rows'][2]
        self.assertEqual(row['block_count'], 2)
        self.assertEqual(row['actual_proposal_sp1_pgu'], 4000)
        self.assertEqual(row['stages']['ecmul']['per_block_pgu_equivalent'], [500, 500])
        self.assertEqual(row['stages']['ecmul']['sum_of_block_pgu_equivalent'], 1000)
        self.assertEqual(report['stages']['ecmul']['capacity']['eligible']['count'], 2)
        self.assertEqual(report['stages']['ecmul']['shape']['evaluation_count'], 2)

    def test_historical_packing_does_not_consume_actual(self):
        history = self.case('history', raw=200, actual=10000, work=1)
        self.fixture.manifest['samples'].remove(history)
        self.fixture.natural([history])
        report = self.build()
        self.assertEqual(report['rows'][-1]['status'], 'quote_only')
        self.assertNotIn('actual_proposal_sp1_pgu', report['rows'][-1])
        self.assertEqual(report['stages']['ecmul']['historical']['matched_blocks'], 1)
        self.assertEqual(report['stages']['ecmul']['historical']['raw_unzen_passing'], 0)
        self.assertEqual(report['stages']['ecmul']['historical']['patched_passing'], 1)

    def test_duplicate_canonical_inputs_across_manifests_are_rejected(self):
        profile = self.write_inputs()
        with self.assertRaisesRegex(ValueError, 'duplicate canonical'):
            load().build_report([self.fixture.path, self.fixture.path], profile)

    def test_fractional_block_prices_are_rounded_before_summing(self):
        self.profile['conversion'] = dict(kind='provided_ratio', ratio=dict(numerator=1, denominator=3),
            purpose='Synthetic rounding check, not a measured conversion')
        multi = self.case('fractional', raw=1, actual=100, work=0, blocks=2)
        report = self.build()
        row = next(row for row in report['rows'] if row['guest_input_sha256'] == multi['input']['sha256'])
        self.assertEqual(row['stages']['calibrated_unzen']['per_block_pgu_equivalent'], [1, 1])
        self.assertEqual(row['stages']['calibrated_unzen']['sum_of_block_pgu_equivalent'], 2)

    def test_boolean_and_impossible_success_counters_are_rejected(self):
        for field, value in (('successful_calls', True), ('successful_native_gas', 6001),
                             ('successful_scalar_work_units', 508), ('failed_native_gas', 1)):
            original = json.loads((self.root/self.evaluation['quote']['path']).read_text())
            self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node']
                ['semantic_counters']['ecmul'].update({field: value}))
            with self.assertRaises(ValueError):
                self.build()
            self.mutate_quote(self.evaluation, lambda quote: quote.update(original))

    def test_profile_scope_and_json_ambiguity_are_rejected(self):
        self.profile['stages'] = ['calibrated_unzen', 'container']
        with self.assertRaisesRegex(ValueError, 'ordered prefix'):
            self.build()
        self.write_inputs()
        path = self.root/'profile.json'
        path.write_text('{"schema_version":1,"schema_version":2}')
        with self.assertRaisesRegex(ValueError, 'duplicate JSON'):
            load().build_report([self.fixture.path], path)

    def test_cli_writes_exclusively(self):
        profile = self.write_inputs()
        output = self.root/'result.json'
        argv = ['--manifest', str(self.fixture.path), '--profile', str(profile), '--output', str(output)]
        self.assertEqual(load().main(argv), 0)
        first = output.read_bytes()
        with self.assertRaises(FileExistsError):
            load().main(argv)
        self.assertEqual(first, output.read_bytes())

    def test_aggregate_byte_preflight_precedes_measured_input_reads(self):
        module = load()
        profile = self.write_inputs()
        limit = module.build_report([self.fixture.path], profile)['resources']['used']['source_bytes']-1
        with mock.patch.object(module, '_input', side_effect=AssertionError('input read too soon')) as reads:
            with self.assertRaisesRegex(ValueError, 'aggregate source byte'):
                module.build_report([self.fixture.path], profile, resource_limits={'source_bytes': limit})
            reads.assert_not_called()

    def test_total_rows_and_blocks_use_small_global_limits(self):
        with self.assertRaisesRegex(ValueError, 'total row'):
            self.build(resource_limits={'rows': 1})
        with self.assertRaisesRegex(ValueError, 'total block'):
            self.build(resource_limits={'blocks': 1})
        self.sidecar(self.evaluation)
        with self.assertRaisesRegex(ValueError, 'total block'):
            self.build(self.root/'selectors.json', resource_limits={'blocks': 2})

    def test_repeated_source_references_are_charged_and_revalidated(self):
        profile = self.write_inputs()
        first = load().build_report([self.fixture.path], profile)
        original = first['resources']['used']['source_bytes']
        self.profile['sources'].append(copy.deepcopy(self.profile['sources'][0]))
        second = self.build()
        self.assertEqual(second['resources']['used']['source_bytes']-original,
            (self.root/'calibration.json').stat().st_size
            + second['sources']['profile']['profile']['byte_length']
            - first['sources']['profile']['profile']['byte_length'])
        self.profile['sources'][-1]['sha256'] = 'b'*64
        with self.assertRaisesRegex(ValueError, 'identity mismatch'):
            self.build()

    def test_full_artifact_objects_are_not_retained_in_report_rows(self):
        report = self.build()
        for row in report['rows']:
            self.assertNotIn('quote', row)
            self.assertNotIn('native', row)
            self.assertNotIn('actual', row)
            self.assertNotIn('baseline_blocks', row)
        self.assertEqual(report['resources']['limits'],
                         dict(source_bytes=256*1024*1024, rows=10000, blocks=100000))

    def test_quote_cannot_expand_beyond_preflighted_native_block_count(self):
        module = load()
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'].append(
            copy.deepcopy(quote['quote']['blocks'][0])))
        profile = self.write_inputs()
        validate = module.workload.validate_quote

        def guarded(quote, *args):
            if quote['key']['guest_input_sha256'] == self.evaluation['input']['sha256']:
                self.fail('oversized block array reached full quote validation')
            return validate(quote, *args)

        with mock.patch.object(module.workload, 'validate_quote', side_effect=guarded):
            with self.assertRaisesRegex(ValueError, 'quote/native block count'):
                module.build_report([self.fixture.path], profile)

    def test_headline_full_constant_and_rank_metrics_use_training_only(self):
        middle = self.case('middle', raw=300, actual=300, work=0)
        last = self.case('last', raw=400, actual=100, work=0)
        self.profile['partition']['evaluation_inputs'] += [middle['input']['sha256'], last['input']['sha256']]
        for row, full in ((self.evaluation, 400), (middle, 300), (last, 200)):
            self.mutate_quote(row, lambda quote, full=full: quote['quote']['prove'].update(total_pgu=str(full)))
        metrics = self.build()['stages']['calibrated_unzen']['shape']['metrics']
        self.assertEqual(metrics['new_full']['training_only_scale'], dict(numerator=2, denominator=3))
        self.assertEqual(metrics['new_full']['wape'], dict(numerator=11, denominator=27))
        self.assertEqual(metrics['new_full']['raw_spearman']['value'], '1.000000')
        self.assertEqual(metrics['unzen']['raw_spearman']['value'], '-1.000000')
        self.assertEqual(metrics['constant']['wape'], dict(numerator=5, denominator=9))
        self.assertEqual(metrics['constant']['raw_spearman']['status'], 'not_available')
        self.assertEqual(metrics['new_full']['native_sum_over_actual'], dict(numerator=1, denominator=1))
        self.assertEqual(metrics['new_full']['native_underquote_count'], 1)
        self.assertNotIn('native_sum_over_actual', metrics['unzen'])

    def test_spearman_uses_average_ties_and_reports_degenerate_sets(self):
        module = load()
        result = module.spearman([1, 2, 2], [1, 2, 3])
        self.assertEqual(result['value'], '0.866025')
        self.assertEqual(result['squared'], dict(numerator=3, denominator=4))
        self.assertEqual(module.spearman([1], [1])['status'], 'not_available')
        self.assertEqual(module.spearman([1, 1], [1, 2])['status'], 'not_available')

    def test_incomplete_semantics_are_explicitly_unavailable_and_cannot_be_overridden(self):
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node'].update(
            semantic_counters=dict(ecmul=None, diagnostic='arithmetic_overflow')))
        row = self.build()['rows'][1]
        self.assertEqual(row['stages']['ecmul']['status'], 'not_available')
        self.assertEqual(row['stages']['ecmul']['missing'],
                         [dict(block_index=0, selector='ecmul_semantic_counters:arithmetic_overflow')])
        with self.assertRaisesRegex(ValueError, 'override incomplete'):
            self.build(self.sidecar(self.evaluation))
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node']
            ['semantic_counters'].update(ecmul=dict(successful_calls=0, successful_native_gas=0,
                successful_scalar_work_units=0, failed_calls=0, failed_native_gas=0)))
        with self.assertRaisesRegex(ValueError, 'partial semantic'):
            self.build()

    def test_semantic_wire_domain_is_u64_and_unknown_diagnostics_are_rejected(self):
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node']
            ['semantic_counters']['ecmul'].update(failed_calls=2**64))
        with self.assertRaisesRegex(ValueError, 'u64'):
            self.build()
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node'].update(
            semantic_counters=dict(ecmul=None, diagnostic='arbitrary-long-message')))
        with self.assertRaisesRegex(ValueError, 'unknown semantic diagnostic'):
            self.build()

    def test_inherited_budget_preserves_raw_policy_and_exposes_integer_rounding(self):
        self.profile['conversion'] = dict(kind='provided_ratio', ratio=dict(numerator=1, denominator=3),
            purpose='Synthetic fractional budget boundary')
        self.profile['limits']['raw_unzen'] = 1
        boundary = self.case('budget-boundary', raw=1, actual=100, work=0)
        self.case('budget-above', raw=2, actual=1000, work=0)
        report = self.build()
        capacity = report['stages']['calibrated_unzen']['capacity']
        inherited = capacity['inherited_unzen_budget_patched']
        self.assertEqual(inherited['floor_threshold_pgu_equivalent'], 0)
        self.assertEqual(inherited['exact_threshold_pgu_equivalent'], dict(numerator=1, denominator=3))
        self.assertEqual(inherited['integer_ceiling_frontier']['accepted_count'], 0)
        self.assertEqual(inherited['exact_budget_frontier']['admitted_inputs'], [boundary['input']['sha256']])
        self.assertEqual(inherited['exact_budget_frontier'], capacity['matched']['unzen'])
        self.assertEqual(capacity['matched']['patched']['accepted_count'], 4)

    def test_inherited_budget_applies_reserves_before_exact_comparison(self):
        self.profile['conversion'] = dict(kind='provided_ratio', ratio=dict(numerator=2, denominator=3),
            purpose='Synthetic exact reserve comparison')
        self.profile['limits']['raw_unzen'] = 3
        self.profile['tariffs']['ecmul_scalar_work_pgu'] = 1
        first = self.case('reserve-fit', raw=1, actual=100, work=1)
        self.case('reserve-over', raw=2, actual=1000, work=1)
        report = self.build()
        inherited = report['stages']['ecmul']['capacity']['inherited_unzen_budget_patched']
        self.assertEqual(inherited['exact_budget_frontier']['admitted_inputs'], [first['input']['sha256']])
        self.assertEqual(inherited['integer_ceiling_frontier']['admitted_inputs'], [first['input']['sha256']])

    def test_linear_access_list_precedes_pairs_without_double_charging_items(self):
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node']['pricing'].update(
            access_list_construction_pairs=3, access_list_construction_other_pairs=2, access_list_address_pairs=1))
        row = self.build()['rows'][1]
        self.assertEqual(row['stages']['ecmul']['per_block_pgu_equivalent'], [500])
        self.assertEqual(row['stages']['access_list_linear']['per_block_pgu_equivalent'], [520])
        self.assertEqual(row['stages']['access_list_pairs']['per_block_pgu_equivalent'], [680])
        self.assertEqual(row['stages']['container']['per_block_pgu_equivalent'], [700])

    def test_old_quotes_without_raw_key_count_do_not_get_linear_zero(self):
        self.mutate_quote(self.evaluation, lambda quote: quote['quote']['blocks'][0]['node']
            ['semantic_counters'].pop('access_list_raw_storage_keys'))
        row = self.build()['rows'][1]
        self.assertEqual(row['stages']['ecmul']['status'], 'complete')
        for stage in ('access_list_linear', 'access_list_pairs', 'container'):
            self.assertEqual(row['stages'][stage]['status'], 'not_available')
            self.assertIn(dict(block_index=0, selector='access_list_raw_storage_keys'),
                          row['stages'][stage]['missing'])


if __name__ == '__main__':
    unittest.main()

class QuoteActualOnlyTests(unittest.TestCase):
    """Whole-proposal observed inputs cannot acquire receipt-qualified capacity."""
    setUp = TargetedUnzenTests.setUp
    case = TargetedUnzenTests.case
    write_inputs = TargetedUnzenTests.write_inputs
    build = TargetedUnzenTests.build
    mutate_quote = TargetedUnzenTests.mutate_quote

    def as_quote_actual(self, row):
        row['measurement'] = 'quote_actual_only'
        row.pop('native')
        quote = self.mutate_quote(row, lambda q: [b.update(block_number=100+i)
            for i, b in enumerate(q['quote']['blocks'])])
        headers = dict(blocks=[dict(number=100+i, gas_used=1000000,
            header_fields=dict(number=hex(100+i), gasUsed=hex(1000000)))
            for i in range(len(quote['quote']['blocks']))])
        row['headers'] = self.fixture.write(row['sample_id']+'.headers.json', headers)
        return row

    def test_whole_proposal_shape_without_native_and_no_capacity_upgrade(self):
        self.as_quote_actual(self.train)
        self.as_quote_actual(self.evaluation)
        report = self.build()
        self.assertEqual(report['stages']['ecmul']['shape']['status'], 'complete')
        self.assertEqual(report['stages']['ecmul']['capacity']['eligible']['count'], 0)
        self.assertTrue(all(r['ineligible_reason'] == 'quote_actual_only' for r in report['rows']))
        self.assertTrue(all(r['measurement'] == 'quote_actual_only' for r in report['rows']))
        self.assertEqual(report['rows'][1]['actual_proposal_sp1_pgu'], 500)
        self.assertNotIn('user_evm_gas_used_total', report['rows'][1])

    def test_mixed_receipt_and_observation_scopes_keep_capacity_separate(self):
        self.as_quote_actual(self.evaluation)
        report = self.build()
        self.assertEqual(report['stages']['ecmul']['capacity']['eligible']['count'], 1)
        self.assertEqual(report['stages']['ecmul']['shape']['status'], 'complete')

    def test_quote_actual_cannot_smuggle_native_or_missing_headers(self):
        original = self.evaluation['native']
        self.as_quote_actual(self.evaluation)
        self.evaluation['native'] = original
        with self.assertRaisesRegex(ValueError, 'quote_actual_only.*native'):
            self.build()
        self.evaluation.pop('native'); self.evaluation.pop('headers')
        with self.assertRaisesRegex(ValueError, 'headers'):
            self.build()

    def test_header_sequence_gas_duplicate_fields_and_hash_are_checked(self):
        self.as_quote_actual(self.evaluation)
        ref = self.evaluation['headers']; path = self.root/ref['path']
        original = json.loads(path.read_text())
        for update, message in (
            (lambda d: d['blocks'][0].update(number=101), 'header.*number'),
            (lambda d: d['blocks'][0].update(gas_used=-1), 'header gas'),
            (lambda d: d['blocks'][0]['header_fields'].update(gasUsed='0x01'), 'header gas'),
        ):
            bad = copy.deepcopy(original); update(bad); path.write_text(json.dumps(bad))
            ref['sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
            with self.assertRaisesRegex(ValueError, message): self.build()
        path.write_text(json.dumps(original)); ref['sha256'] = '0'*64
        with self.assertRaisesRegex(ValueError, 'identity mismatch'): self.build()

    def test_quote_actual_canonical_actual_and_block_budgets(self):
        self.as_quote_actual(self.evaluation)
        with self.assertRaisesRegex(ValueError, 'total block budget'):
            self.build(resource_limits=dict(blocks=1))
        actual_path = self.root/self.evaluation['actual']['path']
        actual = json.loads(actual_path.read_text()); actual['guest_input_sha256'] = '0'*64
        actual_path.write_text(json.dumps(actual))
        self.evaluation['actual']['sha256'] = hashlib.sha256(actual_path.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, 'SP1/input identity'): self.build()

    def test_multi_block_actual_stays_whole(self):
        self.fixture.manifest['samples'].remove(self.evaluation)
        self.evaluation = self.case('evaluation-multi', raw=200, actual=500, work=1, blocks=2)
        self.profile['partition']['evaluation_inputs'] = [self.evaluation['input']['sha256']]
        self.as_quote_actual(self.evaluation)
        report = self.build()
        row = report['rows'][1]
        self.assertEqual(row['block_count'], 2)
        self.assertEqual(row['actual_proposal_sp1_pgu'], 500)
        self.assertEqual(row['block_evm_gas_used'], [1000000, 1000000])
        self.assertEqual(report['stages']['ecmul']['capacity']['eligible']['count'], 1)

    def test_header_preflight_count_checked_before_quote_traversal(self):
        self.as_quote_actual(self.evaluation)
        def extra_block(quote):
            quote['proposal']['block_count'] = 2
            quote['quote']['blocks'].append(copy.deepcopy(quote['quote']['blocks'][0]))
        self.mutate_quote(self.evaluation, extra_block)
        module = load()
        with mock.patch.object(module.workload, 'validate_quote', wraps=module.workload.validate_quote) as validate:
            with self.assertRaisesRegex(ValueError, 'block count mismatch'):
                module.build_report([self.fixture.path], self.write_inputs(), resource_limits=dict(blocks=2))
            self.assertEqual(validate.call_count, 1)
            self.assertEqual(validate.call_args.args[0]['key']['sample_id'], self.train['quote_key']['sample_id'])
