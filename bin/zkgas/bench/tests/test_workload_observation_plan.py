"""Optional artifact/command contract tests using existing fake harness, never a prover."""
import copy
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import artifacts
import workload
import test_workload as support


class ObservationPlanTests(unittest.TestCase):
    def setUp(self):
        self.h = support.HarnessTests('test_implicit_legacy_config_contract_remains_byte_identical')
        self.h.setUp()
        self.addCleanup(self.h.doCleanups)

    def plan(self):
        h = self.h
        path = h.root / 'plan.json'
        value = dict(schema_version=1, elf_sha256=artifacts.identity(h.elf)[0],
                     instruction_limit=100, watches=[dict(label='sample', pc='0x0000000000001000',
                     registers=[0], sample_limit=0, skip_hits=0)])
        h.write(path, value)
        config = artifacts.read_json(h.config)
        config['artifacts']['observation_plan'] = dict(path=path.name, sha256=artifacts.identity(path)[0])
        h.config.write_text(json.dumps(config))
        return path

    def test_omitted_plan_preserves_contract_and_command(self):
        h = self.h
        paths, frozen = workload.load_config(h.config)
        self.assertNotIn('observation_plan', paths)
        self.assertNotIn('observation_plan', frozen['identities'])
        expected = {'schema_version': 1, 'corpus_id': 'test-corpus', 'model_id': 'model-v1',
                    'identities': {k: item['sha256'] for k, item in artifacts.read_json(h.config)['artifacts'].items()},
                    'measurement_kind': 'local execute-only SP1; not a proof',
                    'production_sdk_parity_verified': False, 'limits': workload.DEFAULT_LIMITS,
                    'provenance': {}}
        self.assertEqual(frozen, expected)
        self.assertEqual(h.invoke('run', h.catalog([('control', {})])), 0)
        folder = h.attempts('control')[0]
        command = artifacts.read_json(folder / 'sp1-command.json')['command']
        self.assertEqual(command, [str(h.tool), '--input-bincode', str(folder / 'input.bin'),
                        '--elf', str(h.elf), '--output', str(folder / 'sp1.json'),
                        '--resource-receipt', str(folder / 'sp1-resources.json')])
        self.assertFalse((folder / 'observation-plan.json').exists())

    def test_plan_bound_retained_and_passed_canonical_independently(self):
        h = self.h; h.execution_profile()
        path = self.plan(); paths, contract = workload.load_config(h.config)
        self.assertEqual(paths['observation_plan'], path)
        self.assertEqual(contract['identities']['observation_plan'], artifacts.identity(path)[0])
        original = workload.validate_sp1
        def canonical_with_incomplete_trace(report, native, input_identity, frozen):
            report = dict(report, observation={'status': 'instruction_limit', 'complete': False,
                                              'samples_complete': False})
            return original(report, native, input_identity, frozen)
        with patch.object(workload, 'validate_sp1', side_effect=canonical_with_incomplete_trace):
            self.assertEqual(h.invoke('run', h.catalog([('control', {})])), 0)
            folder = h.attempts('control')[0]
            status = workload.read_attempt(folder)
        self.assertEqual(status['status'], 'complete')
        retained = folder / 'observation-plan.json'
        self.assertEqual(retained.read_bytes(), path.read_bytes())
        command = artifacts.read_json(folder / 'sp1-command.json')['command']
        self.assertEqual(command[-4:], ['--program-manifest', str(folder / 'program-manifest.json'),
                                         '--observation-plan', str(retained)])
        self.assertIn('observation-plan.json', {r['path'] for r in status['artifacts']})
        retained.write_text('{}')
        # Even a rewritten generic receipt cannot replace the frozen plan identity.
        for record in status['artifacts']:
            if record['path'] == retained.name:
                record.update(artifacts.receipt(retained))
        (folder / 'status.json').write_text(json.dumps(status))
        with self.assertRaisesRegex(ValueError, 'retained observation plan identity'):
            workload.read_attempt(folder)

    def test_plan_mutation_rejected_before_any_launch(self):
        h = self.h; path = self.plan(); paths, frozen = workload.load_config(h.config)
        path.write_text('{}')
        with patch.object(workload, 'run') as launch:
            with self.assertRaisesRegex(ValueError, 'observation_plan changed'): workload.recheck(paths, frozen)
            launch.assert_not_called()
        with self.assertRaisesRegex(ValueError, 'observation_plan hash mismatch'): workload.load_config(h.config)

    def test_plan_header_and_size_rejected_at_config_boundary(self):
        h = self.h; path = self.plan(); config = artifacts.read_json(h.config)
        for wrong in ([], dict(schema_version=True, elf_sha256=artifacts.identity(h.elf)[0]),
                      dict(schema_version=1, elf_sha256='0' * 64)):
            path.write_text(json.dumps(wrong))
            config['artifacts']['observation_plan']['sha256'] = artifacts.identity(path)[0]
            h.config.write_text(json.dumps(config))
            with self.assertRaises(ValueError): workload.load_config(h.config)
        path.write_bytes(b' ' * (64 * 1024 + 1))
        config['artifacts']['observation_plan']['sha256'] = artifacts.identity(path)[0]
        h.config.write_text(json.dumps(config))
        with self.assertRaises(ValueError): workload.load_config(h.config)


if __name__ == '__main__': unittest.main()
