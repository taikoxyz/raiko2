import pathlib
import sys
import unittest
from decimal import Decimal
from fractions import Fraction


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
from calibration_model import RelationEquation, derive_affine_opcode_model, exact_rank


class CalibrationModelTests(unittest.TestCase):
    def test_exact_rank_and_affine_reconstruction(self):
        equations = (
            RelationEquation(
                "add-pop", {"ADD": Fraction(3), "POP": Fraction(-2)}, Decimal("50")
            ),
            RelationEquation(
                "mul-pop", {"MUL": Fraction(5), "POP": Fraction(-2)}, Decimal("170")
            ),
        )

        model = derive_affine_opcode_model(
            opcode_keys=("ADD", "MUL", "POP"),
            equations=equations,
            anchor_keys=("POP",),
        )

        self.assertEqual(model.rank, 2)
        self.assertEqual(model.nullity, 1)
        self.assertEqual(
            model.reconstruct_multipliers({"POP": Decimal("20")}),
            {"ADD": Decimal("30"), "MUL": Decimal("42"), "POP": Decimal("20")},
        )

    def test_rejects_wrong_anchor_basis(self):
        equations = (
            RelationEquation("a-b", {"A": Fraction(1), "B": Fraction(-1)}, Decimal("1")),
        )

        with self.assertRaisesRegex(
            ValueError, "anchor columns do not leave a full-rank system"
        ):
            derive_affine_opcode_model(("A", "B"), equations, ())

    def test_solves_full_rank_system_without_anchors(self):
        model = derive_affine_opcode_model(
            ("A",),
            (RelationEquation("a", {"A": Fraction(2)}, Decimal("10")),),
            (),
        )

        self.assertEqual(model.nullity, 0)
        self.assertEqual(model.reconstruct_multipliers({}), {"A": Decimal("5")})

    def test_rejects_duplicate_opcode_anchor_and_relation_identities(self):
        equation = RelationEquation("same", {"A": Fraction(1)}, Decimal("1"))

        with self.assertRaisesRegex(ValueError, "duplicate opcode key"):
            derive_affine_opcode_model(("A", "A"), (equation,), ())
        with self.assertRaisesRegex(ValueError, "duplicate anchor key"):
            derive_affine_opcode_model(("A", "B"), (equation,), ("B", "B"))
        with self.assertRaisesRegex(ValueError, "duplicate relation ID"):
            derive_affine_opcode_model(("A",), (equation, equation), ())

    def test_rejects_empty_or_unknown_coefficient_maps(self):
        with self.assertRaisesRegex(ValueError, "empty coefficient map"):
            derive_affine_opcode_model(
                ("A",), (RelationEquation("empty", {}, Decimal("1")),), ()
            )
        with self.assertRaisesRegex(ValueError, "unknown opcode key"):
            derive_affine_opcode_model(
                ("A",),
                (RelationEquation("unknown", {"B": Fraction(1)}, Decimal("1")),),
                (),
            )

    def test_rejects_non_finite_relation_slopes(self):
        for slope in (Decimal("NaN"), Decimal("Infinity"), Decimal("-Infinity")):
            with self.subTest(slope=slope), self.assertRaisesRegex(
                ValueError, "relation slope must be finite"
            ):
                derive_affine_opcode_model(
                    ("A",), (RelationEquation("bad", {"A": Fraction(1)}, slope),), ()
                )

    def test_rejects_rank_anchor_count_and_non_square_failures(self):
        dependent = (
            RelationEquation("first", {"A": Fraction(1), "B": Fraction(1)}, Decimal("1")),
            RelationEquation("second", {"A": Fraction(2), "B": Fraction(2)}, Decimal("2")),
        )
        with self.assertRaisesRegex(ValueError, "relation matrix rank"):
            derive_affine_opcode_model(("A", "B"), dependent, ())

        equation = RelationEquation("a", {"A": Fraction(1)}, Decimal("1"))
        with self.assertRaisesRegex(ValueError, "anchor count does not equal nullity"):
            derive_affine_opcode_model(("A", "B"), (equation,), ("A", "B"))
        with self.assertRaisesRegex(ValueError, "non-anchor system must be square"):
            derive_affine_opcode_model(("A", "B", "C"), (equation,), ("C",))

    def test_reconstruction_requires_exact_anchor_values(self):
        model = derive_affine_opcode_model(
            ("A", "B"),
            (RelationEquation("a-b", {"A": Fraction(1), "B": Fraction(-1)}, Decimal("1")),),
            ("B",),
        )

        with self.assertRaisesRegex(ValueError, "missing anchor values"):
            model.reconstruct_multipliers({})
        with self.assertRaisesRegex(ValueError, "unexpected anchor values"):
            model.reconstruct_multipliers({"B": Decimal("1"), "C": Decimal("2")})

    def test_current_matched_controls_have_rank_98_with_natural_anchors(self):
        cases = tuple(
            opcode_gas.CaseSpec(
                name=f"opcode:0x{opcode:02x}",
                opcode=opcode,
                scenario=scenario,
                template=template,
                target_raw_gas=raw_gas,
            )
            for opcode, (scenario, template, raw_gas) in sorted(
                opcode_gas.PURE_OPCODE_DEFAULTS.items()
            )
        )
        opcode_keys = tuple(case.name for case in cases)
        equations = []
        coefficient_rows = []
        for case in cases:
            spec = opcode_gas.matched_control_spec(case)
            target_raw_gas_by_key = {case.name: case.target_raw_gas}
            if spec.compound:
                control_raw_gas_by_key = {
                    f"opcode:0x{opcode:02x}": opcode_gas.PURE_OPCODE_DEFAULTS[opcode][2]
                    * count
                    for opcode, count in spec.reference_opcode_counts
                }
                self.assertEqual(
                    sum(control_raw_gas_by_key.values()), spec.reference_raw_gas_total
                )
            else:
                control_raw_gas_by_key = {
                    f"opcode:0x{spec.reference_opcode:02x}": spec.reference_raw_gas
                }

            coefficients = {
                opcode_key: Fraction(
                    target_raw_gas_by_key.get(opcode_key, 0)
                    - control_raw_gas_by_key.get(opcode_key, 0)
                )
                for opcode_key in target_raw_gas_by_key.keys()
                | control_raw_gas_by_key.keys()
            }
            coefficients = {
                opcode_key: coefficient
                for opcode_key, coefficient in coefficients.items()
                if coefficient
            }
            if coefficients:
                equations.append(RelationEquation(case.name, coefficients, Decimal("0")))
                coefficient_rows.append(
                    [coefficients.get(opcode_key, Fraction(0)) for opcode_key in opcode_keys]
                )

        model = derive_affine_opcode_model(
            opcode_keys=opcode_keys,
            equations=tuple(equations),
            anchor_keys=("opcode:0x50", "opcode:0x5f", "opcode:0x80", "opcode:0x90"),
        )

        self.assertEqual(len(opcode_keys), 102)
        self.assertEqual(len(equations), 98)
        self.assertEqual(exact_rank(coefficient_rows), 98)
        self.assertEqual(
            model.anchor_keys,
            ("opcode:0x50", "opcode:0x5f", "opcode:0x80", "opcode:0x90"),
        )


if __name__ == "__main__":
    unittest.main()
