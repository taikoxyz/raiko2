import pathlib
import sys
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class InventoryTests(unittest.TestCase):
    def test_load_uzen_schedule_invokes_xtask_exporter(self):
        completed = mock.Mock(
            stdout=opcode_gas.json.dumps(
                {
                    "opcodes": [{"opcode": "0x01", "multiplier": 7}],
                    "precompiles": [{"address": "0x0100", "multiplier": 9}],
                }
            )
        )

        with mock.patch.object(opcode_gas.subprocess, "run", return_value=completed) as run:
            schedule = opcode_gas.load_current_uzen_schedule()

        run.assert_called_once_with(
            [
                "cargo",
                "run",
                "--quiet",
                "--locked",
                "-p",
                "xtask",
                "--no-default-features",
                "--",
                "export-unzen-zk-gas-schedule",
            ],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertEqual(schedule.opcode_multipliers, {0x01: 7})
        self.assertEqual(schedule.precompile_multipliers, {0x100: 9})

    def test_load_uzen_schedule_surfaces_exporter_stderr(self):
        failure = opcode_gas.subprocess.CalledProcessError(
            returncode=101,
            cmd=["cargo", "run"],
            stderr="error: failed to compile xtask",
        )

        with mock.patch.object(opcode_gas.subprocess, "run", side_effect=failure):
            with self.assertRaisesRegex(
                RuntimeError,
                "failed to compile xtask",
            ) as raised:
                opcode_gas.load_current_uzen_schedule()

        self.assertIs(raised.exception.__cause__, failure)

    def test_bls12_precompile_addresses_match_eip2537(self):
        self.assertEqual(
            {
                address: name
                for address, name in opcode_gas.UZEN_PRECOMPILE_NAMES.items()
                if name.startswith("bls12_")
            },
            {
                0x0B: "bls12_g1add",
                0x0C: "bls12_g1msm",
                0x0D: "bls12_g2add",
                0x0E: "bls12_g2msm",
                0x0F: "bls12_pairing",
                0x10: "bls12_map_fp_to_g1",
                0x11: "bls12_map_fp2_to_g2",
            },
        )

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

    def test_inventory_classifies_active_entries_without_experiment_templates(self):
        schedule = opcode_gas.UnzenSchedule(
            opcode_multipliers={0x01: 7, 0x1E: 9},
            precompile_multipliers={0x04: 11, 0x100: 13},
        )
        manifest = opcode_gas.Manifest(
            name="inventory-test",
            backend="sp1",
            variants=[],
            cases=[
                opcode_gas.CaseSpec(
                    name="add",
                    scenario="arithmetic",
                    template="stack_binary",
                    target_raw_gas=3,
                    opcode=0x01,
                ),
                opcode_gas.CaseSpec(
                    name="identity",
                    scenario="precompile",
                    template="precompile_fixed_32",
                    target_raw_gas=18,
                    kind="precompile",
                    address=0x04,
                ),
            ],
        )

        rows = opcode_gas.build_inventory(manifest, schedule=schedule)
        by_key = {(row.kind, row.identifier): row for row in rows}

        self.assertEqual(by_key[("opcode", "0x1e")].name, "clz")
        self.assertEqual(by_key[("opcode", "0x1e")].status, "unsupported_by_experiment")
        self.assertEqual(by_key[("precompile", "0x100")].name, "p256verify")
        self.assertEqual(
            by_key[("precompile", "0x100")].status,
            "unsupported_by_experiment",
        )


if __name__ == "__main__":
    unittest.main()
