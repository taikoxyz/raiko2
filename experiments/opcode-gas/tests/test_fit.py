import pathlib
import sys
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


if __name__ == "__main__":
    unittest.main()
