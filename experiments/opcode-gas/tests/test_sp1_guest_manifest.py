import pathlib
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]


class Sp1GuestManifestTests(unittest.TestCase):
    def test_sp1_guest_manifest_declares_opcode_lab_binary(self):
        manifest = tomllib.loads((ROOT / "guests" / "sp1" / "Cargo.toml").read_text())
        bins = manifest.get("bin", [])

        self.assertIn(
            {"name": "sp1-opcode-lab", "path": "src/opcode_lab.rs"},
            bins,
        )
        self.assertTrue((ROOT / "guests" / "sp1" / "src" / "opcode_lab.rs").exists())

    def test_sp1_guest_manifest_declares_precompile_lab_binary(self):
        manifest = tomllib.loads((ROOT / "guests" / "sp1" / "Cargo.toml").read_text())
        bins = manifest.get("bin", [])

        self.assertIn(
            {"name": "sp1-precompile-lab", "path": "src/precompile_lab.rs"},
            bins,
        )
        self.assertTrue((ROOT / "guests" / "sp1" / "src" / "precompile_lab.rs").exists())

    def test_sp1_guest_manifest_declares_revm_opcode_lab_binary(self):
        manifest = tomllib.loads((ROOT / "guests" / "sp1" / "Cargo.toml").read_text())
        bins = manifest.get("bin", [])

        self.assertIn(
            {"name": "sp1-revm-opcode-lab", "path": "src/revm_opcode_lab.rs"},
            bins,
        )
        self.assertTrue((ROOT / "guests" / "sp1" / "src" / "revm_opcode_lab.rs").exists())
