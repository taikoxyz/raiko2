import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class FitTests(unittest.TestCase):
    def test_fit_linear_slope_from_raw_runs(self):
        runs = [
            {"case": "add", "target_count": 0, "target_raw_gas": 3, "prover_gas": 100},
            {"case": "add", "target_count": 1, "target_raw_gas": 3, "prover_gas": 130},
            {"case": "add", "target_count": 2, "target_raw_gas": 3, "prover_gas": 160},
        ]

        fit = opcode_gas.fit_case(runs)

        self.assertEqual(fit.slope_per_operation, 30)
        self.assertEqual(fit.slope_per_raw_gas, 10)
        self.assertEqual(fit.r2, 1)

    def test_fit_uses_selected_workload_metric(self):
        runs = [
            {
                "case": "add",
                "target_count": 0,
                "target_raw_gas": 3,
                "prover_gas": 100,
                "risc0_padded_cycles": 1024,
            },
            {
                "case": "add",
                "target_count": 1,
                "target_raw_gas": 3,
                "prover_gas": 130,
                "risc0_padded_cycles": 1124,
            },
            {
                "case": "add",
                "target_count": 2,
                "target_raw_gas": 3,
                "prover_gas": 160,
                "risc0_padded_cycles": 1224,
            },
        ]

        fit = opcode_gas.fit_case(runs, metric="risc0_padded_cycles")

        self.assertEqual(fit.metric, "risc0_padded_cycles")
        self.assertEqual(fit.slope_per_operation, 100)
        self.assertAlmostEqual(fit.slope_per_raw_gas, 100 / 3)
        self.assertEqual(fit.r2, 1)

    def test_damage_result_reports_eth_only_and_zkgas_limited_damage(self):
        damage = opcode_gas.compute_damage_result(
            case="add",
            kind="opcode",
            eth_gas_per_unit=3,
            measured_workload_per_unit=90.0,
            zkgas_multiplier=12,
            eth_gas_limit=30,
            zk_gas_limit=100,
        )

        self.assertEqual(damage.eth_only_units, 10)
        self.assertEqual(damage.eth_only_damage, 900)
        self.assertEqual(damage.zkgas_per_unit, 36)
        self.assertEqual(damage.candidate_units, 2)
        self.assertEqual(damage.candidate_damage, 180)
        self.assertAlmostEqual(damage.attack_reduction, 0.8)
        self.assertEqual(damage.binding_resource, "zkgas")

    def test_damage_result_reports_eth_binding_when_zkgas_has_headroom(self):
        damage = opcode_gas.compute_damage_result(
            case="add",
            kind="opcode",
            eth_gas_per_unit=3,
            measured_workload_per_unit=90.0,
            zkgas_multiplier=12,
            eth_gas_limit=30,
            zk_gas_limit=1000,
        )

        self.assertEqual(damage.candidate_units, 10)
        self.assertEqual(damage.candidate_damage, 900)
        self.assertEqual(damage.attack_reduction, 0)
        self.assertEqual(damage.binding_resource, "eth")

    def test_damage_markdown_explains_metrics_and_reports_r2(self):
        damage = opcode_gas.compute_damage_result(
            case="add",
            kind="opcode",
            eth_gas_per_unit=3,
            measured_workload_per_unit=90.0,
            zkgas_multiplier=12,
            eth_gas_limit=30,
            zk_gas_limit=100,
            r2=0.98,
        )

        with tempfile.TemporaryDirectory() as tmpdir:
            path = pathlib.Path(tmpdir) / "damage.md"
            opcode_gas.write_damage_markdown_report(
                path,
                results=[damage],
                eth_gas_limit=30,
                zk_gas_limit=100,
            )

            report = path.read_text()

        self.assertIn("## Metric Meaning", report)
        self.assertIn("R2", report)
        self.assertIn("`binding_resource = zkgas`", report)
        self.assertIn("| add | opcode | 3 | 90 | 30 | 0.98 |", report)
        self.assertIn("| add | 12 | 36 | 2 | 180 | 80.00% | zkgas | 0.98 |", report)

    def test_current_uzen_multiplier_reads_injected_schedule(self):
        schedule = opcode_gas.UnzenSchedule(
            opcode_multipliers={0x03: 7},
            precompile_multipliers={0x04: 9},
        )
        opcode_case = opcode_gas.CaseSpec(
            name="sub",
            opcode=0x03,
            scenario="stack",
            template="stack_binary",
            target_raw_gas=3,
        )
        precompile_case = opcode_gas.CaseSpec(
            name="identity",
            scenario="precompile",
            template="precompile_fixed_32",
            target_raw_gas=18,
            kind="precompile",
            address=0x04,
        )

        self.assertEqual(opcode_gas.current_uzen_multiplier(opcode_case, schedule), 7)
        self.assertEqual(opcode_gas.current_uzen_multiplier(precompile_case, schedule), 9)


if __name__ == "__main__":
    unittest.main()
