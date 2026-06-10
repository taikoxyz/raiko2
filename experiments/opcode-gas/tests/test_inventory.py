import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class InventoryTests(unittest.TestCase):
    def test_uzen_tables_include_representative_entries(self):
        self.assertEqual(opcode_gas.UZEN_OPCODE_MULTIPLIERS[0x09], 152)
        self.assertEqual(opcode_gas.UZEN_OPCODE_MULTIPLIERS[0x54], 5)
        self.assertEqual(opcode_gas.UZEN_OPCODE_MULTIPLIERS[0xF1], 25)
        self.assertEqual(opcode_gas.UZEN_OPCODE_MULTIPLIERS[0xFE], 0)

        self.assertEqual(opcode_gas.UZEN_PRECOMPILE_MULTIPLIERS[0x01], 81)
        self.assertEqual(opcode_gas.UZEN_PRECOMPILE_MULTIPLIERS[0x05], 1363)
        self.assertEqual(opcode_gas.UZEN_PRECOMPILE_MULTIPLIERS[0x13], 112)

    def test_inventory_marks_manifest_cases_as_measured(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml"
        )

        rows = opcode_gas.build_inventory(manifest)
        by_key = {(row.kind, row.identifier): row for row in rows}

        self.assertEqual(by_key[("opcode", "0x01")].status, "measured")
        self.assertEqual(by_key[("opcode", "0x09")].status, "measured")
        self.assertEqual(by_key[("opcode", "0x61")].status, "measured")
        self.assertEqual(by_key[("opcode", "0x54")].status, "needs_state_or_revm")
        self.assertEqual(by_key[("opcode", "0xf1")].status, "needs_spawn_wrapper")
        self.assertEqual(by_key[("precompile", "0x04")].status, "measured")
        self.assertEqual(by_key[("precompile", "0x05")].status, "measured")

        self.assertFalse([row for row in rows if row.status == "planned_pure_opcode"])
        self.assertFalse([row for row in rows if row.status == "needs_precompile_body"])

    def test_inventory_rows_are_all_classified(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml"
        )

        rows = opcode_gas.build_inventory(manifest)

        self.assertGreaterEqual(len(rows), 120)
        self.assertFalse([row for row in rows if not row.status])


if __name__ == "__main__":
    unittest.main()
