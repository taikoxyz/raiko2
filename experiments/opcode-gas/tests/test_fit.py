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


if __name__ == "__main__":
    unittest.main()
