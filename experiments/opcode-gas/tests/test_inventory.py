import os
import pathlib
import shutil
import sys
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


def fixture_schedule():
    return opcode_gas.UnzenSchedule(
        opcode_multipliers={opcode: 1 for opcode in opcode_gas.UZEN_OPCODE_NAMES},
        precompile_multipliers={address: 1 for address in opcode_gas.UZEN_PRECOMPILE_NAMES},
    )


class InventoryTests(unittest.TestCase):
    def test_load_uzen_schedule_invokes_xtask_exporter(self):
        completed = mock.Mock(
            stdout=opcode_gas.json.dumps(
                {
                    "opcodes": [{"opcode": "0x01", "multiplier": 7}],
                    "precompiles": [
                        {
                            "address": "0x0000000000000000000000000000000000000100",
                            "multiplier": 9,
                        }
                    ],
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
        with self.assertRaises(TypeError):
            schedule.opcode_multipliers[0x02] = 11

    def test_load_uzen_schedule_reports_missing_cargo(self):
        with mock.patch.object(
            opcode_gas.subprocess,
            "run",
            side_effect=FileNotFoundError("cargo executable not found"),
        ):
            with self.assertRaisesRegex(RuntimeError, "cargo executable not found"):
                opcode_gas.load_current_uzen_schedule()

    def test_load_uzen_schedule_reports_empty_or_invalid_json(self):
        for output, expected_detail in (("", "empty output"), ("not json", "not json")):
            with self.subTest(output=output):
                with mock.patch.object(
                    opcode_gas.subprocess,
                    "run",
                    return_value=mock.Mock(stdout=output),
                ):
                    with self.assertRaisesRegex(
                        RuntimeError,
                        f"valid JSON.*{expected_detail}",
                    ):
                        opcode_gas.load_current_uzen_schedule()

    def test_current_uzen_schedule_is_cached(self):
        self.addCleanup(opcode_gas.current_uzen_schedule.cache_clear)
        opcode_gas.current_uzen_schedule.cache_clear()
        schedule = fixture_schedule()
        with mock.patch.object(
            opcode_gas,
            "load_current_uzen_schedule",
            return_value=schedule,
        ) as load:
            first = opcode_gas.current_uzen_schedule()
            second = opcode_gas.current_uzen_schedule()

        self.assertIs(first, second)
        load.assert_called_once_with()

    @unittest.skipUnless(
        os.environ.get("RAIKO2_TEST_UNZEN_SCHEDULE_ROUNDTRIP") == "1",
        "set RAIKO2_TEST_UNZEN_SCHEDULE_ROUNDTRIP=1 to run the Rust exporter",
    )
    @unittest.skipUnless(shutil.which("cargo"), "cargo is not available")
    def test_rust_exporter_round_trips_through_python_loader(self):
        schedule = opcode_gas.load_current_uzen_schedule()

        self.assertTrue(schedule.opcode_multipliers)
        self.assertTrue(schedule.precompile_multipliers)
        self.assertIn(0x1E, schedule.opcode_multipliers)
        self.assertIn(0x100, schedule.precompile_multipliers)

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
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml",
            schedule=fixture_schedule(),
        )

        rows = opcode_gas.build_inventory(manifest, schedule=fixture_schedule())
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
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml",
            schedule=fixture_schedule(),
        )

        rows = opcode_gas.build_inventory(manifest, schedule=fixture_schedule())

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
