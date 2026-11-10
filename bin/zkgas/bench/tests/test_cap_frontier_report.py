"""Offline cap frontiers from authenticated synthetic artifacts; no executors."""
import copy
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))


def load():
    path = BENCH / 'cap_frontier_report.py'
    assert path.is_file(), 'cap-frontier report tool is missing'
    spec = importlib.util.spec_from_file_location('cap_report_subject', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


class CapFrontierTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / 'manifest.json'
        self.contract = {'execution_profile': 'evm-gas-only', 'model_id': 'model-v1',
            'identities': dict(quote='1'*64, sp1='2'*64, elf='3'*64, program_manifest='4'*64)}
        self.manifest = dict(schema_version=1, contract=self.contract, samples=[])

    def write(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value))
        return dict(path=name, sha256=digest(path.read_bytes()))

    def case(self, sid, *, nodes=(100,), raw=(100_000_000,), gas=30_000_000, actual=200,
             compressed=False, scaled=None):
        data = ('canonical-input-' + sid).encode()
        name = sid + '.bin' + ('.gz' if compressed else '')
        path = self.root / name
        path.write_bytes(gzip.compress(data, mtime=0) if compressed else data)
        inp = dict(path=name, sha256=digest(data), byte_length=len(data),
                   compression='gzip' if compressed else 'none', file_sha256=digest(path.read_bytes()))
        blocks = len(nodes)
        public = '0x' + '07'*32
        native = dict(schema_version=1, execution_profile='evm-gas-only',
            guest_input_sha256=digest(data), guest_input_bincode_length=len(data),
            public_output=public, public_values=[7]*32, block_count=blocks,
            blocks=[dict(block_index=i, block_gas_used=gas, user_transaction_gas_used=gas-1000)
                    for i in range(blocks)], user_transaction_gas_used_total=blocks*(gas-1000),
            receipts_by_block=[[dict(transaction_index=0, gas_used=1000, cumulative_gas_used=1000, success=True),
                dict(transaction_index=1, gas_used=gas-1000, cumulative_gas_used=gas, success=False)]
                for _ in range(blocks)])
        scale = 10**18
        scaled = list(scaled) if scaled is not None else [node*scale for node in nodes]
        key = dict(corpus_id='original-corpus', sample_id='quote-' + sid, guest_input_sha256=digest(data))
        quote = dict(schema_version=1, status='complete', execution_profile='evm-gas-only', key=key,
            guest_input_byte_length=len(data), executable={'sha256': self.contract['identities']['quote']},
            calibration_profile='model-v1', proposal=dict(public_output=public, block_count=blocks),
            quote=dict(schema_version=1, execution_profile='evm-gas-only', model_id='model-v1',
                guest_input_sha256=digest(data), guest_input_bincode_length=len(data), public_output=public,
                blocks=[dict(block_index=i, node=dict(unzen_zkgas=raw[i], pricing=dict(
                    known_subtotal_scaled=str(scaled[i]), known_subtotal_pgu=str(nodes[i])))) for i in range(blocks)],
                node=dict(unzen_zkgas_sum=str(sum(raw)), block_local_known_scaled=str(sum(scaled)),
                    sum_of_block_known_pgu=str(sum(nodes)), block_local_known_pgu=str((sum(scaled)+scale-1)//scale)),
                prove=dict(total_pgu='300')))
        report = dict(schema_version=1, exit_code=0, execution_profile='evm-gas-only',
            program_manifest_sha256=self.contract['identities']['program_manifest'],
            measurement_policy='sp1-6.3-streaming-v1', sp1_execution_engine='gas-estimator',
            sp1_sdk_version='6.3.0', sp1_core_executor_version='6.3.0', sp1_core_executor_runner_version='6.3.0',
            guest_input_sha256=digest(data), guest_input_bincode_length=len(data),
            elf_sha256=self.contract['identities']['elf'], executable_sha256=self.contract['identities']['sp1'],
            public_values=[7]*32, prover_gas=actual, execution_report=dict(gas=actual*191//10),
            total_instruction_count=100, total_syscall_count=2,
            executor_opts=dict(memory_limit=24*1024**3, gas_trace_chunk_threshold=134217728,
                gas_trace_chunk_slots=2, shard_size=1<<24, minimal_trace_chunk_threshold=16777216,
                trace_chunk_slots=5, sharding_threshold=dict(element_threshold=402653184, height_threshold=4194304)))
        self.assertEqual(report['execution_report']['gas']*10//191, actual)
        row = dict(sample_id=sid, role='control', status='complete', quote_key=key, input=inp,
            native=self.write(sid+'.native.json', native), quote=self.write(sid+'.quote.json', quote),
            actual=self.write(sid+'.actual.json', report))
        self.manifest['samples'].append(row)
        return row

    def build(self, **kwargs):
        self.path.write_text(json.dumps(self.manifest))
        return load().build_report(self.path, **kwargs)

    def mutate(self, row, artifact, field, value):
        path = self.root / row[artifact]['path']
        obj = json.loads(path.read_text())
        parts = field.split('.')
        target = obj
        for part in parts[:-1]:
            target = target[int(part)] if isinstance(target, list) else target[part]
        target[parts[-1]] = value
        path.write_text(json.dumps(obj))
        row[artifact]['sha256'] = digest(path.read_bytes())

    def natural(self, rows):
        samples, quotes = [], []
        for row in rows:
            samples.append(dict(sample_id=row['quote_key']['sample_id'], guest_input={k: row['input'][k]
                for k in ('path', 'sha256', 'byte_length')}))
            quotes.append(json.loads((self.root / row['quote']['path']).read_text()))
        record = self.write('natural-inputs.json', dict(schema_version=1, corpus_id='original-corpus', samples=samples))
        path = self.root / 'natural-quotes.jsonl'
        path.write_text(''.join(json.dumps(q)+'\n' for q in quotes))
        self.manifest['natural'] = dict(input_manifest=record,
            quotes=dict(path=path.name, sha256=digest(path.read_bytes())))
        return samples, quotes

    def test_empty_frontiers_and_failures_have_no_numeric_success(self):
        report = self.build(node_limits=[0, 100])
        self.assertIsNone(report['unzen_frontier']['max_actual_pgu'])
        self.assertIsNone(report['node_frontiers'][0]['max_actual_pgu'])
        self.assertIsNone(report['natural_sensitivity'])
        for status in ('failed', 'screened', 'not_attempted'):
            self.manifest['samples'].append(dict(sample_id=status, role='candidate', status=status,
                quote_key=dict(corpus_id='original-corpus', sample_id=status, guest_input_sha256='a'*64),
                failure={'message': 'retained reason'}))
        report = self.build()
        self.assertEqual(len(report['rows']), 3)
        for row in report['rows']:
            self.assertIsNone(row['actual'])
            self.assertIsNone(row['actual_proposal_sp1_pgu'])
            self.assertEqual(row['failure']['message'], 'retained reason')

    def test_both_cap_equalities_and_signed_positive_gaps(self):
        first = self.case('equal', actual=200)
        self.case('above-node', nodes=(101,), raw=(100_000_001,), actual=400)
        self.case('below', nodes=(99,), raw=(99_999_999,), actual=100)
        report = self.build(node_limits=[99, 100, 101])
        self.assertEqual(report['unzen_frontier']['accepted_count'], 2)
        self.assertEqual(report['unzen_frontier']['max_actual_sample_ids'], ['equal'])
        self.assertEqual([r['accepted_count'] for r in report['node_frontiers']], [1, 2, 3])
        row = report['rows'][0]
        self.assertEqual(row['quote_key'], first['quote_key'])
        self.assertEqual(row['block_evm_gas_used'], [30_000_000])
        self.assertEqual(row['raw_unzen_per_block'], [100_000_000])
        self.assertEqual(row['actual_minus_full_pgu'], -100)
        self.assertEqual(row['proposal_shortfall_pgu'], 0)
        self.assertEqual(row['actual_over_node'], dict(numerator=200, denominator=100))
        self.assertFalse(row['native']['receipts_by_block'][0][1]['success'])
        self.assertEqual(report['source_manifest']['sha256'], digest(self.path.read_bytes()))
        self.assertEqual(row['sources']['actual']['sha256'], first['actual']['sha256'])

    def test_gzip_identity_and_multiblock_literal_rounding_exclusion(self):
        self.case('multi', nodes=(1, 1), raw=(1, 1), actual=200,
                  scaled=(1, 1), compressed=True)
        self.case('over-budget', gas=30_000_001)
        report = self.build(node_limits=[2])
        row = report['rows'][0]
        self.assertEqual(row['node_literal_sum_pgu'], 2)
        self.assertEqual(row['node_aggregate_ceil_pgu'], 1)
        self.assertEqual(row['ineligible_reason'], 'multi_block_proposal')
        self.assertEqual(report['node_frontiers'][0]['accepted_count'], 0)
        self.assertIsNone(report['unzen_frontier']['max_actual_pgu'])
        self.assertIn('no per-block actual', report['scope'])

    def test_corrupted_complete_joins_are_rejected(self):
        row = self.case('valid')
        original = copy.deepcopy(row)
        originals = {k: (self.root / row[k]['path']).read_bytes() for k in ('native', 'quote', 'actual')}
        mutations = [('native', 'guest_input_sha256', '0'*64), ('native', 'public_values', [8]*32),
            ('quote', 'key.sample_id', 'wrong'), ('quote', 'key.corpus_id', 'wrong'),
            ('quote', 'quote.guest_input_sha256', '0'*64), ('quote', 'proposal.public_output', '0x'+'08'*32),
            ('quote', 'quote.model_id', 'other'), ('quote', 'executable.sha256', '0'*64),
            ('quote', 'execution_profile', 'legacy-unzen'), ('quote', 'proposal.block_count', 2),
            ('quote', 'quote.blocks.0.node.unzen_zkgas', True),
            ('quote', 'quote.blocks.0.node.unzen_zkgas', '100000000'),
            ('quote', 'quote.blocks.0.node.unzen_zkgas', -1),
            ('quote', 'quote.node.unzen_zkgas_sum', '0'),
            ('quote', 'quote.node.sum_of_block_known_pgu', '0'), ('quote', 'quote.prove.total_pgu', '0'),
            ('actual', 'guest_input_sha256', '0'*64), ('actual', 'elf_sha256', '0'*64),
            ('actual', 'program_manifest_sha256', '0'*64), ('actual', 'executable_sha256', '0'*64),
            ('actual', 'public_values', [8]*32), ('actual', 'prover_gas', 201), ('actual', 'prover_gas', 0),
            ('actual', 'execution_report.gas', -1),
            ('actual', 'execution_report.gas', 0), ('actual', 'execution_report.gas', True),
            ('actual', 'execution_report.gas', '3820'), ('actual', 'exit_code', True),
            ('native', 'block_count', True), ('native', 'blocks.0.block_gas_used', 2)]
        for artifact, field, value in mutations:
            row.clear(); row.update(copy.deepcopy(original))
            for name, data in originals.items():
                (self.root / row[name]['path']).write_bytes(data)
            self.mutate(row, artifact, field, value)
            with self.subTest(artifact=artifact, field=field, value=value), self.assertRaises((ValueError, KeyError)):
                self.build()

    def test_source_hash_input_identity_duplicates_and_policies_reject(self):
        row = self.case('valid', compressed=True)
        original = copy.deepcopy(row)
        for field, value in [('sha256', '0'*64), ('file_sha256', '0'*64), ('byte_length', 0),
                             ('compression', 'zip')]:
            row['input'] = dict(original['input'], **{field: value})
            with self.subTest(field=field), self.assertRaises(ValueError): self.build()
        row['input'] = original['input']
        for artifact in ('native', 'quote', 'actual'):
            row[artifact] = dict(original[artifact], sha256='0'*64)
            with self.subTest(artifact=artifact), self.assertRaises(ValueError): self.build()
            row[artifact] = original[artifact]
        duplicate = copy.deepcopy(row); duplicate['sample_id'] = 'duplicate'
        self.manifest['samples'].append(duplicate)
        with self.assertRaises(ValueError): self.build()
        self.manifest['samples'].pop()
        for kwargs in [dict(unzen_limit=True), dict(unzen_limit=-1), dict(node_limits=[-1]), dict(node_limits=[True])]:
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError): self.build(**kwargs)
        path = self.root / row['actual']['path']; saved = path.read_bytes(); path.unlink()
        target = self.root / 'actual-source'; target.write_bytes(saved); path.symlink_to(target)
        with self.assertRaises(ValueError): self.build()

    def test_natural_sensitivity_and_covering_threshold_use_block_prices(self):
        a = self.case('natural-a', nodes=(100, 201), raw=(100_000_000, 100_000_001))
        b = self.case('natural-b', nodes=(150,), raw=(99,))
        self.natural([a, b])
        report = self.build(node_limits=[0, 149, 150, 201])
        natural = report['natural_sensitivity']
        self.assertEqual(natural['block_count'], 3)
        self.assertEqual(natural['proposal_count'], 2)
        self.assertEqual(natural['unzen']['accepted_blocks'], 2)
        self.assertEqual(natural['unzen']['accepted_proposals'], 1)
        self.assertEqual(natural['minimum_node_threshold_covering_old_admitted_blocks_pgu'], 150)
        self.assertEqual(natural['node_frontiers'][2]['block_acceptance_ratio'], dict(numerator=2, denominator=3))
        self.assertEqual(len(report['node_frontiers']), 4)
        report = self.build(node_limits=[0])
        self.assertEqual([r['node_limit_pgu'] for r in report['node_frontiers']], [0, 150])
        self.assertNotIn('actual', natural)

    def test_natural_missing_extra_duplicate_hash_and_public_errors_reject(self):
        row = self.case('natural')
        _, quotes = self.natural([row])
        path = self.root / self.manifest['natural']['quotes']['path']
        for values in [[], quotes*2, [dict(quotes[0], key=dict(quotes[0]['key'], sample_id='extra'))]]:
            path.write_text(''.join(json.dumps(q)+'\n' for q in values))
            self.manifest['natural']['quotes']['sha256'] = digest(path.read_bytes())
            with self.subTest(values=len(values)), self.assertRaises(ValueError): self.build()
        self.natural([row])
        self.manifest['natural']['input_manifest']['sha256'] = '0'*64
        with self.assertRaises(ValueError): self.build()
        self.natural([row]); self.manifest['natural']['quotes']['sha256'] = '0'*64
        with self.assertRaises(ValueError): self.build()
        self.natural([row]); (self.root / row['input']['path']).write_bytes(b'corrupt')
        with self.assertRaises(ValueError): self.build()

    def test_natural_validation_is_independent_of_complete_rows(self):
        row = self.case('natural')
        self.natural([row])
        self.manifest['samples'] = []
        record = self.manifest['natural']['quotes']
        path = self.root / record['path']
        original = json.loads(path.read_text())
        mutations = [('quote.model_id', 'other'), ('calibration_profile', 'other'),
            ('executable.sha256', '0'*64), ('execution_profile', 'legacy-unzen'),
            ('quote.execution_profile', 'legacy-unzen'), ('quote.public_output', '0x'+'08'*32),
            ('guest_input_byte_length', 1), ('quote.guest_input_sha256', '0'*64),
            ('quote.blocks.0.node.unzen_zkgas', True), ('quote.node.unzen_zkgas_sum', '0'),
            ('quote.blocks.0.node.pricing.known_subtotal_pgu', '99'), ('proposal.block_count', 2)]
        for field, value in mutations:
            quote = copy.deepcopy(original)
            target = quote
            parts = field.split('.')
            for part in parts[:-1]:
                target = target[int(part)] if isinstance(target, list) else target[part]
            target[parts[-1]] = value
            path.write_text(json.dumps(quote)+'\n')
            record['sha256'] = digest(path.read_bytes())
            with self.subTest(field=field), self.assertRaises(ValueError): self.build()
        path.write_text(json.dumps(original)+'\n')
        record['sha256'] = digest(path.read_bytes())
        manifest_record = self.manifest['natural']['input_manifest']
        source = self.root / manifest_record['path']
        inputs = json.loads(source.read_text())
        inputs['samples'] *= 2
        source.write_text(json.dumps(inputs))
        manifest_record['sha256'] = digest(source.read_bytes())
        with self.assertRaises(ValueError): self.build()

    def test_empty_natural_and_no_old_admitted_blocks_have_no_covering_threshold(self):
        self.natural([])
        self.assertIsNone(self.build(node_limits=[])['natural_sensitivity'])
        row = self.case('above-old-cap', raw=(100_000_001,))
        self.natural([row])
        report = self.build(node_limits=[0])
        natural = report['natural_sensitivity']
        self.assertIsNone(natural['minimum_node_threshold_covering_old_admitted_blocks_pgu'])
        self.assertEqual(natural['unzen']['block_acceptance_ratio'], dict(numerator=0, denominator=1))
        self.assertEqual([r['node_limit_pgu'] for r in report['node_frontiers']], [0])

    def test_frontier_helper_preserves_tied_case_ids_and_empty_maxima(self):
        self.case('tie-a', actual=200)
        self.case('tie-b', actual=200)
        report = self.build(node_limits=[100])
        self.assertEqual(report['unzen_frontier']['max_actual_sample_ids'], ['tie-a', 'tie-b'])
        self.assertEqual(load().frontier(report['rows']),
                         {key: value for key, value in report['node_frontiers'][0].items() if key != 'node_limit_pgu'})
        empty = load().frontier([])
        self.assertIsNone(empty['max_actual_pgu'])
        self.assertIsNone(empty['max_positive_node_gap_pgu'])

    def test_saved_derived_metrics_do_not_override_raw_actuals(self):
        row = self.case('raw-authority')
        self.mutate(row, 'actual', 'metrics', {'actual_sp1_pgu': 999999999})
        report = self.build()
        self.assertEqual(report['rows'][0]['actual_proposal_sp1_pgu'], 200)
        self.assertEqual(report['unzen_frontier']['max_actual_pgu'], 200)
        self.assertEqual([r['node_limit_pgu'] for r in report['node_frontiers']],
                         load().DEFAULT_NODE_LIMITS)

    def test_cli_is_exclusive_and_preserves_all_sources(self):
        self.case('complete', compressed=True)
        self.path.write_text(json.dumps(self.manifest))
        before = {p.name: p.read_bytes() for p in self.root.iterdir()}
        output = self.root / 'report.json'
        args = ['--manifest', str(self.path), '--output', str(output), '--node-limit', '100']
        self.assertEqual(load().main(args), 0)
        with self.assertRaises(FileExistsError): load().main(args)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.root.iterdir() if p != output})


if __name__ == '__main__':
    unittest.main()
