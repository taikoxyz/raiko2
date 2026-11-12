"""A declared benchmark program is bound to exact ELF and source bytes."""
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch


class ProgramManifestTests(unittest.TestCase):
    def load(self):
        source = Path(__file__).resolve().parents[1] / 'record_workload_program.py'
        self.assertTrue(source.is_file(), 'program provenance recorder is missing')
        spec = importlib.util.spec_from_file_location('record_workload_program', source)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_embedded_chain_configuration_is_part_of_source_snapshot(self):
        recorder = self.load()
        root = Path(__file__).resolve().parents[4]
        self.assertTrue(root / 'config/chain_spec_list_default.json' in recorder.source_paths(root),
                        'embedded default chain configuration is missing from the snapshot')

    def test_record_binds_profile_sdk_elf_and_sources_without_paths(self):
        recorder = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            elf = root / 'program.elf'
            data = bytearray(64)
            data[:7] = b'\x7fELF\x02\x01\x01'
            struct.pack_into('<HHI', data, 16, 2, 243, 1)
            elf.write_bytes(data)
            source = root / 'source.rs'
            source.write_text('fixed source')
            output = root / 'manifest.json'
            with patch.object(recorder, 'source_paths', return_value=[source]), \
                    patch.object(recorder, 'source_revision', return_value='a' * 40):
                recorder.record(root, elf, output)
                manifest = json.loads(output.read_text())
                self.assertEqual(manifest['execution_profile'], 'evm-gas-only')
                self.assertEqual(manifest['sp1_sdk_version'], '6.3.0')
                self.assertEqual(manifest['elf_sha256'], recorder.digest(data))
                self.assertEqual(manifest['source_sha256'], {'source.rs': recorder.digest(source.read_bytes())})
                self.assertNotIn(str(root), output.read_text())
                with self.assertRaises(FileExistsError):
                    recorder.record(root, elf, output)

    def test_invalid_program_creates_no_manifest(self):
        recorder = self.load()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            elf = root / 'program.elf'
            elf.write_bytes(b'not an ELF')
            output = root / 'manifest.json'
            with self.assertRaises(ValueError):
                recorder.record(root, elf, output)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
