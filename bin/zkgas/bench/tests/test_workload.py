import gzip
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import artifacts
import process_guard
import shared_memory
import workload


FAKE = r'''
import base64, hashlib, json, os, pathlib, sys, time, uuid
args = dict(zip(sys.argv[1::2], sys.argv[2::2]))
def read(path): return json.loads(pathlib.Path(path).read_text())
def write(path, data):
    with pathlib.Path(path).open('x') as out: json.dump(data, out)
def digest(path): return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
output = [7] * 32
if '--input' in args:
    request = pathlib.Path(args['--input']).read_bytes()
    case = json.loads(request)
    if case.get('native_reject'): sys.exit(3)
    if case.get('native_sleep'): time.sleep(10)
    pathlib.Path(args['--output-bincode']).write_bytes(request)
    write(args['--output'], {'ordinary_guest_input': True})
    write(args['--report'], {'schema_version': 1,
        'guest_input_sha256': hashlib.sha256(request).hexdigest(),
        'guest_input_bincode_length': len(request),
        'construction_request_sha256': hashlib.sha256(request).hexdigest(),
        'public_values': output, 'public_output': '0x'+bytes(output).hex()})
else:
    raw = pathlib.Path(args['--input-bincode']).read_bytes()
    case = json.loads(raw)
    sha = hashlib.sha256(raw).hexdigest()
    if '--corpus-id' in args:
        write(args['--output'], {'schema_version': 1, 'status': 'complete',
            'key': {'corpus_id': args['--corpus-id'], 'sample_id': args['--sample-id'],
                    'guest_input_sha256': sha}, 'guest_input_byte_length': len(raw),
            'executable': {'sha256': digest(sys.argv[0])}, 'calibration_profile': 'model-v1',
            'proposal': {'public_output': '0x'+bytes(output).hex()},
            'quote': {'model_id': 'model-v1', 'node': {'block_local_known_pgu': str(case.get('node', 3))},
                      'prove': {'total_pgu': '100'}}})
    else:
        if case.get('sp1_fail'): sys.exit(4)
        sdkid = 'sp1_'+base64.urlsafe_b64encode(uuid.uuid4().bytes).decode()
        resources = []
        for name in (sdkid+'_m', sdkid+'_t', 'sem.'+sdkid+'_t_filled', 'sem.'+sdkid+'_t_empty'):
            resource = pathlib.Path(sys.argv[0]).parent/'fake-shm'/name
            with resource.open('xb') as out: out.write(b'owned')
            info = resource.stat()
            resources.append({'name': name, 'device': info.st_dev, 'inode': info.st_ino, 'uid': info.st_uid})
        write(args['--resource-receipt'], {'schema_version': 1, 'stage': 'sp1_owned_shared_memory',
              'producer_pid': os.getpid(), 'resources': resources})
        write(args['--output'], {'schema_version': 1, 'exit_code': 0,
            'measurement_policy': 'sp1-6.3-streaming-v1', 'sp1_execution_engine': 'gas-estimator',
            'sp1_sdk_version': '6.3.0', 'sp1_core_executor_version': '6.3.0',
            'sp1_core_executor_runner_version': '6.3.0',
            'guest_input_sha256': '0'*64 if case.get('bad_hash') else sha,
            'guest_input_bincode_length': len(raw), 'elf_sha256': digest(args['--elf']),
            'executable_sha256': digest(sys.argv[0]),
            'public_values': [8]*32 if case.get('bad_output') else output,
            'prover_gas': case.get('actual', 10), 'total_instruction_count': 100,
            'total_syscall_count': 2,
            'executor_opts': {'memory_limit': 24*1024**3, 'gas_trace_chunk_threshold': 134217728,
                'gas_trace_chunk_slots': 2, 'shard_size': 1<<24,
                'minimal_trace_chunk_threshold': 16777216, 'trace_chunk_slots': 5,
                'sharding_threshold': {'element_threshold': 402653184, 'height_threshold': 4194304}}})
'''


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.fake_shm = self.root / 'fake-shm'
        self.fake_shm.mkdir()
        cleanup = patch.object(process_guard, 'cleanup_shared_memory',
            side_effect=lambda path, pid: shared_memory.cleanup(path, pid, self.fake_shm))
        cleanup.start()
        self.addCleanup(cleanup.stop)
        self.tool = self.root / 'fake-tool'
        self.tool.write_text(f'#!{sys.executable}\n' + FAKE)
        self.tool.chmod(0o700)
        self.elf, self.model = self.root / 'guest.elf', self.root / 'model.json'
        self.elf.write_bytes(b'explicit-program')
        self.model.write_text('{"artifact_sha256":"model-v1"}')
        self.config = self.root / 'config.json'
        self.write(self.config, {'schema_version': 1, 'corpus_id': 'test-corpus', 'model_id': 'model-v1',
                                'artifacts': {name: {'path': path.name, 'sha256': artifacts.identity(path)[0]}
                                              for name, path in [('native', self.tool), ('quote', self.tool),
                                                                 ('sp1', self.tool), ('elf', self.elf),
                                                                 ('model', self.model)]}})
        self.runroot = self.root / 'run'
        # Disk reservation itself is tested below. Integration tests remain deterministic on
        # a small development disk and still run the actual supervisor/process cleanup.
        self.disk = patch('shutil.disk_usage', return_value=type('Space', (), {'free': 10**12})())
        self.disk.start()
        self.addCleanup(self.disk.stop)

    def write(self, path, value):
        artifacts.exclusive_json(path, value)

    def catalog(self, values):
        cases = []
        for sid, value in values:
            path = self.root / (sid + '.json')
            self.write(path, value)
            cases.append({'sample_id': sid, 'request_path': path.name,
                          'request_sha256': artifacts.identity(path)[0], 'role': 'control'})
        path = self.root / 'catalog.json'
        self.write(path, {'schema_version': 1, 'cases': cases})
        return path

    def invoke(self, action, catalog=None, extra=()):
        arguments = [action, '--output-root', str(self.runroot)]
        if action in ('run', 'screen', 'measure'):
            arguments += ['--config', str(self.config)]
        if catalog:
            arguments += ['--catalog', str(catalog)]
        return workload.main(arguments + list(extra))

    def attempts(self, sid):
        return sorted((self.runroot / sid).glob('attempt-*'))

    def test_full_pipeline_exact_ranking_and_validated_resume(self):
        # Values chosen so converting to float would lose the ordering.
        large = 2**60
        catalog = self.catalog([('control', {'actual': large, 'node': large}),
                                ('case', {'actual': large+1, 'node': large})])
        self.assertEqual(self.invoke('run', catalog), 0)
        result = workload.summary(self.runroot)
        self.assertEqual(result['ranked_sample_ids'], ['case', 'control'])
        self.assertEqual(len(result['observations']['observations']), 2)
        self.assertEqual(self.invoke('run', catalog), 0)
        self.assertEqual(len(self.attempts('case')), 1)
        self.assertEqual(self.invoke('verify'), 0)
        output = self.root / 'summary.json'
        self.assertEqual(self.invoke('report', extra=['--output', str(output)]), 0)
        with self.assertRaises(FileExistsError):
            self.invoke('report', extra=['--output', str(output)])

    def test_screen_then_measure_reuses_validated_native_input(self):
        catalog = self.catalog([('control', {})])
        self.assertEqual(self.invoke('screen', catalog), 0)
        screened = self.attempts('control')[0]
        self.assertEqual(workload.read_attempt(screened)['status'], 'screened')
        self.assertEqual(self.invoke('run', catalog), 0)
        attempts = self.attempts('control')
        self.assertEqual(len(attempts), 2)
        completed = next(p for p in attempts if workload.read_attempt(p)['status'] == 'complete')
        self.assertFalse((completed / 'native-command.json').exists())
        self.assertTrue((completed / 'sp1-command.json').exists())

    def test_retained_input_measurement_needs_no_generator(self):
        catalog = self.catalog([('control', {})])
        self.invoke('screen', catalog)
        previous = self.attempts('control')[0]
        status = workload.read_attempt(previous)
        record = next(r for r in status['artifacts'] if r['path'] == 'input.bin.gz')
        saved = self.root / 'saved.bin'
        artifacts.restore(previous / 'input.bin.gz', saved, record)
        self.runroot = self.root / 'retained-run'
        self.assertEqual(self.invoke('measure', extra=['--input-bincode', str(saved),
                         '--native-report', str(previous / 'native.json'), '--sample-id', 'saved']), 0)

    def test_native_rejection_retained_and_retry_is_explicit(self):
        catalog = self.catalog([('rejected', {'native_reject': True}), ('unattempted', {})])
        self.assertEqual(self.invoke('run', catalog), 1)
        folder = self.attempts('rejected')[0]
        status = workload.read_attempt(folder)
        self.assertEqual(status['status'], 'native_rejected')
        self.assertEqual(artifacts.read_json(folder / 'failure-observation.json')['status'], 'failed')
        self.assertFalse((folder / 'sp1-command.json').exists())
        self.assertFalse((self.runroot / 'unattempted').exists())
        self.assertEqual(workload.summary(self.runroot)['attempts'][-1]['status'], 'not_attempted')
        self.assertEqual(workload.summary(self.runroot)['observations']['observations'][0]['failure']['code'],
                         'native_rejected')
        with self.assertRaisesRegex(ValueError, 'retry-failed'):
            self.invoke('run', catalog)
        self.assertEqual(self.invoke('run', catalog, ['--retry-failed']), 1)
        self.assertEqual(len(self.attempts('rejected')), 2)

    def test_sp1_fail_hash_mismatch_and_output_mismatch_are_unscored(self):
        for flag, expected in [('sp1_fail', 'measurement_failed'), ('bad_hash', 'validation_failed'),
                               ('bad_output', 'validation_failed')]:
            with self.subTest(flag=flag):
                sub = self.root / flag
                sub.mkdir()
                request = sub / 'request.json'
                self.write(request, {flag: True})
                catalog = sub / 'catalog.json'
                self.write(catalog, {'schema_version': 1, 'cases': [{'sample_id': flag,
                    'request_path': 'request.json', 'request_sha256': artifacts.identity(request)[0]}]})
                self.runroot = sub / 'run'
                self.assertEqual(self.invoke('run', catalog), 1)
                status = workload.read_attempt(self.attempts(flag)[0])
                self.assertEqual(status['status'], expected)
                self.assertNotIn('metrics', status)
                self.assertEqual(workload.summary(self.runroot)['ranked_sample_ids'], [])

    def test_continue_after_failure_attempts_remaining_cases_but_preserves_nonzero_exit(self):
        catalog = self.catalog([('rejected', {'native_reject': True}), ('control', {})])
        self.assertEqual(self.invoke('run', catalog, ['--continue-after-failure']), 1)
        result = workload.summary(self.runroot)
        self.assertEqual([row['status'] for row in result['attempts']], ['complete', 'native_rejected'])
        self.assertEqual(result['ranked_sample_ids'], ['control'])
        with self.assertRaisesRegex(ValueError, 'retry-failed'):
            self.invoke('run', catalog, ['--continue-after-failure'])
        self.assertEqual(len(self.attempts('rejected')), 1)

    def test_corrupt_successful_retention_cannot_be_retried(self):
        catalog = self.catalog([('control', {})])
        self.invoke('run', catalog)
        folder = self.attempts('control')[0]
        archive = folder / 'input.bin.gz'
        archive.write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, 'corrupt successful'):
            self.invoke('run', catalog, ['--retry-failed'])

    def test_partial_status_requires_explicit_retry_and_remains_immutable(self):
        catalog = self.catalog([('control', {})])
        self.runroot.mkdir()
        paths, contract = workload.load_config(self.config)
        contract['catalog_sha256'] = artifacts.identity(catalog)[0]
        contract['cases'] = artifacts.read_json(catalog)['cases']
        self.write(self.runroot / 'contract.json', contract)
        partial = self.runroot / 'control' / 'attempt-partial'
        partial.mkdir(parents=True)
        (partial / 'status.json').write_text('{truncated')
        with self.assertRaisesRegex(ValueError, 'retry-failed'):
            self.invoke('run', catalog)
        self.assertEqual(self.invoke('run', catalog, ['--retry-failed']), 0)
        self.assertEqual((partial / 'status.json').read_text(), '{truncated')
        self.assertEqual(len(workload.summary(self.runroot)['attempts']), 2)

    def test_failed_measurement_after_screen_requires_retry(self):
        catalog = self.catalog([('control', {'sp1_fail': True})])
        self.assertEqual(self.invoke('screen', catalog), 0)
        self.assertEqual(self.invoke('run', catalog), 1)
        with self.assertRaisesRegex(ValueError, 'retry-failed'):
            self.invoke('run', catalog)
        self.assertEqual(self.invoke('run', catalog, ['--retry-failed']), 1)

    def test_changed_tool_and_request_rejected_before_execution(self):
        catalog = self.catalog([('control', {})])
        self.tool.write_text(self.tool.read_text() + '\n# changed\n')
        with self.assertRaisesRegex(ValueError, 'frozen native hash'):
            self.invoke('run', catalog)
        self.assertFalse(self.runroot.exists())

    def test_nonportable_ids_and_weakened_limits_rejected(self):
        with self.assertRaises(ValueError):
            workload.sample_id('../escape')
        config = artifacts.read_json(self.config)
        config['limits'] = {'pss_mib': 6145}
        path = self.root / 'weakened.json'
        self.write(path, config)
        with self.assertRaisesRegex(ValueError, 'cannot weaken'):
            workload.load_config(path)

    def test_model_artifact_metadata_must_match_configured_model_id(self):
        config = artifacts.read_json(self.config)
        config['model_id'] = 'different-model'
        path = self.root / 'different-model.json'
        self.write(path, config)
        with self.assertRaisesRegex(ValueError, 'model artifact metadata ID'):
            workload.load_config(path)

    def test_disk_guard_failure_retained(self):
        catalog = self.catalog([('control', {})])
        with patch('shutil.disk_usage', return_value=type('Space', (), {'free': 0})()):
            self.assertEqual(self.invoke('run', catalog), 1)
        status = workload.read_attempt(self.attempts('control')[0])
        self.assertEqual(status['status'], 'stage_failed')
        self.assertIn('insufficient disk', status['failure']['message'])


class ArtifactTests(unittest.TestCase):
    def test_roundtrip_corruption_limit_and_no_clobber(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = root / 'input.bin'
            path.write_bytes(bytes(range(256)) * 500)
            original = artifacts.identity(path)
            record = artifacts.compress(path)
            self.assertFalse(path.exists())
            archive = root / record['path']
            artifacts.restore(archive, path, record)
            self.assertEqual(artifacts.identity(path), original)
            with self.assertRaises(FileExistsError):
                artifacts.restore(archive, path, record)
            with self.assertRaises(ValueError):
                artifacts.identity(archive, compressed=True, limit=10)
            archive.write_bytes(b'bad')
            with self.assertRaisesRegex(ValueError, 'archive identity'):
                artifacts.restore(archive, root / 'new.bin', record)


class SharedMemoryTests(unittest.TestCase):
    def objects(self, root):
        sid = 'sp1_AAAAAAAAAAAAAAAAAAAAAA=='
        records = []
        for name in (sid+'_m', sid+'_t', 'sem.'+sid+'_t_filled', 'sem.'+sid+'_t_empty'):
            path = root / name
            path.write_bytes(b'owned pages')
            info = path.stat()
            records.append({'name': name, 'device': info.st_dev, 'inode': info.st_ino, 'uid': info.st_uid})
        return records

    def test_exact_receipted_objects_only_and_idempotent_cleanup(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            resources = self.objects(root)
            unknown = root / 'unrelated-object'
            unknown.write_bytes(b'preserve')
            path = root / 'receipt.json'
            artifacts.exclusive_json(path, {'schema_version': 1, 'stage': 'sp1_owned_shared_memory',
                'producer_pid': 123, 'resources': resources})
            self.assertEqual(shared_memory.cleanup(path, 123, root)['status'], 'verified')
            self.assertTrue(all(not (root / item['name']).exists() for item in resources))
            self.assertEqual(unknown.read_bytes(), b'preserve')
            self.assertEqual(shared_memory.cleanup(path, 123, root)['objects'][0]['status'], 'already_removed')

    def test_changed_identity_or_producer_cannot_delete_any_objects(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            resources = self.objects(root)
            resources[1]['inode'] += 1
            receipt = root / 'receipt.json'
            artifacts.exclusive_json(receipt, {'schema_version': 1, 'stage': 'sp1_owned_shared_memory',
                                               'producer_pid': 123, 'resources': resources})
            with self.assertRaisesRegex(ValueError, 'producer mismatch'):
                shared_memory.cleanup(receipt, 124, root)
            with self.assertRaisesRegex(ValueError, 'identity changed'):
                shared_memory.cleanup(receipt, 123, root)
            self.assertTrue(all((root / item['name']).exists() for item in resources))

    def test_empty_incomplete_and_mixed_resource_sets_never_unlink(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            complete = self.objects(root)
            mixed = [dict(item) for item in complete]
            mixed[-1]['name'] = 'sem.sp1_BBBBBBBBBBBBBBBBBBBBBB==_t_empty'
            for index, records in enumerate(([], complete[:1], complete[:3], mixed)):
                path = root / f'incomplete-{index}.json'
                artifacts.exclusive_json(path, {'schema_version': 1, 'stage': 'sp1_owned_shared_memory',
                                               'producer_pid': 123, 'resources': records})
                with self.assertRaises(ValueError):
                    shared_memory.cleanup(path, 123, root)
                self.assertTrue(all((root / item['name']).exists() for item in complete))


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.limits = dict(workload.DEFAULT_LIMITS, timeout_seconds=.3)

    def test_timeout_terminates_descendants(self):
        pidfile = self.root / 'child.pid'
        script = ('import subprocess,time,pathlib; '
                  'p=subprocess.Popen(["sleep","30"]); '
                  f'pathlib.Path({str(pidfile)!r}).write_text(str(p.pid)); time.sleep(30)')
        with self.assertRaisesRegex(process_guard.StageFailure, 'wall_time_limit'):
            process_guard.run([sys.executable, '-c', script], self.root, 'timeout', self.limits)
        pid = int(pidfile.read_text())
        self.assertFalse(Path(f'/proc/{pid}').exists())
        self.assertEqual(artifacts.read_json(self.root / 'timeout-outcome.json')['failure'], 'wall_time_limit')

    def test_detached_child_surviving_leader_is_failure_and_is_killed(self):
        pidfile = self.root / 'child.pid'
        script = ('import subprocess,pathlib; '
                  'p=subprocess.Popen(["sleep","30"],start_new_session=True); '
                  f'pathlib.Path({str(pidfile)!r}).write_text(str(p.pid))')
        with self.assertRaisesRegex(process_guard.StageFailure, 'descendants_survived_leader'):
            process_guard.run([sys.executable, '-c', script], self.root, 'orphan', self.limits)
        self.assertFalse(Path(f'/proc/{int(pidfile.read_text())}').exists())

    def test_detached_child_born_after_last_resource_snapshot_is_killed(self):
        release, pidfile, ready = (self.root / name for name in ('release', 'late.pid', 'ready'))
        script = ('import subprocess,pathlib,time; '
                  f'pathlib.Path({str(ready)!r}).touch(); '
                  f'release=pathlib.Path({str(release)!r}); '
                  'exec("while not release.exists(): time.sleep(.005)"); '
                  'p=subprocess.Popen(["sleep","30"],start_new_session=True); '
                  f'pathlib.Path({str(pidfile)!r}).write_text(str(p.pid)); time.sleep(30)')
        original = process_guard.tree

        def late_snapshot(root, known):
            deadline = time.monotonic() + 3
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(.005)
            self.assertTrue(ready.exists())
            captured, live, pss = original(root, known)
            release.touch()
            while not pidfile.exists() and time.monotonic() < deadline:
                time.sleep(.005)
            self.assertTrue(pidfile.exists())
            # Force termination using the snapshot from before the detached child existed.
            return captured, live, 7000 * artifacts.MIB

        with patch.object(process_guard, 'tree', side_effect=late_snapshot):
            with self.assertRaisesRegex(process_guard.StageFailure, 'process_tree_pss_limit'):
                process_guard.run([sys.executable, '-c', script], self.root, 'late', self.limits)
        self.assertFalse(Path(f'/proc/{int(pidfile.read_text())}').exists())

    def test_output_size_is_enforced(self):
        self.limits['output_mib'] = 1
        output = self.root / 'large.bin'
        script = f'import pathlib,time; pathlib.Path({str(output)!r}).write_bytes(b"x"*2000000); time.sleep(10)'
        with self.assertRaisesRegex(process_guard.StageFailure, 'output_size_limit'):
            process_guard.run([sys.executable, '-c', script], self.root, 'large', self.limits, [output])

    def test_sp1_sparse_shared_memory_is_not_subject_to_artifact_fsize_limit(self):
        script = ('import os; fd=os.memfd_create("sp1-sparse-memory"); '
                  'os.ftruncate(fd, 4*1024**3); os.close(fd); print("sparse allocation completed")')
        process_guard.run([sys.executable, '-c', script], self.root, 'sp1', self.limits)
        record = artifacts.read_json(self.root / 'sp1-outcome.json')
        self.assertEqual(record['exit_code'], 0)
        self.assertFalse(record['kernel_file_size_cap'])
        self.assertIn(b'sparse allocation completed', (self.root / 'sp1.log').read_bytes())

    def test_sp1_log_cap_is_strict_without_kernel_fsize_limit(self):
        self.limits.update(log_mib=1, timeout_seconds=5)
        script = 'import os; [os.write(1,b"x"*65536) for _ in range(64)]'
        with self.assertRaisesRegex(process_guard.StageFailure, 'log_size_limit'):
            process_guard.run([sys.executable, '-c', script], self.root, 'sp1', self.limits)
        self.assertEqual((self.root / 'sp1.log').stat().st_size, artifacts.MIB)

    def test_forced_stop_removes_only_full_receipted_owned_resources(self):
        resource_path = self.root / 'sp1-resources.json'
        unknown = self.root / 'unrelated-shared-object'
        unknown.write_bytes(b'preserve')
        script = f'''import os,json,pathlib,time
root=pathlib.Path({str(self.root)!r})
sid="sp1_AAAAAAAAAAAAAAAAAAAAAA=="
records=[]
for name in (sid+'_m',sid+'_t','sem.'+sid+'_t_filled','sem.'+sid+'_t_empty'):
    path=root/name
    path.write_bytes(b'owned pages')
    info=path.stat()
    records.append({{'name':name,'device':info.st_dev,'inode':info.st_ino,'uid':info.st_uid}})
pathlib.Path({str(resource_path)!r}).write_text(json.dumps({{"schema_version":1,"stage":"sp1_owned_shared_memory","producer_pid":os.getpid(),"resources":records}}))
time.sleep(30)
'''
        with patch.object(process_guard, 'cleanup_shared_memory',
                          side_effect=lambda path, pid: shared_memory.cleanup(path, pid, self.root)):
            with self.assertRaisesRegex(process_guard.StageFailure, 'wall_time_limit'):
                process_guard.run([sys.executable, '-c', script], self.root, 'sp1', self.limits,
                                  resource_receipt=resource_path)
        outcome = artifacts.read_json(self.root / 'sp1-outcome.json')
        self.assertEqual(outcome['shared_memory_cleanup']['status'], 'verified')
        self.assertEqual([obj['status'] for obj in outcome['shared_memory_cleanup']['objects']], ['unlinked'] * 4)
        self.assertEqual(unknown.read_bytes(), b'preserve')

    def test_fast_exit_retains_final_stdout_and_stderr_after_resource_sample(self):
        original = process_guard.tree
        for code in (0, 4):
            calls = []

            def delayed_snapshot(root, known):
                result = original(root, known)
                if not calls:
                    calls.append(True)
                    time.sleep(.2)
                return result

            script = ('import sys,time; time.sleep(.05); print("FINAL_STDOUT"); '
                      f'print("FINAL_ERROR_EXPLANATION",file=sys.stderr); sys.exit({code})')
            stage = f'fast-{code}'
            with patch.object(process_guard, 'tree', side_effect=delayed_snapshot):
                if code:
                    with self.assertRaisesRegex(process_guard.StageFailure, 'nonzero_exit'):
                        process_guard.run([sys.executable, '-c', script], self.root, stage, self.limits)
                else:
                    process_guard.run([sys.executable, '-c', script], self.root, stage, self.limits)
            self.assertIn(b'FINAL_STDOUT', (self.root / f'{stage}.log').read_bytes())
            self.assertIn(b'FINAL_ERROR_EXPLANATION', (self.root / f'{stage}.log').read_bytes())

    def test_child_born_during_last_sample_cannot_escape_success_path(self):
        release, pidfile = self.root / 'release-success', self.root / 'success-child.pid'
        script = ('import subprocess,pathlib,time; '
                  f'release=pathlib.Path({str(release)!r}); '
                  'exec("while not release.exists(): time.sleep(.005)"); '
                  'p=subprocess.Popen(["sleep","30"],start_new_session=True); '
                  f'pathlib.Path({str(pidfile)!r}).write_text(str(p.pid))')
        original = process_guard.tree
        calls = []

        def delayed_snapshot(root, known):
            result = original(root, known)
            if not calls:
                calls.append(True)
                release.touch()
                time.sleep(.2)
            return result

        with patch.object(process_guard, 'tree', side_effect=delayed_snapshot):
            with self.assertRaisesRegex(process_guard.StageFailure, 'descendants_survived_leader'):
                process_guard.run([sys.executable, '-c', script], self.root, 'last-sample', self.limits)
        self.assertFalse(Path(f'/proc/{int(pidfile.read_text())}').exists())

    def test_pss_failure_and_sigterm_cleanup(self):
        with patch.object(process_guard, 'tree', return_value=({}, set(), 7000 * artifacts.MIB)):
            with self.assertRaisesRegex(process_guard.StageFailure, 'process_tree_pss_limit'):
                process_guard.run([sys.executable, '-c', 'import time; time.sleep(10)'],
                                  self.root, 'memory', self.limits)
        pidfile = self.root / 'cancel.pid'
        script = self.root / 'supervisor.py'
        script.write_text('import sys\n' +
            f'sys.path.insert(0, {str(Path(workload.__file__).parent)!r})\n' +
            'from pathlib import Path\nimport process_guard,workload\n' +
            f'root=Path({str(self.root)!r})\n' +
            f'code="import os,time,pathlib; pathlib.Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(30)"\n' +
            'process_guard.run([sys.executable,"-c",code],root,"cancel",dict(workload.DEFAULT_LIMITS))\n')
        parent = subprocess.Popen([sys.executable, str(script)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 3
            while not pidfile.exists() and time.monotonic() < deadline:
                time.sleep(.02)
            self.assertTrue(pidfile.exists())
            parent.send_signal(signal.SIGTERM)
            self.assertNotEqual(parent.wait(timeout=5), 0)
            self.assertFalse(Path(f'/proc/{int(pidfile.read_text())}').exists())
            self.assertIn('InterruptedError', artifacts.read_json(self.root / 'cancel-outcome.json')['failure'])
        finally:
            if parent.poll() is None:
                parent.kill()
                parent.wait()


if __name__ == '__main__':
    unittest.main()
