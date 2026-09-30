import copy
import errno
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile
import types
import unittest
from decimal import Decimal, localcontext
from fractions import Fraction
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
OPCODE_GAS = ROOT / "experiments" / "opcode-gas"
sys.path.insert(0, str(OPCODE_GAS))

import context_production_campaign as production
import context_opcode_campaign as discovery_campaign
import context_approximation as approximation


MANIFEST = OPCODE_GAS / "manifests" / "sp1-context-production-v1.json"


class ContextDiagnosticTests(unittest.TestCase):
    def _source_result(self):
        source = json.loads(
            (
                OPCODE_GAS
                / "derivations"
                / "a41befb63e663890896ba67d"
                / "result.json"
            ).read_text()
        )
        source["rows"] = self._all_rows()
        return source

    @staticmethod
    def _report(row):
        return {
            "stage": "controlled-block",
            "mode": "execute",
            "sp1_execution_engine": "gas-estimator",
            "exit_code": 0,
            "guest_input_sha256": "0x" + row["backend_input_sha256"],
            "sp1_proposal_elf_sha256": row["sp1_proposal_elf_sha256"],
            "guest_launcher_sha256": row["guest_launcher_sha256"],
            "controlled_block": {
                "status": "accepted",
                "row_id": row["row_id"],
                "observation": {
                    "backend_input_sha256": row["backend_input_sha256"],
                },
            },
            "total_instruction_count": 5,
            "total_syscall_count": 2,
            "touched_memory_addresses": 0,
            "opcode_counts": [
                {"label": "ADD", "count": 2},
                {"label": "MUL", "count": 3},
            ],
            "syscall_counts": [
                {"label": "COMMIT", "count": 2},
            ],
        }

    @staticmethod
    def _all_rows():
        return [
            json.loads(line)
            for line in (
                OPCODE_GAS
                / "derivations"
                / "a41befb63e663890896ba67d"
                / "rows.jsonl"
            ).read_text().splitlines()
        ]

    def _selected_rows(self):
        return approximation.select_context_diagnostic_rows(self._all_rows())

    def test_diagnostic_selector_is_exact_88_row_repeat_zero_panel(self):
        rows = self._selected_rows()
        self.assertEqual(len(rows), 88)
        self.assertEqual({row["repeat_index"] for row in rows}, {0})
        self.assertEqual({row["lane"] for row in rows}, {"target", "control"})
        self.assertEqual(
            {(row["scenario"], row["count"]) for row in rows},
            set(approximation.CONTEXT_DIAGNOSTIC_PANEL),
        )

    def test_extractor_canonicalizes_sorted_counts_and_rejects_bad_metrics(self):
        report = self._report(self._selected_rows()[0])
        report["opcode_counts"] = list(reversed(report["opcode_counts"]))
        extracted = approximation.extract_sp1_diagnostics(report)
        self.assertEqual(
            extracted,
            {
                "total_instruction_count": 5,
                "total_syscall_count": 2,
                "touched_memory_addresses": 0,
                "opcode_counts": [
                    {"label": "ADD", "count": 2},
                    {"label": "MUL", "count": 3},
                ],
                "syscall_counts": [{"label": "COMMIT", "count": 2}],
            },
        )
        for field, value, error in (
            ("total_instruction_count", 5.0, "nonnegative integer"),
            ("touched_memory_addresses", -1, "nonnegative integer"),
            ("opcode_counts", [{"label": "ADD", "count": 5}], "duplicate"),
            ("opcode_counts", [{"label": "ADD", "count": 4}], "total"),
            ("syscall_counts", [{"label": "COMMIT", "count": 2.0}], "nonnegative integer"),
        ):
            mutated = copy.deepcopy(report)
            if error == "duplicate":
                mutated[field].append({"label": "ADD", "count": 0})
            else:
                mutated[field] = value
            with self.subTest(field=field, value=value), self.assertRaisesRegex(ValueError, error):
                approximation.extract_sp1_diagnostics(mutated)
        missing = copy.deepcopy(report)
        missing.pop("syscall_counts")
        with self.assertRaisesRegex(ValueError, "missing"):
            approximation.extract_sp1_diagnostics(missing)

    def test_sidecar_binds_source_identity_rows_and_reports_without_fit_inputs(self):
        rows = self._selected_rows()
        result = self._source_result()
        reports = [self._report(row) for row in rows]
        sidecar = approximation.build_context_diagnostic_sidecar(result, reports)
        self.assertEqual(sidecar["source_result_identity"], result["result_identity"])
        self.assertEqual(
            sidecar["source_result_identity_sha256"],
            result["result_identity_sha256"],
        )
        self.assertEqual(
            list(sidecar["rows"]),
            [row["row_id"] for row in rows],
        )
        self.assertTrue(
            all(
                not ({"total_instruction_count", "total_syscall_count", "touched_memory_addresses", "opcode_counts", "syscall_counts"} & set(row))
                for row in rows
            )
        )
        with self.assertRaisesRegex(ValueError, "source row"):
            approximation.build_context_diagnostic_sidecar(
                result, reports[:-1]
            )
        mismatched = copy.deepcopy(reports)
        mismatched[0]["controlled_block"]["row_id"] = "1" * 64
        with self.assertRaisesRegex(ValueError, "source row"):
            approximation.build_context_diagnostic_sidecar(result, mismatched)

    def test_sidecar_verifier_rejects_result_identity_mismatch_and_diagnostic_fit_input(self):
        rows = self._selected_rows()
        result = self._source_result()
        sidecar = approximation.build_context_diagnostic_sidecar(
            result, [self._report(row) for row in rows]
        )
        approximation.verify_context_diagnostic_sidecar(sidecar, result)
        persisted = json.loads(production.canonical_json(sidecar))
        approximation.verify_context_diagnostic_sidecar(persisted, result)
        rehashed = copy.deepcopy(sidecar)
        rehashed["source_result_identity"]["implementation_revision"] = "0" * 40
        unhashed = dict(rehashed)
        unhashed.pop("artifact_sha256")
        unhashed.pop("diagnostic_id")
        unhashed.pop("diagnostic_identity_sha256")
        rehashed["diagnostic_identity_sha256"] = production.sha256_bytes(
            production.canonical_json(unhashed)
        )
        rehashed["diagnostic_id"] = rehashed["diagnostic_identity_sha256"][:24]
        rehashed["artifact_sha256"] = production.sha256_bytes(
            production.canonical_json(
                {
                    **unhashed,
                    "diagnostic_identity_sha256": rehashed[
                        "diagnostic_identity_sha256"
                    ],
                    "diagnostic_id": rehashed["diagnostic_id"],
                }
            )
        )
        with self.assertRaisesRegex(ValueError, "strict source"):
            approximation.verify_context_diagnostic_sidecar(rehashed)
        changed_result = copy.deepcopy(result)
        changed_result["result_identity_sha256"] = "1" * 64
        with self.assertRaisesRegex(ValueError, "source result"):
            approximation.verify_context_diagnostic_sidecar(sidecar, changed_result)
        coefficient_input = {"rows": rows, "diagnostics": sidecar["rows"]}
        with self.assertRaisesRegex(ValueError, "diagnostic"):
            approximation.reject_context_diagnostics_from_coefficient_input(coefficient_input)

    def test_runner_resumes_atomic_reports_and_directory_verify_requires_all_reports(self):
        rows = self._selected_rows()
        source = self._source_result()
        source["manifest"] = production.load_production_context_manifest(MANIFEST)
        launcher_sha256 = rows[0]["guest_launcher_sha256"]
        elf_sha256 = rows[0]["sp1_proposal_elf_sha256"]

        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            launcher = root / "guest-launcher"
            launcher.write_bytes(b"launcher")
            out = root / "diagnostics"
            calls = []

            def interrupted_runner(*, row, output, **_kwargs):
                calls.append(row["row_id"])
                if len(calls) == 2:
                    raise RuntimeError("interrupted")
                report = self._report(row)
                approximation._write_create_only(
                    output, production.canonical_json(report) + b"\n"
                )
                return report

            def complete_runner(*, row, output, **_kwargs):
                report = self._report(row)
                approximation._write_create_only(
                    output, production.canonical_json(report) + b"\n"
                )
                return report

            with mock.patch.object(
                approximation,
                "load_context_diagnostic_source_result",
                return_value=source,
            ), mock.patch.object(
                approximation,
                "sha256_file",
                side_effect=lambda path: (
                    elf_sha256
                    if pathlib.Path(path).name == "sp1_shasta_proposal.elf"
                    else launcher_sha256
                ),
            ), mock.patch.object(
                approximation,
                "_run_diagnostic_report",
                side_effect=interrupted_runner,
            ), self.assertRaisesRegex(RuntimeError, "interrupted"):
                approximation.run_context_diagnostics(
                    result_directory=ROOT / "experiments/opcode-gas/derivations/a41befb63e663890896ba67d",
                    launcher=launcher,
                    out=out,
                )
            self.assertEqual(len(list((out / "reports").iterdir())), 1)
            resumed_report_path = out / "reports" / f"{rows[0]['row_id']}.json"
            resumed_report_path.write_bytes(
                json.dumps(json.loads(resumed_report_path.read_bytes()), indent=2).encode()
                + b"\n"
            )
            with mock.patch.object(
                approximation,
                "load_context_diagnostic_source_result",
                return_value=source,
            ), mock.patch.object(
                approximation,
                "sha256_file",
                side_effect=lambda path: (
                    elf_sha256
                    if pathlib.Path(path).name == "sp1_shasta_proposal.elf"
                    else launcher_sha256
                ),
            ), mock.patch.object(
                approximation,
                "_run_diagnostic_report",
                side_effect=complete_runner,
            ), self.assertRaisesRegex(ValueError, "not canonical"):
                approximation.run_context_diagnostics(
                    result_directory=ROOT / "experiments/opcode-gas/derivations/a41befb63e663890896ba67d",
                    launcher=launcher,
                    out=out,
                )
            self.assertFalse((out / "diagnostics.json").exists())
            resumed_report_path.write_bytes(
                production.canonical_json(self._report(rows[0])) + b"\n"
            )
            resumed_report_target = root / "resumed-report.json"
            resumed_report_target.write_bytes(resumed_report_path.read_bytes())
            resumed_report_path.unlink()
            resumed_report_path.symlink_to(resumed_report_target)
            with mock.patch.object(
                approximation,
                "load_context_diagnostic_source_result",
                return_value=source,
            ), mock.patch.object(
                approximation,
                "sha256_file",
                side_effect=lambda path: (
                    elf_sha256
                    if pathlib.Path(path).name == "sp1_shasta_proposal.elf"
                    else launcher_sha256
                ),
            ), mock.patch.object(
                approximation,
                "_run_diagnostic_report",
                side_effect=complete_runner,
            ), self.assertRaisesRegex(ValueError, "not a regular"):
                approximation.run_context_diagnostics(
                    result_directory=ROOT / "experiments/opcode-gas/derivations/a41befb63e663890896ba67d",
                    launcher=launcher,
                    out=out,
                )
            self.assertFalse((out / "diagnostics.json").exists())
            resumed_report_path.unlink()
            resumed_report_path.write_bytes(resumed_report_target.read_bytes())
            mismatched_report = self._report(rows[0])
            mismatched_report["controlled_block"]["row_id"] = rows[1]["row_id"]
            resumed_report_path.write_bytes(
                production.canonical_json(mismatched_report) + b"\n"
            )

            def should_not_run(**_kwargs):
                raise AssertionError("resumed report must be checked before a new launch")

            with mock.patch.object(
                approximation,
                "load_context_diagnostic_source_result",
                return_value=source,
            ), mock.patch.object(
                approximation,
                "sha256_file",
                side_effect=lambda path: (
                    elf_sha256
                    if pathlib.Path(path).name == "sp1_shasta_proposal.elf"
                    else launcher_sha256
                ),
            ), mock.patch.object(
                approximation,
                "_run_diagnostic_report",
                side_effect=should_not_run,
            ), self.assertRaisesRegex(ValueError, "source row mismatch"):
                approximation.run_context_diagnostics(
                    result_directory=ROOT / "experiments/opcode-gas/derivations/a41befb63e663890896ba67d",
                    launcher=launcher,
                    out=out,
                )
            self.assertFalse((out / "diagnostics.json").exists())
            resumed_report_path.write_bytes(
                production.canonical_json(self._report(rows[0])) + b"\n"
            )
            unrelated_temp = out / "reports" / ".important.tmp"
            unrelated_temp.write_bytes(b"do-not-delete")
            with mock.patch.object(
                approximation,
                "load_context_diagnostic_source_result",
                return_value=source,
            ), mock.patch.object(
                approximation,
                "sha256_file",
                side_effect=lambda path: (
                    elf_sha256
                    if pathlib.Path(path).name == "sp1_shasta_proposal.elf"
                    else launcher_sha256
                ),
            ), self.assertRaisesRegex(ValueError, "inventory"):
                approximation.run_context_diagnostics(
                    result_directory=ROOT / "experiments/opcode-gas/derivations/a41befb63e663890896ba67d",
                    launcher=launcher,
                    out=out,
                )
            unrelated_temp.unlink()

            with mock.patch.object(
                approximation,
                "load_context_diagnostic_source_result",
                return_value=source,
            ), mock.patch.object(
                approximation,
                "sha256_file",
                side_effect=lambda path: (
                    elf_sha256
                    if pathlib.Path(path).name == "sp1_shasta_proposal.elf"
                    else launcher_sha256
                ),
            ), mock.patch.object(
                approximation,
                "_run_diagnostic_report",
                side_effect=complete_runner,
            ):
                sidecar = approximation.run_context_diagnostics(
                    result_directory=ROOT / "experiments/opcode-gas/derivations/a41befb63e663890896ba67d",
                    launcher=launcher,
                    out=out,
                )
            self.assertEqual(len(sidecar["rows"]), 88)
            approximation.verify_context_diagnostics_path(out)
            tampered = copy.deepcopy(sidecar)
            first_row = next(iter(tampered["rows"].values()))
            first_row["diagnostics"]["touched_memory_addresses"] = 1
            unhashed = dict(tampered)
            unhashed.pop("artifact_sha256")
            unhashed.pop("diagnostic_id")
            unhashed.pop("diagnostic_identity_sha256")
            tampered["diagnostic_identity_sha256"] = production.sha256_bytes(
                production.canonical_json(unhashed)
            )
            tampered["diagnostic_id"] = tampered["diagnostic_identity_sha256"][:24]
            tampered["artifact_sha256"] = production.sha256_bytes(
                production.canonical_json(
                    {
                        **unhashed,
                        "diagnostic_identity_sha256": tampered[
                            "diagnostic_identity_sha256"
                        ],
                        "diagnostic_id": tampered["diagnostic_id"],
                    }
                )
            )
            (out / "diagnostics.json").write_bytes(
                production.canonical_json(tampered) + b"\n"
            )
            with self.assertRaisesRegex(ValueError, "diagnostics"):
                approximation.verify_context_diagnostics_path(out)
            (out / "diagnostics.json").write_bytes(
                production.canonical_json(sidecar) + b"\n"
            )
            (out / "reports" / f"{rows[0]['row_id']}.json").unlink()
            with self.assertRaisesRegex(ValueError, "inventory"):
                approximation.verify_context_diagnostics_path(out)


class ContextDeclaredApproximationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = (
            OPCODE_GAS
            / "derivations"
            / "a41befb63e663890896ba67d"
        )
        cls.manifest = production.load_production_context_manifest(
            cls.source / "campaign-manifest.json"
        )
        cls.rows = [
            json.loads(line)
            for line in (cls.source / "rows.jsonl").read_text().splitlines()
        ]
        cls.subtotal = production.load_production_v5_subtotal_model(
            cls.manifest, ROOT
        )

    @staticmethod
    def _fraction(payload):
        return Fraction(int(payload["numerator"]), int(payload["denominator"]))

    @staticmethod
    def _rewrite_scenario_delta(rows, scenario, per_event_delta):
        rewritten = copy.deepcopy(rows)
        zero = {
            row["lane"]: Decimal(row["prover_gas"])
            for row in rewritten
            if row["scenario"] == scenario
            and row["count"] == 0
            and row["repeat_index"] == 0
        }
        baseline_gap = zero["target"] - zero["control"]
        controls = {
            (row["count"], row["repeat_index"]): Decimal(row["prover_gas"])
            for row in rewritten
            if row["scenario"] == scenario and row["lane"] == "control"
        }
        for row in rewritten:
            if row["scenario"] != scenario or row["lane"] != "target" or row["count"] == 0:
                continue
            row["prover_gas"] = str(
                controls[(row["count"], row["repeat_index"])]
                + baseline_gap
                + Decimal(row["count"] * per_event_delta)
            )
        return rewritten

    def test_source_loader_binds_full_identity_and_every_file_hash(self):
        source = approximation.load_context_approximation_source_result(self.source)
        self.assertEqual(
            source["result_identity_sha256"],
            "a41befb63e663890896ba67de48a51e65de49c4b281229a37b0e271a094e1943",
        )
        self.assertEqual(
            source["source_file_sha256s"]["result.json"],
            "634783f4518151ad16aa5f92022f54cb1e5d258e85a5b16196159bfdc9c285a2",
        )
        self.assertEqual(
            set(source["source_file_sha256s"]),
            set(production.PRODUCTION_CONTEXT_RESULT_INVENTORY),
        )
        with tempfile.TemporaryDirectory() as directory:
            copied = pathlib.Path(directory) / source["result_id"]
            shutil.copytree(self.source, copied)
            envelope = json.loads((copied / "result.json").read_bytes())
            envelope["result_identity_sha256"] = (
                source["result_id"] + "1" * 40
            )
            envelope["artifact_sha256"] = production.sha256_bytes(
                production.canonical_json(
                    {key: value for key, value in envelope.items() if key != "artifact_sha256"}
                )
            )
            (copied / "result.json").chmod(0o644)
            (copied / "result.json").write_bytes(
                production.canonical_json(envelope) + b"\n"
            )
            with self.assertRaisesRegex(ValueError, "strict source"):
                approximation.load_context_approximation_source_result(copied)

    def test_builder_pairs_count_zero_adds_one_replacement_and_preserves_rejections(self):
        built = approximation.build_declared_context_approximation(
            self.manifest, self.rows, self.subtotal
        )
        self.assertEqual(
            set(built["classes"]),
            {
                "address",
                "caller",
                "callvalue:zero",
                "callvalue:nonzero",
                "calldataload:zero",
                "calldataload:partial",
                "calldataload:full",
                "calldatasize",
                "timestamp",
            },
        )
        self.assertEqual(len(built["controlled_rows"]), 327)
        address = built["scenario_costs"]["address_canonical"]
        count_one = next(
            row
            for row in built["controlled_rows"]
            if row["scenario"] == "address_canonical"
            and row["count"] == 1
            and row["repeat_index"] == 0
        )
        self.assertEqual(
            self._fraction(count_one["observed_target_control_marginal_exact"]),
            Fraction(99),
        )
        self.assertEqual(
            self._fraction(address["replacement_cost_exact"]),
            Fraction(
                Decimal(
                    "24.005379037858121176747031351368990855052366915359737824471331436422526620808534"
                )
            ),
        )
        load = built["scenario_costs"]["calldataload_full_32_offset_0"]
        self.assertEqual(
            self._fraction(load["replacement_cost_exact"]),
            Fraction(
                Decimal(
                    "160.21132743956716642801549589643853847797742842557414869147573936756164015088704"
                )
            ),
        )
        self.assertTrue(
            all(
                value["status"] == "declared_approximation"
                for value in built["classes"].values()
            )
        )
        self.assertEqual(
            built["classes"]["calldatasize"]["shape"],
            "constant_per_execution",
        )
        decisions = json.loads((self.source / "campaign-decisions.json").read_bytes())
        self.assertEqual(
            built["strict_rejections"],
            {
                key: {
                    "status": "rejected",
                    "rejection_reasons": value["rejection_reasons"],
                }
                for key, value in decisions["families"].items()
            },
        )

    def test_builder_fails_closed_on_pair_and_residual_ledger_errors(self):
        missing = self.rows[:-1]
        with self.assertRaisesRegex(ValueError, "missing.*repeat|missing.*row"):
            approximation.build_declared_context_approximation(
                self.manifest, missing, self.subtotal
            )
        duplicate = [*self.rows, copy.deepcopy(self.rows[-1])]
        with self.assertRaisesRegex(ValueError, "duplicate"):
            approximation.build_declared_context_approximation(
                self.manifest, duplicate, self.subtotal
            )
        residual = copy.deepcopy(self.rows)
        for row in residual:
            if (
                row["scenario"] == "address_canonical"
                and row["count"] == 1
                and row["lane"] == "target"
            ):
                row["actual_raw_gas_by_key"]["opcode:0x60"] += 3
                row["actual_operation_event_count_by_key"]["opcode:0x60"] += 1
        with self.assertRaisesRegex(ValueError, "residual|non-target"):
            approximation.build_declared_context_approximation(
                self.manifest, residual, self.subtotal
            )

    def test_builder_pairs_and_retains_each_exact_repeat_observation(self):
        rows = copy.deepcopy(self.rows)
        changed = next(
            row
            for row in rows
            if row["scenario"] == "address_canonical"
            and row["count"] == 1
            and row["lane"] == "target"
            and row["repeat_index"] == 1
        )
        changed["prover_gas"] = str(int(changed["prover_gas"]) + 3)
        built = approximation.build_declared_context_approximation(
            self.manifest, rows, self.subtotal
        )
        evidence = [
            row
            for row in built["controlled_rows"]
            if row["scenario"] == "address_canonical" and row["count"] == 1
        ]
        self.assertEqual([row["repeat_index"] for row in evidence], [0, 1, 2])
        self.assertEqual(
            [
                self._fraction(row["observed_target_control_marginal_exact"])
                for row in evidence
            ],
            [Fraction(99), Fraction(102), Fraction(99)],
        )
        self.assertTrue(
            all(
                isinstance(row["target_row_id"], str)
                and isinstance(row["control_row_id"], str)
                and "target_row_ids" not in row
                and "control_row_ids" not in row
                for row in evidence
            )
        )

    def test_builder_floors_negative_classes_and_chooses_conservative_sibling_maximum(self):
        rows = self._rewrite_scenario_delta(
            self.rows, "callvalue_zero", -100
        )
        rows = self._rewrite_scenario_delta(
            rows, "callvalue_zero_calldata_1", -200
        )
        built = approximation.build_declared_context_approximation(
            self.manifest, rows, self.subtotal
        )
        self.assertEqual(
            self._fraction(built["classes"]["callvalue:zero"]["cost_exact"]),
            Fraction(0),
        )
        rows = self._rewrite_scenario_delta(
            self.rows, "callvalue_zero", 10
        )
        rows = self._rewrite_scenario_delta(
            rows, "callvalue_zero_calldata_1", 20
        )
        built = approximation.build_declared_context_approximation(
            self.manifest, rows, self.subtotal
        )
        replacement = self._fraction(
            built["scenario_costs"]["callvalue_zero"]["replacement_cost_exact"]
        )
        self.assertEqual(
            self._fraction(built["classes"]["callvalue:zero"]["cost_exact"]),
            Fraction(20) + replacement,
        )

    def test_builder_rejects_discovery_coefficients_and_sp1_diagnostics(self):
        discovery = copy.deepcopy(self.rows)
        discovery[0]["body_scale"] = "1"
        with self.assertRaisesRegex(ValueError, "forbidden"):
            approximation.build_declared_context_approximation(
                self.manifest, discovery, self.subtotal
            )
        diagnostics = copy.deepcopy(self.rows)
        diagnostics[0]["total_instruction_count"] = 1
        with self.assertRaisesRegex(ValueError, "diagnostic"):
            approximation.build_declared_context_approximation(
                self.manifest, diagnostics, self.subtotal
            )

    def test_seal_is_deterministic_create_only_and_verifier_rejects_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            out = pathlib.Path(directory) / "derivations"
            with mock.patch.object(
                approximation,
                "_fsync_directory",
                wraps=approximation._fsync_directory,
            ) as fsync_directory:
                first = approximation.seal_context_approximation(
                    source_result=self.source, out_root=out
                )
            self.assertIn(
                mock.call(out),
                fsync_directory.call_args_list,
            )
            verified = approximation.verify_context_approximation_path(
                pathlib.Path(first["directory"])
            )
            self.assertEqual(verified["approximation_id"], first["approximation_id"])
            with self.assertRaisesRegex(ValueError, "already exists"):
                approximation.seal_context_approximation(
                    source_result=self.source, out_root=out
                )
            artifact = pathlib.Path(first["directory"]) / "context-approximation.json"
            self.assertEqual(stat.S_IMODE(artifact.stat().st_mode), 0o444)
            original = artifact.read_bytes()
            tampered = json.loads(original)
            tampered["classes"]["address"]["cost_exact"]["numerator"] = "0"
            artifact.chmod(0o644)
            artifact.write_bytes(production.canonical_json(tampered) + b"\n")
            with self.assertRaisesRegex(ValueError, "identity|replay|fraction"):
                approximation.verify_context_approximation_path(
                    pathlib.Path(first["directory"])
                )


class ProductionContextManifestTests(unittest.TestCase):
    def test_canonical_manifest_payload_is_deeply_independent(self):
        payload = production.canonical_production_context_manifest_payload()
        payload["execution"]["parity_row"]["count"] = 2
        regenerated = production.canonical_production_context_manifest_payload()
        self.assertEqual(regenerated["execution"]["parity_row"]["count"], 1)

    def test_parsed_manifest_and_parity_authority_are_deeply_immutable(self):
        manifest = production.load_production_context_manifest(MANIFEST)

        with self.assertRaises(TypeError):
            manifest.execution["parity_row"]["count"] = 2
        with self.assertRaises(TypeError):
            production.EXECUTION["parity_row"]["count"] = 2
        with self.assertRaises(TypeError):
            production.PARITY_ROW_CONTRACT["count"] = 2

    def test_manifest_rejects_mutated_nested_parity_contract(self):
        payload = production.canonical_production_context_manifest_payload()
        payload["execution"]["parity_row"]["count"] = 2
        with self.assertRaisesRegex(ValueError, "frozen contract"):
            production.ProductionContextManifest.from_mapping(payload)

    def test_operation_inventory_binds_trace_schedule_candidates_and_scenarios(self):
        manifest = production.load_production_context_manifest(MANIFEST)

        self.assertEqual(
            tuple(
                (
                    operation.mnemonic,
                    operation.key,
                    operation.opcode,
                    operation.production_schedule,
                    operation.trace_input_kind,
                    operation.control_opcode,
                )
                for operation in manifest.operations
            ),
            (
                ("ADDRESS", "opcode:0x30", 0x30, "UNZEN_ZK_GAS_SCHEDULE", "context_fixed", 0x5F),
                ("CALLER", "opcode:0x33", 0x33, "UNZEN_ZK_GAS_SCHEDULE", "context_fixed", 0x5F),
                ("CALLVALUE", "opcode:0x34", 0x34, "UNZEN_ZK_GAS_SCHEDULE", "context_value", 0x5F),
                ("CALLDATALOAD", "opcode:0x35", 0x35, "UNZEN_ZK_GAS_SCHEDULE", "calldata_load", 0x90),
                ("CALLDATASIZE", "opcode:0x36", 0x36, "UNZEN_ZK_GAS_SCHEDULE", "calldata_size", 0x5F),
                ("TIMESTAMP", "opcode:0x42", 0x42, "UNZEN_ZK_GAS_SCHEDULE", "context_value", 0x5F),
            ),
        )
        for operation in manifest.operations:
            self.assertEqual(
                operation.production_schedule_sha256,
                "b27c29fb5fe482de4b3e22784de0cc6c49a5f1f0160ec8a6eaa6997869864927",
            )
            self.assertEqual(operation.component_kind, "opcode")
            self.assertEqual(operation.trace_selector_ref, "opcode_raw_gas_execution")
            self.assertEqual(
                operation.transaction_scope_selector_ref,
                "non_anchor_started_transaction",
            )
            self.assertEqual(operation.pricing_basis, "raw_gas_slope")
            self.assertEqual(operation.dispatch_status, "not_applicable")
            self.assertFalse(operation.spawned)

        timestamp = manifest.operation("opcode:0x42")
        self.assertEqual(
            timestamp.required_scenario_ids,
            (
                "timestamp_post_unzen_delta_17",
                "timestamp_post_unzen_delta_86400",
            ),
        )
        self.assertEqual(
            timestamp.diagnostic_scenario_ids,
            ("timestamp_zero_unreachable",),
        )
        self.assertEqual(
            manifest.operation("opcode:0x36").candidate_ids,
            ("calldatasize_length", "calldatasize_boundary"),
        )

    def test_repeat_and_family_promotion_contracts_are_machine_readable(self):
        manifest = production.load_production_context_manifest(MANIFEST)

        self.assertEqual(
            dict(manifest.repeat_contract),
            {
                "exact_repeats": 3,
                "equality_fields": (
                    "prover_gas",
                    "public_output",
                    "backend_input_sha256",
                    "host_trace_sha256",
                ),
                "mismatch_outcome": "reject_scenario",
            },
        )
        self.assertEqual(
            dict(manifest.family_promotion_contract),
            {
                "required_evidence": "all_required_scenarios_classes_and_gates",
                "required_gates": (
                    "exact_event_matching",
                    "control_lane_contamination",
                    "repeat",
                    "signal",
                    "fit",
                    "count_holdout",
                    "extrapolation",
                    "scenario_holdout",
                ),
                "partial_application": "forbidden",
                "failed_family_status": "explicit_gap",
                "final_holdout_parameter_influence": "forbidden",
                "final_holdout_model_switching": "forbidden",
            },
        )
        with self.assertRaises(TypeError):
            manifest.repeat_contract["equality_fields"][0] = "mutated"
        with self.assertRaises(TypeError):
            manifest.family_promotion_contract["required_gates"][0] = "mutated"

    def test_manifest_freezes_inventory_splits_counts_and_repeats(self):
        manifest = production.load_production_context_manifest(MANIFEST)

        self.assertEqual(
            manifest.keys,
            (
                "opcode:0x30",
                "opcode:0x33",
                "opcode:0x34",
                "opcode:0x35",
                "opcode:0x36",
                "opcode:0x42",
            ),
        )
        self.assertEqual(manifest.fit_counts, (0, 1, 2, 4, 8, 16))
        self.assertEqual(manifest.validation_counts, (0, 32, 64))
        self.assertEqual(manifest.repeats, 3)
        self.assertEqual(
            manifest.execution["parity_row"],
            {
                "scenario": "address_canonical",
                "split": "fit",
                "count": 1,
                "lane": "target",
                "repeat_index": 0,
            },
        )
        by_split = {
            split: tuple(row.name for row in manifest.scenarios if row.split == split)
            for split in ("fit", "model_selection", "final_holdout", "unreachable")
        }
        self.assertEqual(
            by_split,
            {
                "fit": (
                    "address_canonical",
                    "caller_canonical",
                    "callvalue_zero",
                    "callvalue_nonzero_7",
                    "calldataload_empty_offset_0",
                    "calldataload_full_32_offset_0",
                    "calldataload_partial_33_offset_17",
                    "calldataload_out_of_range_4_offset_64",
                    "calldatasize_0",
                    "calldatasize_1",
                    "calldatasize_31",
                    "calldatasize_32",
                    "calldatasize_33",
                    "calldatasize_64",
                    "timestamp_post_unzen_delta_17",
                ),
                "model_selection": (
                    "calldatasize_2",
                    "calldatasize_63",
                    "calldatasize_65",
                    "calldatasize_96",
                ),
                "final_holdout": (
                    "address_alternate",
                    "caller_alternate",
                    "callvalue_zero_calldata_1",
                    "callvalue_nonzero_4294967297",
                    "calldataload_empty_offset_1",
                    "calldataload_out_of_range_96_offset_128",
                    "calldataload_partial_31_offset_30",
                    "calldataload_full_96_offset_32",
                    "calldatasize_15",
                    "calldatasize_47",
                    "calldatasize_127",
                    "calldatasize_255",
                    "timestamp_post_unzen_delta_86400",
                ),
                "unreachable": ("timestamp_zero_unreachable",),
            },
        )
        rows = production.production_context_row_specs(manifest)
        self.assertEqual(len(rows), 846)
        self.assertEqual(len({row.row_id for row in rows}), len(rows))
        self.assertEqual({row.repeat_index for row in rows}, {0, 1, 2})

        callvalue = manifest.operation("opcode:0x34")
        self.assertEqual(
            callvalue.required_scenario_ids,
            (
                "callvalue_zero",
                "callvalue_nonzero_7",
                "callvalue_zero_calldata_1",
                "callvalue_nonzero_4294967297",
            ),
        )
        calldataload = manifest.operation("opcode:0x35")
        self.assertEqual(
            calldataload.required_scenario_ids,
            (
                "calldataload_empty_offset_0",
                "calldataload_full_32_offset_0",
                "calldataload_partial_33_offset_17",
                "calldataload_out_of_range_4_offset_64",
                "calldataload_empty_offset_1",
                "calldataload_out_of_range_96_offset_128",
                "calldataload_partial_31_offset_30",
                "calldataload_full_96_offset_32",
            ),
        )

    def test_manifest_freezes_models_formulas_and_gates(self):
        manifest = production.load_production_context_manifest(MANIFEST)

        self.assertEqual(
            tuple(candidate.name for candidate in manifest.model_candidates),
            (
                "address_constant",
                "caller_constant",
                "callvalue_classes",
                "calldataload_access_classes",
                "calldatasize_length",
                "calldatasize_boundary",
                "timestamp_nonzero",
            ),
        )
        self.assertEqual(
            manifest.model_selection_order,
            ("calldatasize_length", "calldatasize_boundary"),
        )
        self.assertEqual(
            {candidate.name: candidate.terms for candidate in manifest.model_candidates},
            {
                "address_constant": ("address_constant",),
                "caller_constant": ("caller_constant",),
                "callvalue_classes": ("callvalue_zero", "callvalue_nonzero"),
                "calldataload_access_classes": ("load_zero", "load_partial", "load_full"),
                "calldatasize_length": ("beta_0", "beta_length"),
                "calldatasize_boundary": ("beta_0", "beta_words", "beta_partial"),
                "timestamp_nonzero": ("timestamp_nonzero",),
            },
        )
        self.assertEqual(
            manifest.model_candidate("calldatasize_length").formula,
            "beta_0 + beta_length * input_length",
        )
        self.assertEqual(
            manifest.model_candidate("calldatasize_boundary").formula,
            "beta_0 + beta_words * ceil(input_length / 32) + beta_partial * I(input_length mod 32 != 0)",
        )
        self.assertEqual(
            dict(manifest.quality_gates),
            {
                "signal_evaluation_count": 16,
                "signal_min_formula": "max(1000, 20 * 143)",
                "signal_min_prover_gas": 2860,
                "r2_min": "0.995",
                "relative_coefficient_stderr_max": "0.05",
                "fit_residual_signal_max": "0.02",
                "control_residual_abs_max_prover_gas": 2860,
                "control_residual_abs_max_formula": "max(1000, 20 * 143)",
                "count_holdout_ape_max": "0.10",
                "extrapolation_ape_max": "0.10",
                "final_scenario_row_ape_max": "0.10",
                "final_scenario_family_mape_max": "0.05",
                "sibling_slope_relative_difference_max": "0.05",
                "finite_nonnegative_coefficients_and_predictions": True,
            },
        )
        self.assertEqual(
            dict(manifest.fit_equations),
            {
                "target_observed": "Y_s(n) = [P_s(n) - P_s(0)] - [K_s(n) - K_s(0)]",
                "target_predicted": "Y_hat_s(n) = n * f_s(context_features)",
                "control_residual": "C_s(n) = [P_control_s(n) - P_control_s(0)] - [K_control_s(n) - K_control_s(0)]",
                "ape": "abs(predicted_increment - observed_increment) / abs(observed_increment)",
            },
        )

    def test_timestamp_zero_is_unreachable_and_all_measured_timestamps_are_post_unzen(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        zero = manifest.scenario("timestamp_zero_unreachable")
        self.assertEqual(zero.split, "unreachable")
        self.assertEqual(zero.context, {"timestamp_delta": 0})
        self.assertEqual(zero.reachability, "unreachable_under_version_identity")
        self.assertEqual(zero.counts(manifest), ())
        measured = [row for row in manifest.scenarios if row.key == "opcode:0x42" and row.split != "unreachable"]
        self.assertTrue(measured)
        self.assertTrue(all(row.context["timestamp_delta"] > 0 for row in measured))

    def test_calldata_zero_class_has_empty_and_nonempty_out_of_range_siblings(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        zero = [
            row for row in manifest.scenarios
            if row.key == "opcode:0x35" and row.model_class == "load_zero"
        ]
        self.assertEqual(
            {(row.context["input_length"], row.context["offset"]) for row in zero},
            {(0, 0), (4, 64), (0, 1), (96, 128)},
        )

    def test_sources_bind_exact_sealed_artifacts_and_execution_surfaces(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        validated = production.validate_production_context_sources(manifest, ROOT)

        self.assertEqual(set(validated), {"operation_coverage_v5", "higher_layer", "discovery"})
        self.assertEqual(
            manifest.execution,
            {
                "stage": "controlled-block",
                "proof_type": "sp1",
                "mode": "execute",
                "sp1_prover": "local",
                "sp1_execution_engine": "gas-estimator",
                "launcher_path": "target/release/guest-launcher",
                "launcher_binding": "sha256_at_clean_prepare",
                "production_elf_path": "crates/guests/elf/sp1_shasta_proposal.elf",
                "production_vk_path": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
                "guest_artifact_binding": "sha256_at_clean_prepare",
                "implementation_revision_binding": "clean_git_head_at_prepare",
                "trace_schema_source": "crates/zkgas-trace/src/reconstruct.rs",
                "trace_schema_version": 4,
                "trace_source_binding": "sha256_at_clean_prepare",
                "parity_row": {
                    "scenario": "address_canonical",
                    "split": "fit",
                    "count": 1,
                    "lane": "target",
                    "repeat_index": 0,
                },
            },
        )
        self.assertEqual(
            manifest.version_identity,
            {
                "taiko_fork": "Unzen",
                "ethereum_upgrade": "Fusaka",
                "revm_spec_id": "OSAKA",
                "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
                "primary_metric": "proverGas",
                "proving_backend": "sp1",
                "sp1_sdk_version": "6.3.0",
                "timestamp_semantics": "strictly_post_unzen_activation",
            },
        )

    def test_parser_rejects_contract_drift(self):
        mutations = []
        payload = production.canonical_production_context_manifest_payload()
        payload["unknown"] = True
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["scenarios"].append(copy.deepcopy(payload["scenarios"][0]))
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["scenarios"][0]["split"] = "final_holdout"
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["model_candidates"][0]["terms"].append("opened_after_sampling")
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["quality_gates"]["r2_min"] = 0.995
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["sources"]["operation_coverage_v5"]["path"] = str(ROOT / "coverage.json")
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["model_candidates"][0]["discovery_coefficient"] = "71.4"
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["operations"].pop()
        mutations.append(payload)

        for field, value in (
            ("key", "opcode:0x31"),
            ("opcode", 0x31),
            ("mnemonic", "BALANCE"),
            ("production_schedule", "caller_selected"),
            ("production_schedule_sha256", "0" * 64),
            ("trace_selector_ref", "untyped"),
            ("trace_input_kind", "static_raw_gas"),
            ("control_opcode", 0x50),
        ):
            payload = production.canonical_production_context_manifest_payload()
            payload["operations"][0][field] = value
            mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["operations"][0]["candidate_ids"] = []
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["operations"][0]["required_scenario_ids"] = []
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["operations"][0]["diagnostic_scenario_ids"] = [
            payload["operations"][0]["required_scenario_ids"][0]
        ]
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["repeat_contract"]["equality_fields"].remove("host_trace_sha256")
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["family_promotion_contract"]["partial_application"] = "allowed"
        mutations.append(payload)

        payload = production.canonical_production_context_manifest_payload()
        payload["family_promotion_contract"]["final_holdout_model_switching"] = "allowed"
        mutations.append(payload)

        for payload in mutations:
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                production.ProductionContextManifest.from_mapping(payload)

    def test_json_loader_rejects_binary_floats_and_duplicate_object_keys(self):
        canonical = production.canonical_production_context_manifest_payload()
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "manifest.json"
            text = json.dumps(canonical, sort_keys=True, separators=(",", ":"))
            path.write_text(text.replace('"0.995"', "0.995", 1))
            with self.assertRaisesRegex(ValueError, "binary float"):
                production.load_production_context_manifest(path)

            path.write_text('{"schema_version":1,"schema_version":1}')
            with self.assertRaisesRegex(ValueError, "duplicate JSON field"):
                production.load_production_context_manifest(path)

    def _linked_source_view(self, directory):
        root = pathlib.Path(directory)
        manifest = production.load_production_context_manifest(MANIFEST)
        relative_files = {
            manifest.sources["operation_coverage_v5"]["path"],
            manifest.sources["higher_layer"]["path"],
            manifest.sources["higher_layer"]["directory_identity_path"],
        }
        discovery_result = ROOT / manifest.sources["discovery"]["path"]
        relative_files.update(
            str(path.relative_to(ROOT)) for path in discovery_result.parent.iterdir()
        )
        for relative in relative_files:
            source = ROOT / relative
            destination = root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.hardlink_to(source)
        return root, manifest, discovery_result

    def test_source_validation_rejects_missing_discovery_sibling(self):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            root, manifest, discovery_result = self._linked_source_view(directory)
            (root / discovery_result.parent.relative_to(ROOT) / "source-identity.json").unlink()
            with self.assertRaisesRegex(ValueError, "inventory"):
                production.validate_production_context_sources(manifest, root)

    def test_source_validation_rejects_tampered_discovery_sibling(self):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            root, manifest, discovery_result = self._linked_source_view(directory)
            sibling = root / discovery_result.parent.relative_to(ROOT) / "source-identity.json"
            original = sibling.read_bytes()
            sibling.unlink()
            sibling.write_bytes(original + b" ")
            with self.assertRaisesRegex(ValueError, "hash differs"):
                production.validate_production_context_sources(manifest, root)


class ProductionContextIdentityTests(unittest.TestCase):
    def test_workload_and_row_ids_bind_all_semantic_dimensions(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        scenario = manifest.scenario("callvalue_nonzero_7")
        workload = production.production_context_workload_id(manifest, scenario, 8)
        self.assertEqual(len(workload), 64)
        self.assertEqual(
            workload,
            production.production_context_workload_id(manifest, scenario, 8),
        )
        changed = copy.deepcopy(production.canonical_production_context_manifest_payload())
        changed["scenarios"][3]["context"]["value"] = "8"
        with self.assertRaises(ValueError):
            production.ProductionContextManifest.from_mapping(changed)

        target = production.production_context_row_id(
            workload_id=workload, lane="target", repeat_index=0
        )
        self.assertEqual(len(target), 64)
        for lane, repeat in (("control", 0), ("target", 1), ("control", 2)):
            self.assertNotEqual(
                target,
                production.production_context_row_id(
                    workload_id=workload, lane=lane, repeat_index=repeat
                ),
            )
        for lane, repeat in (("other", 0), ("target", -1), ("target", 3)):
            with self.assertRaises(ValueError):
                production.production_context_row_id(
                    workload_id=workload, lane=lane, repeat_index=repeat
                )

    def test_row_specs_have_one_workload_id_per_scenario_and_count(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        rows = production.production_context_row_specs(manifest)
        grouped = {}
        for row in rows:
            grouped.setdefault((row.scenario, row.count), set()).add(row.workload_id)
        self.assertTrue(grouped)
        self.assertTrue(all(len(ids) == 1 for ids in grouped.values()))

    def test_fixture_requests_bind_every_row_to_the_structured_rust_builder(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        fixtures = production.production_context_fixture_requests(manifest)
        self.assertEqual(len(fixtures), 846)
        self.assertEqual(
            [fixture.row_id for fixture in fixtures],
            [row.row_id for row in production.production_context_row_specs(manifest)],
        )
        self.assertEqual(
            production.production_context_fixture_requests(manifest), fixtures
        )
        self.assertEqual({fixture.builder_input["program"]["lane"] for fixture in fixtures}, {"target", "control"})
        self.assertEqual({fixture.builder_input["program"]["count"] for fixture in fixtures}, {0, 1, 2, 4, 8, 16, 32, 64})
        for fixture in fixtures:
            builder_input = fixture.builder_input
            self.assertEqual(builder_input["row_id"], fixture.row_id)
            self.assertEqual(builder_input["workload_family"], "context_opcode")
            self.assertEqual(builder_input["block_count"], 1)
            self.assertEqual(builder_input["transaction_count"], 1)
            self.assertEqual(builder_input["expected_final_state_root"], "0x" + "00" * 32)
            self.assertEqual(builder_input["expected_raw_gas_by_key"], {})
            self.assertEqual(builder_input["expected_operation_event_count_by_key"], {})
            self.assertEqual(builder_input["expected_context_features"], {})
            self.assertEqual(builder_input["expected_features"], {})
            self.assertEqual(builder_input["expected_diagnostics"], {})
            program = builder_input["program"]
            self.assertEqual(program["kind"], "context_opcode_loop")
            self.assertEqual(program["workload_id"], fixture.workload_id)
            self.assertEqual(program["repeat_index"], fixture.repeat_index)
            self.assertNotIn("family", program)
            self.assertNotIn("scenario", program)

        by_scenario = {fixture.scenario: fixture for fixture in fixtures if fixture.lane == "target" and fixture.count in {0, 1} and fixture.repeat_index == 0}
        self.assertEqual(by_scenario["address_alternate"].builder_input["program"]["profile"], {"kind": "address", "address_profile": "alternate"})
        self.assertEqual(by_scenario["caller_alternate"].builder_input["program"]["profile"], {"kind": "caller", "caller_profile": "alternate"})
        self.assertEqual(by_scenario["callvalue_nonzero_4294967297"].builder_input["program"]["profile"], {"kind": "callvalue", "value": 4_294_967_297, "value_class": "nonzero", "input_length": 0})
        self.assertEqual(
            by_scenario["callvalue_zero_calldata_1"].builder_input["program"]["profile"],
            {
                "kind": "callvalue",
                "value": 0,
                "value_class": "zero",
                "input_length": 1,
            },
        )
        self.assertEqual(
            by_scenario["calldataload_empty_offset_1"].builder_input["program"]["profile"],
            {
                "kind": "calldataload",
                "input_length": 0,
                "offset": 1,
                "access_class": "zero",
            },
        )
        self.assertEqual(
            by_scenario["calldataload_out_of_range_96_offset_128"].builder_input["program"]["profile"],
            {
                "kind": "calldataload",
                "input_length": 96,
                "offset": 128,
                "access_class": "zero",
            },
        )
        self.assertEqual(by_scenario["calldataload_partial_31_offset_30"].builder_input["program"]["profile"], {"kind": "calldataload", "input_length": 31, "offset": 30, "access_class": "partial"})
        self.assertEqual(by_scenario["calldatasize_255"].builder_input["program"]["profile"], {"kind": "calldatasize", "input_length": 255})
        self.assertEqual(by_scenario["timestamp_post_unzen_delta_86400"].builder_input["program"]["profile"], {"kind": "timestamp", "timestamp_delta": 86_400, "value_class": "nonzero"})

    @staticmethod
    def _identity_bundle(fixture):
        spec = copy.deepcopy(fixture.builder_input)
        spec.update(
            {
                "expected_final_state_root": "0x" + "12" * 32,
                "expected_raw_gas_by_key": {"opcode:0x30": fixture.count * 2},
                "expected_context_features": (
                    {"context_fixed:opcode:0x30": fixture.count}
                    if fixture.lane == "target" and fixture.count
                    else {}
                ),
                "expected_features": {"proposal_startup": 1, "block_base": 1, "tx_base": 1, "native_value_transfer": 0},
                "expected_diagnostics": {"bytecode_length": 256},
                "expected_backend_input_sha256": "a" * 64,
                "expected_host_trace_sha256": "b" * 64,
            }
        )
        observation = {
            "row_id": fixture.row_id,
            "backend_input_sha256": "a" * 64,
            "host_trace_sha256": "b" * 64,
            "actual_final_state_root": "0x" + "12" * 32,
            "actual_raw_gas_by_key": dict(spec["expected_raw_gas_by_key"]),
            "actual_operation_event_count_by_key": dict(
                spec["expected_operation_event_count_by_key"]
            ),
            "actual_context_features": dict(spec["expected_context_features"]),
            "actual_features": dict(spec["expected_features"]),
            "actual_diagnostics": dict(spec["expected_diagnostics"]),
        }
        return {
            "schema_version": 1,
            "fixture_spec_sha256": production.sha256_bytes(production.canonical_json(spec)),
            "spec": spec,
            "observation": observation,
        }

    def test_fixture_identity_requires_exact_observed_ledger_equality(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        fixture = next(
            row
            for row in production.production_context_fixture_requests(manifest)
            if row.scenario == "address_canonical"
            and row.count == 2
            and row.lane == "target"
            and row.repeat_index == 0
        )
        bundle = self._identity_bundle(fixture)
        validated = production.validate_production_context_fixture_identity(fixture, bundle)
        self.assertEqual(validated["fixture_spec_sha256"], bundle["fixture_spec_sha256"])
        for field in (
            "expected_raw_gas_by_key",
            "expected_context_features",
            "expected_features",
            "expected_diagnostics",
            "expected_final_state_root",
            "expected_backend_input_sha256",
            "expected_host_trace_sha256",
        ):
            drifted = copy.deepcopy(bundle)
            if isinstance(drifted["spec"][field], dict):
                drifted["spec"][field]["drift"] = 1
            else:
                drifted["spec"][field] = "c" * 64
            drifted["fixture_spec_sha256"] = production.sha256_bytes(
                production.canonical_json(drifted["spec"])
            )
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "mismatch"):
                production.validate_production_context_fixture_identity(fixture, drifted)

    def test_fixture_identity_invokes_native_rust_trace_without_sp1(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        fixture = production.production_context_fixture_requests(manifest)[0]
        bundle = self._identity_bundle(fixture)
        with tempfile.TemporaryDirectory() as directory:
            launcher = pathlib.Path(directory) / "guest-launcher"
            launcher.write_bytes(b"test launcher")

            def run(command, **kwargs):
                self.assertEqual(
                    command[1:7],
                    [
                        "--stage",
                        "controlled-block-identity",
                        "--proof-type",
                        "native",
                        "--mode",
                        "execute",
                    ],
                )
                self.assertNotIn("--sp1-prover", command)
                self.assertNotIn("--elf", command)
                output = pathlib.Path(command[command.index("--json-out") + 1])
                output.write_bytes(production.canonical_json(bundle) + b"\n")
                return subprocess.CompletedProcess(command, 0, "", "")

            with mock.patch.object(production.subprocess, "run", side_effect=run) as invoked:
                observed = production.run_production_context_fixture_identity(
                    fixture, launcher=launcher, repo_root=ROOT
                )
            self.assertEqual(invoked.call_count, 1)
            self.assertEqual(observed["fixture_spec_sha256"], bundle["fixture_spec_sha256"])

    def test_standard_estimator_parity_is_identity_only_not_a_model_sample(self):
        common = {
            "stage": "controlled-block",
            "mode": "execute",
            "gas": 159_030_265,
            "total_instruction_count": 136_594_192,
            "total_syscall_count": 229_747,
            "public_values": "0x1234",
            "guest_input_sha256": "0x" + "a" * 64,
            "exit_code": 0,
            "controlled_block": {
                "status": "accepted",
                "row_id": "c" * 64,
                "observation": {"host_trace_sha256": "b" * 64},
            },
        }
        standard = {**common, "sp1_execution_engine": "standard"}
        estimator = {
            **common,
            "sp1_execution_engine": "gas-estimator",
            "sp1_proposal_elf_sha256": "d" * 64,
            "guest_launcher_sha256": "e" * 64,
        }
        identity = production.production_context_parity_identity(
            row_id="c" * 64, standard=standard, gas_estimator=estimator
        )
        self.assertEqual(identity["kind"], "production_context_parity_v1")
        self.assertFalse(identity["model_sample"])
        self.assertEqual(identity["standard_execution"]["mode"], "execute")
        self.assertEqual(
            identity["gas_estimator_assets"]["sp1_proposal_elf_sha256"],
            "d" * 64,
        )
        self.assertEqual(len(identity["identity_sha256"]), 64)
        drifted = copy.deepcopy(estimator)
        drifted["gas"] += 1
        with self.assertRaisesRegex(ValueError, "parity mismatch"):
            production.production_context_parity_identity(
                row_id="c" * 64, standard=standard, gas_estimator=drifted
            )

    def test_standard_estimator_parity_requires_accepted_row_bound_successes(self):
        common = {
            "stage": "controlled-block",
            "mode": "execute",
            "gas": 1,
            "total_instruction_count": 2,
            "total_syscall_count": 3,
            "public_values": "0x1234",
            "guest_input_sha256": "0x" + "a" * 64,
            "exit_code": 0,
            "controlled_block": {
                "status": "accepted",
                "row_id": "c" * 64,
                "observation": {"host_trace_sha256": "b" * 64},
            },
        }
        standard = {**copy.deepcopy(common), "sp1_execution_engine": "standard"}
        estimator = {
            **copy.deepcopy(common),
            "sp1_execution_engine": "gas-estimator",
            "sp1_proposal_elf_sha256": "d" * 64,
            "guest_launcher_sha256": "e" * 64,
        }
        invalid_reports = []
        wrong_row = copy.deepcopy(estimator)
        wrong_row["controlled_block"]["row_id"] = "d" * 64
        invalid_reports.append(("wrong row", standard, wrong_row))
        wrong_standard_row = copy.deepcopy(standard)
        wrong_standard_row["controlled_block"]["row_id"] = "d" * 64
        invalid_reports.append(("wrong standard row", wrong_standard_row, estimator))
        rejected = copy.deepcopy(estimator)
        rejected["controlled_block"]["status"] = "rejected"
        invalid_reports.append(("rejected status", standard, rejected))
        rejected_standard = copy.deepcopy(standard)
        rejected_standard["controlled_block"]["status"] = "rejected"
        invalid_reports.append(
            ("rejected standard status", rejected_standard, estimator)
        )
        nonzero_standard = copy.deepcopy(standard)
        nonzero_estimator = copy.deepcopy(estimator)
        nonzero_standard["exit_code"] = 1
        nonzero_estimator["exit_code"] = 1
        invalid_reports.append(
            ("matching nonzero exits", nonzero_standard, nonzero_estimator)
        )
        for field in ("row_id", "status"):
            missing = copy.deepcopy(estimator)
            del missing["controlled_block"][field]
            invalid_reports.append((f"missing {field}", standard, missing))
            missing_standard = copy.deepcopy(standard)
            del missing_standard["controlled_block"][field]
            invalid_reports.append(
                (f"missing standard {field}", missing_standard, estimator)
            )
        missing_trace = copy.deepcopy(estimator)
        del missing_trace["controlled_block"]["observation"]["host_trace_sha256"]
        invalid_reports.append(("missing host trace", standard, missing_trace))
        missing_controlled = copy.deepcopy(estimator)
        del missing_controlled["controlled_block"]
        invalid_reports.append(
            ("missing controlled block", standard, missing_controlled)
        )
        for case, candidate_standard, candidate_estimator in invalid_reports:
            with self.subTest(case=case), self.assertRaisesRegex(
                ValueError, "parity evidence"
            ):
                production.production_context_parity_identity(
                    row_id="c" * 64,
                    standard=candidate_standard,
                    gas_estimator=candidate_estimator,
                )

    def test_parity_identity_validator_rejects_placeholder_extra_and_asset_drift(self):
        common = {
            "stage": "controlled-block",
            "mode": "execute",
            "gas": 1,
            "total_instruction_count": 2,
            "total_syscall_count": 3,
            "public_values": "0x01",
            "guest_input_sha256": "0x" + "a" * 64,
            "exit_code": 0,
            "controlled_block": {
                "status": "accepted",
                "row_id": "c" * 64,
                "observation": {"host_trace_sha256": "b" * 64},
            },
        }
        identity = production.production_context_parity_identity(
            row_id="c" * 64,
            standard={**copy.deepcopy(common), "sp1_execution_engine": "standard"},
            gas_estimator={
                **copy.deepcopy(common),
                "sp1_execution_engine": "gas-estimator",
                "sp1_proposal_elf_sha256": "d" * 64,
                "guest_launcher_sha256": "e" * 64,
            },
        )
        production.validate_production_context_parity_identity(
            identity,
            row_ids=("c" * 64,),
            production_elf_sha256="d" * 64,
            guest_launcher_sha256="e" * 64,
        )
        invalid = []
        placeholder = {"kind": "production_context_parity_v1", "model_sample": False}
        placeholder["identity_sha256"] = production.sha256_bytes(
            production.canonical_json(placeholder)
        )
        invalid.append(placeholder)
        extra = copy.deepcopy(identity)
        extra["placeholder"] = True
        invalid.append(extra)
        malformed = copy.deepcopy(identity)
        malformed["standard_execution"]["status"] = "rejected"
        malformed["identity_sha256"] = production.sha256_bytes(
            production.canonical_json(
                {key: value for key, value in malformed.items() if key != "identity_sha256"}
            )
        )
        invalid.append(malformed)
        for candidate in invalid:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                production.validate_production_context_parity_identity(
                    candidate,
                    row_ids=("c" * 64,),
                    production_elf_sha256="d" * 64,
                    guest_launcher_sha256="e" * 64,
                )
        with self.assertRaisesRegex(ValueError, "asset join"):
            production.validate_production_context_parity_identity(
                identity,
                row_ids=("c" * 64,),
                production_elf_sha256="f" * 64,
                guest_launcher_sha256="e" * 64,
            )


class ProductionContextCliTests(unittest.TestCase):
    def test_cli_registers_live_campaign_surfaces(self):
        completed = subprocess.run(
            [sys.executable, str(OPCODE_GAS / "opcode_gas.py"), "--help"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn("generate-context-production-manifest", completed.stdout)
        self.assertIn("validate-context-production-manifest", completed.stdout)
        for live in (
            "prepare-context-production",
            "resume-context-production",
            "run-context-production",
            "fit-context-production",
            "verify-context-production-result",
            "seal-context-production-result",
            "run-context-diagnostics",
            "verify-context-diagnostics",
            "seal-context-approximation",
            "verify-context-approximation",
        ):
            self.assertIn(live, completed.stdout)

    def test_generate_is_create_only_and_validate_prints_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "manifest.json"
            generate = [
                sys.executable,
                str(OPCODE_GAS / "opcode_gas.py"),
                "generate-context-production-manifest",
                "--out",
                str(output),
            ]
            subprocess.run(generate, cwd=ROOT, check=True, capture_output=True, text=True)
            self.assertEqual(output.read_bytes(), MANIFEST.read_bytes())
            rejected = subprocess.run(generate, cwd=ROOT, capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)

            validated = subprocess.run(
                [
                    sys.executable,
                    str(OPCODE_GAS / "opcode_gas.py"),
                    "validate-context-production-manifest",
                    "--manifest",
                    str(output),
                ],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertIn(production.load_production_context_manifest(output).identity_sha256, validated.stdout)


class ProductionContextFitTests(unittest.TestCase):
    @staticmethod
    def _frozen_fixture(fixture):
        builder_input = copy.deepcopy(fixture.builder_input)
        builder_input["expected_backend_input_sha256"] = "a" * 64
        builder_input["expected_host_trace_sha256"] = "b" * 64
        builder_input["expected_operation_event_count_by_key"] = {}
        return production.ProductionContextFixtureRequest(
            scenario=fixture.scenario,
            split=fixture.split,
            count=fixture.count,
            lane=fixture.lane,
            repeat_index=fixture.repeat_index,
            workload_id=fixture.workload_id,
            row_id=fixture.row_id,
            builder_input=builder_input,
        )

    @staticmethod
    def _parity_fixture(manifest):
        contract = production.PARITY_ROW_CONTRACT
        return next(
            fixture
            for fixture in production.production_context_fixture_requests(manifest)
            if fixture.scenario == contract["scenario"]
            and fixture.split == contract["split"]
            and fixture.count == contract["count"]
            and fixture.lane == contract["lane"]
            and fixture.repeat_index == contract["repeat_index"]
        )

    @staticmethod
    def _parity_identity(*, row_id, launcher, elf):
        identity = {
            "kind": "production_context_parity_v1",
            "row_id": row_id,
            "row_contract": dict(production.PARITY_ROW_CONTRACT),
            "model_sample": False,
            "standard_execution": {
                "stage": "controlled-block",
                "mode": "execute",
                "engine": "standard",
                "status": "accepted",
                "exit_code": 0,
            },
            "gas_estimator_execution": {
                "stage": "controlled-block",
                "mode": "execute",
                "engine": "gas-estimator",
                "status": "accepted",
                "exit_code": 0,
            },
            "gas_estimator_assets": {
                "sp1_proposal_elf_sha256": production._sha256_file(elf),
                "guest_launcher_sha256": production._sha256_file(launcher),
            },
            "equal_values": {
                "gas": 1,
                "total_instruction_count": 2,
                "total_syscall_count": 3,
                "public_values": "0x01",
                "guest_input_sha256": "0x" + "a" * 64,
                "host_trace_sha256": "b" * 64,
                "exit_code": 0,
            },
        }
        identity["identity_sha256"] = production.sha256_bytes(
            production.canonical_json(identity)
        )
        return identity

    @staticmethod
    def _subtotal_model(control_price="5"):
        return production.ProductionSubtotalModel.from_mappings(
            fixed_costs={
                "proposal_startup": "100",
                "block_base": "20",
                "tx_base": "3",
                "native_value_transfer": "7",
            },
            opcode_prices={
                "opcode:0x5f": control_price,
                "opcode:0x60": "2",
                "opcode:0x90": "11",
            },
        )

    @staticmethod
    def _row(*, scenario, split, count, lane, repeat, prover_gas, target_key, control_key):
        measured = target_key if lane == "target" else control_key
        raw = {"opcode:0x60": 9}
        event_counts = {"opcode:0x60": 3}
        if count:
            opcode = int(measured.removeprefix("opcode:0x"), 16)
            raw[measured] = count * production._measurement_raw_gas(opcode)
            event_counts[measured] = count
        return {
            "row_id": production.sha256_bytes(
                production.canonical_json(
                    [scenario, split, count, lane, repeat]
                )
            ),
            "scenario": scenario,
            "split": split,
            "count": count,
            "lane": lane,
            "repeat_index": repeat,
            "prover_gas": str(prover_gas),
            "public_output": "0x1234",
            "backend_input_sha256": "a" * 64,
            "host_trace_sha256": "b" * 64,
            "sp1_proposal_elf_sha256": "d" * 64,
            "guest_launcher_sha256": "e" * 64,
            "actual_raw_gas_by_key": raw,
            "actual_operation_event_count_by_key": event_counts,
            "actual_context_features": (
                {f"context_fixed:{target_key}": count}
                if lane == "target" and count
                else {}
            ),
            "actual_features": {
                "proposal_startup": 1,
                "block_base": 1,
                "tx_base": 1,
                "native_value_transfer": 0,
            },
            "actual_diagnostics": {"bytecode_length": 256},
        }

    def _address_rows(self, *, coefficient=300, control_price=5):
        rows = []
        base = Decimal(141)
        non_target = Decimal(18)
        for scenario, split, counts in (
            ("address_canonical", "fit", (0, 1, 2, 4, 8, 16)),
            ("address_alternate", "final_holdout", (0, 32, 64)),
        ):
            for count in counts:
                for lane in ("control", "target"):
                    increment = (
                        Decimal(count * coefficient)
                        if lane == "target"
                        else Decimal(count * 2 * control_price)
                    )
                    for repeat in range(3):
                        rows.append(
                            self._row(
                                scenario=scenario,
                                split=split,
                                count=count,
                                lane=lane,
                                repeat=repeat,
                                prover_gas=base + non_target + increment,
                                target_key="opcode:0x30",
                                control_key="opcode:0x5f",
                            )
                        )
        return rows

    def _complete_rows(self, *, calldatasize_model="length"):
        manifest = production.load_production_context_manifest(MANIFEST)
        model = self._subtotal_model()

        def event_cost(scenario):
            context = scenario.context
            if scenario.key == "opcode:0x30":
                return Decimal(300)
            if scenario.key == "opcode:0x33":
                return Decimal(310)
            if scenario.key == "opcode:0x34":
                return Decimal(320 if context["value_class"] == "zero" else 340)
            if scenario.key == "opcode:0x35":
                return {
                    "zero": Decimal(330),
                    "partial": Decimal(350),
                    "full": Decimal(370),
                }[context["access_class"]]
            if scenario.key == "opcode:0x36":
                length = Decimal(context["input_length"])
                if calldatasize_model == "length":
                    return Decimal(300) + Decimal(2) * length
                words = Decimal((context["input_length"] + 31) // 32)
                partial = Decimal(context["input_length"] % 32 != 0)
                if calldatasize_model == "boundary_zero_words":
                    return Decimal(300) + Decimal(100) * partial
                return Decimal(300) + Decimal(20) * words + Decimal(5) * partial
            if scenario.key == "opcode:0x42":
                return Decimal(360)
            raise AssertionError(scenario.key)

        rows = []
        for scenario in manifest.scenarios:
            if scenario.reachability != "measured":
                continue
            operation = manifest.operation(scenario.key)
            control_key = f"opcode:0x{operation.control_opcode:02x}"
            control_price = model.opcode_prices[control_key]
            feature_key = production._expected_context_feature(operation, scenario)
            for count in scenario.counts(manifest):
                for lane in ("control", "target"):
                    measured_key = scenario.key if lane == "target" else control_key
                    measured_opcode = (
                        scenario.opcode if lane == "target" else operation.control_opcode
                    )
                    raw = {"opcode:0x60": 9}
                    event_counts = {"opcode:0x60": 3}
                    if count:
                        raw[measured_key] = (
                            count * production._measurement_raw_gas(measured_opcode)
                        )
                        event_counts[measured_key] = count
                    increment = Decimal(count) * (
                        event_cost(scenario)
                        if lane == "target"
                        else production._measurement_raw_gas(measured_opcode)
                        * control_price
                    )
                    for repeat in range(3):
                        row = self._row(
                            scenario=scenario.name,
                            split=scenario.split,
                            count=count,
                            lane=lane,
                            repeat=repeat,
                            prover_gas=Decimal(159) + increment,
                            target_key=scenario.key,
                            control_key=control_key,
                        )
                        row["actual_raw_gas_by_key"] = dict(raw)
                        row["actual_operation_event_count_by_key"] = dict(
                            event_counts
                        )
                        row["actual_context_features"] = (
                            {feature_key: count}
                            if lane == "target" and count
                            else {}
                        )
                        rows.append(row)
        return rows

    def _real_calldataload_rows(self):
        rows = [
            row
            for row in self._complete_rows()
            if row["scenario"].startswith("calldataload_")
        ]
        swap1_price = self._subtotal_model().opcode_prices["opcode:0x90"]
        for row in rows:
            count = row["count"]
            common_raw = {"opcode:0x60": 9}
            common_events = {"opcode:0x60": 3}
            if count == 0:
                row["actual_raw_gas_by_key"] = common_raw
                row["actual_operation_event_count_by_key"] = common_events
                continue
            if row["lane"] == "target":
                row["actual_raw_gas_by_key"] = {
                    **common_raw,
                    "opcode:0x35": 3 * count,
                    "opcode:0x90": 3 * count,
                }
                row["actual_operation_event_count_by_key"] = {
                    **common_events,
                    "opcode:0x35": count,
                    "opcode:0x90": count,
                }
                row["prover_gas"] = str(
                    Decimal(row["prover_gas"]) + Decimal(3 * count) * swap1_price
                )
            else:
                row["actual_raw_gas_by_key"] = {
                    **common_raw,
                    "opcode:0x90": 6 * count,
                }
                row["actual_operation_event_count_by_key"] = {
                    **common_events,
                    "opcode:0x90": 2 * count,
                }
                row["prover_gas"] = str(
                    Decimal(row["prover_gas"]) + Decimal(3 * count) * swap1_price
                )
        return rows

    def test_calldataload_shared_swap1_is_retained_after_measurement_subtraction(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        result = production.fit_production_context_rows(
            manifest,
            self._real_calldataload_rows(),
            self._subtotal_model(),
            family_keys=("opcode:0x35",),
        )["families"]["opcode:0x35"]
        self.assertEqual(
            result["coefficients"],
            {"load_full": "370", "load_partial": "350", "load_zero": "330"},
        )
        self.assertEqual(result["status"], "accepted")
        self.assertEqual(result["maximum_control_residual"], "0")

    def test_measurement_subtraction_retains_shared_control_opcode_work(self):
        self.assertEqual(
            production._subtract_measurement_contribution(
                {"opcode:0x60": 9, "opcode:0x90": 6},
                {"opcode:0x60": 3, "opcode:0x90": 2},
                key="opcode:0x90",
                count=1,
                per_event_raw_gas=3,
            ),
            (
                {"opcode:0x60": 9, "opcode:0x90": 3},
                {"opcode:0x60": 3, "opcode:0x90": 1},
            ),
        )
        self.assertEqual(
            production._subtract_measurement_contribution(
                {"opcode:0x60": 9, "opcode:0x5f": 4},
                {"opcode:0x60": 3, "opcode:0x5f": 2},
                key="opcode:0x5f",
                count=1,
                per_event_raw_gas=2,
            ),
            (
                {"opcode:0x60": 9, "opcode:0x5f": 2},
                {"opcode:0x60": 3, "opcode:0x5f": 1},
            ),
        )
        unchanged = ({"opcode:0x60": 9}, {"opcode:0x60": 3})
        self.assertEqual(
            production._subtract_measurement_contribution(
                *unchanged,
                key="opcode:0x90",
                count=0,
                per_event_raw_gas=3,
            ),
            unchanged,
        )

    def test_measurement_subtraction_rejects_malformed_aggregate_ledgers(self):
        cases = (
            (
                "missing",
                {"opcode:0x60": 9},
                {"opcode:0x60": 3},
            ),
            (
                "underflow",
                {"opcode:0x90": 2},
                {"opcode:0x90": 1},
            ),
            (
                "non-integral",
                {"opcode:0x90": 5},
                {"opcode:0x90": 2},
            ),
            (
                "per-event raw gas differs",
                {"opcode:0x90": 4},
                {"opcode:0x90": 2},
            ),
            (
                "nonnegative",
                {"opcode:0x90": -3},
                {"opcode:0x90": 1},
            ),
        )
        for expected, raw_gas, event_counts in cases:
            with self.subTest(expected=expected), self.assertRaisesRegex(
                ValueError, expected
            ):
                production._subtract_measurement_contribution(
                    raw_gas,
                    event_counts,
                    key="opcode:0x90",
                    count=1,
                    per_event_raw_gas=3,
                )

    def test_target_fit_is_independent_of_historical_control_price(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        rows = self._address_rows()
        accepted = production.fit_production_context_rows(
            manifest,
            rows,
            self._subtotal_model("5"),
            family_keys=("opcode:0x30",),
        )
        rejected_control = production.fit_production_context_rows(
            manifest,
            rows,
            self._subtotal_model("500"),
            family_keys=("opcode:0x30",),
        )
        self.assertEqual(
            accepted["families"]["opcode:0x30"]["coefficients"],
            rejected_control["families"]["opcode:0x30"]["coefficients"],
        )
        self.assertEqual(
            accepted["families"]["opcode:0x30"]["coefficients"],
            {"address_constant": "300"},
        )
        self.assertEqual(accepted["families"]["opcode:0x30"]["status"], "accepted")
        self.assertEqual(
            rejected_control["families"]["opcode:0x30"]["status"], "rejected"
        )
        self.assertIn(
            "control_contamination",
            rejected_control["families"]["opcode:0x30"]["rejection_reasons"],
        )

    def test_v5_subtotal_excludes_only_target_and_allows_exact_zero_work(self):
        row = self._address_rows()[0]
        row["actual_raw_gas_by_key"]["opcode:0x00"] = 0
        row["actual_operation_event_count_by_key"]["opcode:0x00"] = 0
        subtotal = production.evaluate_v5_subtotal(
            row, self._subtotal_model(), excluded_target_key=None
        )
        self.assertEqual(subtotal, Decimal(141))
        row["actual_raw_gas_by_key"]["opcode:0xfe"] = 1
        row["actual_operation_event_count_by_key"]["opcode:0xfe"] = 1
        with self.assertRaisesRegex(ValueError, "unpriced"):
            production.evaluate_v5_subtotal(
                row, self._subtotal_model(), excluded_target_key=None
            )

        manifest = production.load_production_context_manifest(MANIFEST)
        loaded = production.load_production_v5_subtotal_model(manifest, ROOT)
        self.assertEqual(set(loaded.fixed_costs), set(self._subtotal_model().fixed_costs))
        self.assertIn("opcode:0x5f", loaded.opcode_prices)
        self.assertNotIn("opcode:0x30", loaded.opcode_model_kinds)
        self.assertEqual(len(loaded.opcode_model_kinds), 103)

    def test_v5_subtotal_prices_each_structured_family_with_authoritative_predictor(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        model = production.load_production_v5_subtotal_model(manifest, ROOT)
        cases = {
            "opcode:0x0a": {
                "interpreter_raw_gas": 60,
                "model_input": {"kind": "exp", "exponent_byte_length": 2},
            },
            "opcode:0x20": {
                "interpreter_raw_gas": 36,
                "model_input": {
                    "kind": "keccak",
                    "input_length": 32,
                    "memory_growth_event": 0,
                    "memory_evm_gas_delta": 0,
                    "memory_4k_boundary_event": 0,
                },
            },
            "opcode:0x51": {
                "interpreter_raw_gas": 3,
                "model_input": {
                    "kind": "memory_access",
                    "memory_growth_event": 0,
                    "memory_evm_gas_delta": 0,
                    "memory_4k_boundary_event": 0,
                },
            },
            "opcode:0x52": {
                "interpreter_raw_gas": 3,
                "model_input": {
                    "kind": "memory_access",
                    "memory_growth_event": 0,
                    "memory_evm_gas_delta": 0,
                    "memory_4k_boundary_event": 0,
                },
            },
            "opcode:0x53": {
                "interpreter_raw_gas": 3,
                "model_input": {
                    "kind": "memory_access",
                    "memory_growth_event": 0,
                    "memory_evm_gas_delta": 0,
                    "memory_4k_boundary_event": 0,
                },
            },
            "opcode:0x5e": {
                "interpreter_raw_gas": 6,
                "model_input": {
                    "kind": "memory_copy",
                    "copy_words": 1,
                    "memory_growth_event": 0,
                    "memory_evm_gas_delta": 0,
                    "memory_4k_boundary_event": 0,
                },
            },
        }
        self.assertEqual(
            {key: model.opcode_model_kinds[key] for key in cases},
            {
                "opcode:0x0a": "exp",
                "opcode:0x20": "keccak",
                "opcode:0x51": "memory_access",
                "opcode:0x52": "memory_access",
                "opcode:0x53": "memory_access",
                "opcode:0x5e": "memory_copy",
            },
        )
        for key, component in cases.items():
            opcode = int(key.removeprefix("opcode:0x"), 16)
            row = {
                "actual_features": {name: 0 for name in model.fixed_costs},
                "actual_raw_gas_by_key": {key: component["interpreter_raw_gas"]},
                "actual_operation_event_count_by_key": {key: 1},
                "actual_typed_opcode_components_by_key": {key: [component]},
            }
            expected = production.predict_opcode_event(
                model.typed_registry, production._opcode_event(opcode, component)
            )
            with self.subTest(key=key):
                self.assertEqual(
                    production.evaluate_v5_subtotal(
                        row, model, excluded_target_key=None
                    ),
                    expected,
                )
            missing = copy.deepcopy(row)
            missing.pop("actual_typed_opcode_components_by_key")
            with self.assertRaisesRegex(ValueError, "missing_typed_features"):
                production.evaluate_v5_subtotal(
                    missing, model, excluded_target_key=None
                )
            mismatched = copy.deepcopy(row)
            mismatched["actual_raw_gas_by_key"][key] += 1
            with self.assertRaisesRegex(ValueError, "units mismatch"):
                production.evaluate_v5_subtotal(
                    mismatched, model, excluded_target_key=None
                )
        extra = {
            "actual_features": {name: 0 for name in model.fixed_costs},
            "actual_raw_gas_by_key": {"opcode:0x60": 3},
            "actual_operation_event_count_by_key": {"opcode:0x60": 1},
            "actual_typed_opcode_components_by_key": {
                "opcode:0x0a": [cases["opcode:0x0a"]]
            },
        }
        with self.assertRaisesRegex(ValueError, "extras"):
            production.evaluate_v5_subtotal(extra, model, excluded_target_key=None)

    def test_v5_static_subtotal_matches_authoritative_push1_predictor(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        model = production.load_production_v5_subtotal_model(manifest, ROOT)
        component = {
            "interpreter_raw_gas": 3,
            "model_input": {"kind": "static_raw_gas", "raw_gas": 3},
        }
        row = {
            "actual_features": {name: 0 for name in model.fixed_costs},
            "actual_raw_gas_by_key": {"opcode:0x60": 3},
            "actual_operation_event_count_by_key": {"opcode:0x60": 1},
        }
        expected = production.predict_opcode_event(
            model.typed_registry, production._opcode_event(0x60, component)
        )
        self.assertEqual(
            production.evaluate_v5_subtotal(row, model, excluded_target_key=None),
            expected,
        )
        missing = copy.deepcopy(row)
        missing.pop("actual_operation_event_count_by_key")
        with self.assertRaisesRegex(ValueError, "event-count ledger"):
            production.evaluate_v5_subtotal(missing, model, excluded_target_key=None)
        mismatched = copy.deepcopy(row)
        mismatched["actual_operation_event_count_by_key"] = {"opcode:0x61": 1}
        with self.assertRaisesRegex(ValueError, "event-count ledger"):
            production.evaluate_v5_subtotal(mismatched, model, excluded_target_key=None)

    def test_zero_coefficient_relative_stderr_is_explicitly_not_applicable(self):
        fit = production.fit_exact_decimal_model(
            matrix=((1, 0), (2, 0), (0, 1), (0, 2)),
            observed=(10, 20, "0.2", "-0.1"),
            terms=("positive", "zero"),
        )
        self.assertEqual(fit["coefficients"]["zero"], "0")
        self.assertEqual(
            fit["relative_coefficient_stderr"]["zero"],
            {"status": "not_applicable_zero_coefficient", "value": None},
        )
        self.assertNotIn("coefficient_stderr", production._candidate_gate_reasons(
            fit, tuple(Decimal(str(value)) for value in fit["observed"])
        ))
        encoded = production.canonical_json(fit)
        self.assertNotIn(b"Infinity", encoded)
        self.assertNotIn(b"NaN", encoded)
        self.assertEqual(json.loads(encoded), fit)

    def test_prepared_row_evidence_requires_every_frozen_observation(self):
        row_id = "1" * 64
        payload = {
            "row_id": row_id,
            "workload_id": "2" * 64,
            "scenario": "address_canonical",
            "split": "fit",
            "count": 1,
            "lane": "target",
            "repeat_index": 0,
            "builder_input": {
                "row_id": row_id,
                "workload_family": "context_opcode",
                "split": "fit",
                "block_count": 1,
                "transaction_count": 1,
                "program": {
                    "kind": "context_opcode_loop",
                    "workload_id": "2" * 64,
                    "repeat_index": 0,
                    "opcode": "address",
                    "lane": "target",
                    "count": 1,
                    "profile": {
                        "kind": "address",
                        "address_profile": "canonical",
                    },
                },
                "expected_final_state_root": "0x" + "3" * 64,
                "expected_raw_gas_by_key": {"opcode:0x60": 3},
                "expected_operation_event_count_by_key": {"opcode:0x60": 1},
                "expected_context_features": {"context_fixed:opcode:0x30": 1},
                "expected_features": {
                    "proposal_startup": 1,
                    "block_base": 1,
                    "tx_base": 1,
                    "native_value_transfer": 0,
                },
                "expected_diagnostics": {"bytecode_length": 256},
                "expected_backend_input_sha256": "a" * 64,
                "expected_host_trace_sha256": "b" * 64,
            },
        }
        row_input = {
            **payload,
            "input_sha256": production.sha256_bytes(production.canonical_json(payload)),
        }
        evidence = {
            "row_id": row_id,
            "input_sha256": row_input["input_sha256"],
            "workload_id": "2" * 64,
            "scenario": "address_canonical",
            "split": "fit",
            "count": 1,
            "lane": "target",
            "repeat_index": 0,
            "prover_gas": "123",
            "public_output": "0x1234",
            "backend_input_sha256": "a" * 64,
            "host_trace_sha256": "b" * 64,
            "sp1_proposal_elf_sha256": "c" * 64,
            "guest_launcher_sha256": "d" * 64,
            "actual_final_state_root": "0x" + "3" * 64,
            "actual_raw_gas_by_key": {"opcode:0x60": 3},
            "actual_operation_event_count_by_key": {"opcode:0x60": 1},
            "actual_context_features": {"context_fixed:opcode:0x30": 1},
            "actual_features": {
                "proposal_startup": 1,
                "block_base": 1,
                "tx_base": 1,
                "native_value_transfer": 0,
            },
            "actual_diagnostics": {"bytecode_length": 256},
        }
        evidence["evidence_sha256"] = production.sha256_bytes(
            production.canonical_json(evidence)
        )
        self.assertEqual(
            production._validate_prepared_row_evidence(
                row_input,
                evidence,
                production_elf_sha256="c" * 64,
                guest_launcher_sha256="d" * 64,
            ),
            evidence,
        )
        substitutions = {
            "actual_final_state_root": "0x" + "4" * 64,
            "actual_raw_gas_by_key": {"opcode:0x61": 6},
            "actual_operation_event_count_by_key": {"opcode:0x61": 2},
            "actual_context_features": {"context_fixed:opcode:0x33": 1},
            "actual_features": {
                "proposal_startup": 1,
                "block_base": 2,
                "tx_base": 1,
                "native_value_transfer": 0,
            },
            "actual_diagnostics": {"bytecode_length": 257},
        }
        for field, replacement in substitutions.items():
            forged = copy.deepcopy(evidence)
            forged[field] = replacement
            forged_unhashed = dict(forged)
            forged_unhashed.pop("evidence_sha256")
            forged["evidence_sha256"] = production.sha256_bytes(
                production.canonical_json(forged_unhashed)
            )
            with self.subTest(field=field), self.assertRaisesRegex(
                ValueError, "frozen observation"
            ):
                production._validate_prepared_row_evidence(
                    row_input,
                    forged,
                    production_elf_sha256="c" * 64,
                    guest_launcher_sha256="d" * 64,
                )

    def test_residualization_rejects_incomplete_or_contaminated_inputs(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        cases = []
        missing_zero = [row for row in self._address_rows() if row["count"] != 0]
        cases.append(("count-zero", missing_zero))
        duplicate = self._address_rows()
        duplicate.append(copy.deepcopy(duplicate[-1]))
        cases.append(("duplicate", duplicate))
        mismatched = self._address_rows()
        mismatched[0]["actual_raw_gas_by_key"]["opcode:0x60"] += 1
        cases.append(("non-target ledger", mismatched))
        unpriced = self._address_rows()
        for row in unpriced:
            if row["scenario"] == "address_canonical" and row["count"] == 0:
                row["actual_raw_gas_by_key"]["opcode:0xfe"] = 1
                row["actual_operation_event_count_by_key"]["opcode:0xfe"] = 1
        cases.append(("unpriced", unpriced))
        target_in_subtotal = self._address_rows()
        next(row for row in target_in_subtotal if row["lane"] == "target")[
            "subtotal_keys"
        ] = ["opcode:0x30"]
        cases.append(("target context", target_in_subtotal))
        forbidden = self._address_rows()
        forbidden[0]["body_scale"] = "1"
        cases.append(("forbidden", forbidden))
        for expected, rows in cases:
            with self.subTest(expected=expected), self.assertRaisesRegex(
                ValueError, expected
            ):
                production.fit_production_context_rows(
                    manifest,
                    rows,
                    self._subtotal_model(),
                    family_keys=("opcode:0x30",),
                )

    def test_exact_models_cover_frozen_family_feature_shapes(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        expected = {
            "address_constant": (["1"],),
            "caller_constant": (["1"],),
            "callvalue_classes": (["1", "0"], ["0", "1"]),
            "calldataload_access_classes": (
                ["1", "0", "0"],
                ["0", "1", "0"],
                ["0", "0", "1"],
            ),
            "calldatasize_length": (["1", "33"],),
            "calldatasize_boundary": (["1", "2", "1"],),
            "timestamp_nonzero": (["1"],),
        }
        scenarios = {
            "address_constant": "address_canonical",
            "caller_constant": "caller_canonical",
            "callvalue_classes": ("callvalue_zero", "callvalue_nonzero_7"),
            "calldataload_access_classes": (
                "calldataload_empty_offset_0",
                "calldataload_partial_33_offset_17",
                "calldataload_full_32_offset_0",
            ),
            "calldatasize_length": "calldatasize_33",
            "calldatasize_boundary": "calldatasize_33",
            "timestamp_nonzero": "timestamp_post_unzen_delta_17",
        }
        for candidate, scenario_names in scenarios.items():
            if isinstance(scenario_names, str):
                scenario_names = (scenario_names,)
            vectors = tuple(
                production.production_context_design_row(
                    manifest.model_candidate(candidate), manifest.scenario(name)
                )
                for name in scenario_names
            )
            with self.subTest(candidate=candidate):
                self.assertEqual(
                    tuple([str(value) for value in row] for row in vectors),
                    expected[candidate],
                )

    def test_exact_decimal_fit_records_matrix_rank_residuals_and_gates(self):
        fit = production.fit_exact_decimal_model(
            matrix=((1, 0), (2, 0), (0, 1), (0, 2), (3, 0), (0, 3)),
            observed=(10, 20, 7, 14, 30, 21),
            terms=("left", "right"),
        )
        self.assertEqual(fit["coefficients"], {"left": "10", "right": "7"})
        self.assertEqual(fit["rank"], 2)
        self.assertEqual(fit["residuals"], ["0"] * 6)
        self.assertEqual(fit["design_matrix"][0], ["1", "0"])
        with self.assertRaisesRegex(ValueError, "rank deficient"):
            production.fit_exact_decimal_model(
                matrix=((1, 2), (2, 4)), observed=(1, 2), terms=("a", "b")
            )
        with self.assertRaisesRegex(ValueError, "nonnegative"):
            production.fit_exact_decimal_model(
                matrix=((1,), (2,), (3,)), observed=(-1, -2, -3), terms=("a",)
            )
        with self.assertRaisesRegex(ValueError, "finite"):
            production.fit_exact_decimal_model(
                matrix=((1,), (2,), (3,)),
                observed=("NaN", "2", "3"),
                terms=("a",),
            )

    def test_repeat_signal_and_exact_ape_fail_closed(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        rows = self._address_rows(coefficient=300)
        drifted = copy.deepcopy(rows)
        drifted[1]["public_output"] = "0xabcd"
        with self.assertRaisesRegex(ValueError, "repeat drift"):
            production.fit_production_context_rows(
                manifest,
                drifted,
                self._subtotal_model(),
                family_keys=("opcode:0x30",),
            )
        weak = production.fit_production_context_rows(
            manifest,
            self._address_rows(coefficient=100),
            self._subtotal_model(),
            family_keys=("opcode:0x30",),
        )
        self.assertIn(
            "insufficient_signal",
            weak["families"]["opcode:0x30"]["rejection_reasons"],
        )
        self.assertEqual(production.exact_ape(Decimal(11), Decimal(10)), Decimal("0.1"))
        with self.assertRaisesRegex(ValueError, "zero observed"):
            production.exact_ape(Decimal(1), Decimal(0))

    def test_negative_candidate_retains_observed_family_diagnostics(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        rows = [
            row
            for row in self._complete_rows()
            if row["scenario"].startswith("timestamp_")
        ]
        for row in rows:
            if row["count"] == 0:
                continue
            if row["lane"] == "target":
                row["prover_gas"] = str(Decimal(159) - Decimal(row["count"]))
            else:
                row["prover_gas"] = str(
                    Decimal(row["prover_gas"]) + Decimal(100 * row["count"])
                )

        family = production.fit_production_context_rows(
            manifest,
            rows,
            self._subtotal_model(),
            family_keys=("opcode:0x42",),
        )["families"]["opcode:0x42"]

        self.assertEqual(family["status"], "rejected")
        self.assertIsNone(family["selected_candidate"])
        self.assertEqual(family["coefficients"], {})
        self.assertEqual(
            set(family["rejection_reasons"]),
            {
                "control_contamination",
                "insufficient_signal",
                "negative_coefficient",
                "no_candidate_passed",
            },
        )
        self.assertEqual(family["maximum_control_residual"], "6400")
        self.assertIsNone(family["final_holdout_mape"])
        self.assertTrue(family["sibling_decisions"])
        fit_row = next(
            row
            for row in family["rows"]
            if row["scenario"] == "timestamp_post_unzen_delta_17"
            and row["count"] == 16
        )
        self.assertEqual(
            fit_row,
            {
                "scenario": "timestamp_post_unzen_delta_17",
                "split": "fit",
                "count": 16,
                "observed_increment": "-16",
                "predicted_increment": None,
                "control_residual": "1600",
            },
        )
        self.assertEqual(
            family["final_holdout_rows"],
            [
                {
                    "scenario": "timestamp_post_unzen_delta_86400",
                    "count": 32,
                    "observed_increment": "-32",
                    "predicted_increment": None,
                    "ape": None,
                },
                {
                    "scenario": "timestamp_post_unzen_delta_86400",
                    "count": 64,
                    "observed_increment": "-64",
                    "predicted_increment": None,
                    "ape": None,
                },
            ],
        )
        candidate = family["candidate_reports"]["timestamp_nonzero"]
        self.assertEqual(candidate["coefficients"], {"timestamp_nonzero": "-1"})
        self.assertIn("negative_coefficient", candidate["rejection_reasons"])

    def test_all_six_families_recover_and_calldatasize_selection_is_fixed_order(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        length = production.fit_production_context_rows(
            manifest, self._complete_rows(), self._subtotal_model()
        )
        self.assertTrue(length["all_families_accepted"])
        self.assertEqual(
            length["families"]["opcode:0x34"]["coefficients"],
            {"callvalue_zero": "320", "callvalue_nonzero": "340"},
        )
        self.assertEqual(
            length["families"]["opcode:0x35"]["coefficients"],
            {"load_zero": "330", "load_partial": "350", "load_full": "370"},
        )
        self.assertEqual(
            length["families"]["opcode:0x36"]["selected_candidate"],
            "calldatasize_length",
        )
        self.assertEqual(
            set(length["families"]["opcode:0x36"]["candidate_reports"]),
            {"calldatasize_length", "calldatasize_boundary"},
        )
        self.assertEqual(
            length["families"]["opcode:0x36"]["selection_decisions"],
            [
                {"candidate": "calldatasize_length", "status": "accepted"},
                {"candidate": "calldatasize_boundary", "status": "rejected"},
            ],
        )
        self.assertTrue(
            length["families"]["opcode:0x36"]["candidate_reports"]
            ["calldatasize_boundary"]["fit_rows"]
        )
        self.assertEqual(
            length["families"]["opcode:0x36"]["coefficients"],
            {"beta_0": "300", "beta_length": "2"},
        )

        boundary = production.fit_production_context_rows(
            manifest,
            self._complete_rows(calldatasize_model="boundary"),
            self._subtotal_model(),
            family_keys=("opcode:0x36",),
        )
        self.assertEqual(
            boundary["families"]["opcode:0x36"]["selected_candidate"],
            "calldatasize_boundary",
        )
        self.assertEqual(
            boundary["families"]["opcode:0x36"]["selection_decisions"],
            [
                {"candidate": "calldatasize_length", "status": "rejected"},
                {"candidate": "calldatasize_boundary", "status": "accepted"},
            ],
        )

        zero_words = production.fit_production_context_rows(
            manifest,
            self._complete_rows(calldatasize_model="boundary_zero_words"),
            self._subtotal_model(),
            family_keys=("opcode:0x36",),
        )["families"]["opcode:0x36"]
        self.assertEqual(zero_words["selected_candidate"], "calldatasize_boundary")
        self.assertEqual(
            zero_words["coefficients"],
            {"beta_0": "300", "beta_words": "0", "beta_partial": "100"},
        )
        self.assertIn(
            "negative_coefficient",
            zero_words["candidate_reports"]["calldatasize_length"][
                "rejection_reasons"
            ],
        )

    def test_fit_decisions_are_independent_of_ambient_decimal_precision(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        rows = self._complete_rows(calldatasize_model="boundary")
        encoded = []
        for precision in (6, 28, 80):
            with localcontext() as decimal_context:
                decimal_context.prec = precision
                encoded.append(
                    production.canonical_json(
                        production.fit_production_context_rows(
                            manifest,
                            rows,
                            self._subtotal_model(),
                        )
                    )
                )
        self.assertEqual(encoded[0], encoded[1])
        self.assertEqual(encoded[1], encoded[2])

    def test_fit_holdout_and_sibling_gates_do_not_tune_coefficients(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        baseline_rows = self._complete_rows()
        baseline = production.fit_production_context_rows(
            manifest,
            baseline_rows,
            self._subtotal_model(),
            family_keys=("opcode:0x30",),
        )["families"]["opcode:0x30"]

        final_drift = copy.deepcopy(baseline_rows)
        for row in final_drift:
            if (
                row["scenario"] == "address_alternate"
                and row["lane"] == "target"
                and row["count"]
            ):
                row["prover_gas"] = str(Decimal(row["prover_gas"]) + 10_000)
        rejected = production.fit_production_context_rows(
            manifest,
            final_drift,
            self._subtotal_model(),
            family_keys=("opcode:0x30",),
        )["families"]["opcode:0x30"]
        self.assertEqual(rejected["coefficients"], baseline["coefficients"])
        self.assertEqual(rejected["selected_candidate"], baseline["selected_candidate"])
        self.assertIn("count_holdout", rejected["rejection_reasons"])
        self.assertIn("extrapolation", rejected["rejection_reasons"])
        self.assertIn("final_holdout_row", rejected["rejection_reasons"])

        nonlinear = copy.deepcopy(baseline_rows)
        for row in nonlinear:
            if (
                row["scenario"] == "address_canonical"
                and row["lane"] == "target"
                and row["count"] == 8
            ):
                row["prover_gas"] = str(Decimal(row["prover_gas"]) + 2_000)
        nonlinear_result = production.fit_production_context_rows(
            manifest,
            nonlinear,
            self._subtotal_model(),
            family_keys=("opcode:0x30",),
        )["families"]["opcode:0x30"]
        self.assertTrue(
            {"fit_r2", "fit_residual"} & set(nonlinear_result["rejection_reasons"])
        )

        sibling_drift = copy.deepcopy(baseline_rows)
        for row in sibling_drift:
            if (
                row["scenario"] == "calldataload_out_of_range_4_offset_64"
                and row["lane"] == "target"
            ):
                row["prover_gas"] = str(
                    Decimal(row["prover_gas"]) + Decimal(row["count"] * 30)
                )
        sibling = production.fit_production_context_rows(
            manifest,
            sibling_drift,
            self._subtotal_model(),
            family_keys=("opcode:0x35",),
        )["families"]["opcode:0x35"]
        self.assertIn("sibling_inconsistency", sibling["rejection_reasons"])

        final_partial_drift = copy.deepcopy(baseline_rows)
        for row in final_partial_drift:
            if (
                row["scenario"] == "calldataload_partial_31_offset_30"
                and row["lane"] == "target"
                and row["count"]
            ):
                row["prover_gas"] = str(
                    Decimal(row["prover_gas"]) + Decimal(row["count"] * 28)
                )
        partial = production.fit_production_context_rows(
            manifest,
            final_partial_drift,
            self._subtotal_model(),
            family_keys=("opcode:0x35",),
        )["families"]["opcode:0x35"]
        self.assertEqual(partial["coefficients"]["load_partial"], "350")
        self.assertIn("sibling_inconsistency", partial["rejection_reasons"])
        partial_sibling = next(
            row
            for row in partial["sibling_decisions"]
            if row["model_class"] == "load_partial"
        )
        self.assertEqual(
            {row["split"]: row["slope"] for row in partial_sibling["diagnostic_slopes"]},
            {"fit": "350", "final_holdout": "378"},
        )

    def test_prepare_run_resume_is_create_only_and_hash_bound(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        fixture = self._frozen_fixture(
            self._parity_fixture(manifest)
        )
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            launcher = root / "guest-launcher"
            elf = root / "proposal.elf"
            vk = root / "proposal.vk"
            trace = root / "reconstruct.rs"
            for path, value in (
                (launcher, b"launcher"),
                (elf, b"elf"),
                (vk, b"vk"),
                (trace, b"trace"),
            ):
                path.write_bytes(value)
            run = root / "run"
            invalid_builders = []
            missing = copy.deepcopy(fixture.builder_input)
            missing.pop("expected_raw_gas_by_key")
            invalid_builders.append(("missing", missing))
            extra = copy.deepcopy(fixture.builder_input)
            extra["expected_unknown_observation"] = {}
            invalid_builders.append(("extra", extra))
            type_drift = copy.deepcopy(fixture.builder_input)
            type_drift["expected_raw_gas_by_key"] = {"opcode:0x60": "3"}
            invalid_builders.append(("type", type_drift))
            root_type_drift = copy.deepcopy(fixture.builder_input)
            root_type_drift["expected_final_state_root"] = 3
            invalid_builders.append(("root-type", root_type_drift))
            hash_type_drift = copy.deepcopy(fixture.builder_input)
            hash_type_drift["expected_backend_input_sha256"] = 10
            invalid_builders.append(("hash-type", hash_type_drift))
            for label, invalid_builder in invalid_builders:
                invalid_fixture = production.ProductionContextFixtureRequest(
                    scenario=fixture.scenario,
                    split=fixture.split,
                    count=fixture.count,
                    lane=fixture.lane,
                    repeat_index=fixture.repeat_index,
                    workload_id=fixture.workload_id,
                    row_id=fixture.row_id,
                    builder_input=invalid_builder,
                )
                invalid_run = root / f"invalid-{label}"
                with self.subTest(label=label), self.assertRaisesRegex(
                    ValueError, "prepared row semantic fields"
                ):
                    production.prepare_production_context_run(
                        manifest=manifest,
                        row_requests=(invalid_fixture,),
                        run=invalid_run,
                        launcher=launcher,
                        production_elf=elf,
                        production_vk=vk,
                        trace_source=trace,
                        implementation_revision="1" * 40,
                        source_hashes={"operation_coverage_v5": "2" * 64},
                        parity_identity=self._parity_identity(
                            row_id=fixture.row_id, launcher=launcher, elf=elf
                        ),
                    )
                self.assertFalse(invalid_run.exists())
            substituted = self._parity_identity(
                row_id=fixture.row_id, launcher=launcher, elf=elf
            )
            substituted["equal_values"]["host_trace_sha256"] = "c" * 64
            substituted_without_hash = dict(substituted)
            substituted_without_hash.pop("identity_sha256")
            substituted["identity_sha256"] = production.sha256_bytes(
                production.canonical_json(substituted_without_hash)
            )
            with self.assertRaisesRegex(ValueError, "prepared row evidence join"):
                production.prepare_production_context_run(
                    manifest=manifest,
                    row_requests=(fixture,),
                    run=root / "substituted-parity",
                    launcher=launcher,
                    production_elf=elf,
                    production_vk=vk,
                    trace_source=trace,
                    implementation_revision="1" * 40,
                    source_hashes={"operation_coverage_v5": "2" * 64},
                    parity_identity=substituted,
                )
            production.prepare_production_context_run(
                manifest=manifest,
                row_requests=(fixture,),
                run=run,
                launcher=launcher,
                production_elf=elf,
                production_vk=vk,
                trace_source=trace,
                implementation_revision="1" * 40,
                source_hashes={"operation_coverage_v5": "2" * 64},
                parity_identity=self._parity_identity(
                    row_id=fixture.row_id, launcher=launcher, elf=elf
                ),
            )
            with self.assertRaisesRegex(ValueError, "already exists"):
                production.prepare_production_context_run(
                    manifest=manifest,
                    row_requests=(fixture,),
                    run=run,
                    launcher=launcher,
                    production_elf=elf,
                    production_vk=vk,
                    trace_source=trace,
                    implementation_revision="1" * 40,
                    source_hashes={"operation_coverage_v5": "2" * 64},
                    parity_identity=self._parity_identity(
                        row_id=fixture.row_id, launcher=launcher, elf=elf
                    ),
                )

            report = {
                "stage": "controlled-block",
                "mode": "execute",
                "gas": 123,
                "public_values": "0x1234",
                "exit_code": 0,
                "sp1_execution_engine": "gas-estimator",
                "sp1_proposal_elf_sha256": production._sha256_file(elf),
                "guest_launcher_sha256": production._sha256_file(launcher),
                "guest_input_sha256": "0x" + "a" * 64,
                "controlled_block": {
                    "status": "accepted",
                    "row_id": fixture.row_id,
                    "observation": {
                        "backend_input_sha256": "a" * 64,
                        "host_trace_sha256": "b" * 64,
                        "public_output": "0x1234",
                        "actual_final_state_root": "0x" + "00" * 32,
                        "actual_raw_gas_by_key": {},
                        "actual_operation_event_count_by_key": {},
                        "actual_context_features": {},
                        "actual_features": {},
                        "actual_diagnostics": {},
                    },
                },
            }
            calls = []

            def execute(_row_input):
                calls.append(_row_input["row_id"])
                return report

            with mock.patch.object(
                production,
                "_require_current_implementation_revision",
                side_effect=ValueError("production context implementation revision differs"),
            ), self.assertRaisesRegex(ValueError, "revision differs"):
                production.run_production_context_campaign(run, executor=execute)
            self.assertEqual(calls, [])

            row_input = production._load_canonical_json(
                run / "row-inputs" / f"{fixture.row_id}.json", label="test row"
            )
            missing_provenance = copy.deepcopy(report)
            del missing_provenance["sp1_proposal_elf_sha256"]
            with self.assertRaisesRegex(ValueError, "incomplete"):
                production._normalize_execution_report(
                    row_input,
                    missing_provenance,
                    production_elf_sha256=production._sha256_file(elf),
                    guest_launcher_sha256=production._sha256_file(launcher),
                )
            wrong_provenance = copy.deepcopy(report)
            wrong_provenance["guest_launcher_sha256"] = "f" * 64
            with self.assertRaisesRegex(ValueError, "not accepted"):
                production._normalize_execution_report(
                    row_input,
                    wrong_provenance,
                    production_elf_sha256=production._sha256_file(elf),
                    guest_launcher_sha256=production._sha256_file(launcher),
                )

            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ):
                production.run_production_context_campaign(run, executor=execute)
                production.run_production_context_campaign(run, executor=execute)
            self.assertEqual(calls, [fixture.row_id])
            self.assertTrue((run / "execution-complete.json").is_file())
            row_path = run / "rows" / f"{fixture.row_id}.json"
            original_row = row_path.read_bytes()
            relabeled = json.loads(original_row)
            relabeled["scenario"] = "caller_canonical"
            relabeled_unhashed = dict(relabeled)
            relabeled_unhashed.pop("evidence_sha256")
            relabeled["evidence_sha256"] = production.sha256_bytes(
                production.canonical_json(relabeled_unhashed)
            )
            row_path.write_bytes(production.canonical_json(relabeled) + b"\n")
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ), self.assertRaisesRegex(ValueError, "prepared-row join"):
                production.run_production_context_campaign(run, executor=execute)
            row_path.write_bytes(original_row)
            swapped = json.loads(original_row)
            swapped["lane"] = "control" if fixture.lane == "target" else "target"
            swapped["repeat_index"] = (fixture.repeat_index + 1) % 3
            swapped_unhashed = dict(swapped)
            swapped_unhashed.pop("evidence_sha256")
            swapped["evidence_sha256"] = production.sha256_bytes(
                production.canonical_json(swapped_unhashed)
            )
            row_path.write_bytes(production.canonical_json(swapped) + b"\n")
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ), self.assertRaisesRegex(ValueError, "prepared-row join"):
                production.run_production_context_campaign(run, executor=execute)
            row_path.write_bytes(original_row)
            forged = json.loads(original_row)
            forged["guest_launcher_sha256"] = "f" * 64
            forged_unhashed = dict(forged)
            forged_unhashed.pop("evidence_sha256")
            forged["evidence_sha256"] = production.sha256_bytes(
                production.canonical_json(forged_unhashed)
            )
            row_path.write_bytes(production.canonical_json(forged) + b"\n")
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ), self.assertRaisesRegex(ValueError, "prepared-row join"):
                production.run_production_context_campaign(run, executor=execute)
            row_path.write_bytes(original_row)
            launcher.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "launcher hash"):
                with mock.patch.object(
                    production, "_require_current_implementation_revision"
                ):
                    production.run_production_context_campaign(run, executor=execute)

    def test_prepare_rejects_holdout_row_as_parity_authority(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        final_fixture = next(
            fixture
            for fixture in production.production_context_fixture_requests(manifest)
            if fixture.scenario == "address_alternate"
            and fixture.count == 32
            and fixture.lane == "target"
            and fixture.repeat_index == 0
        )
        final_fixture = self._frozen_fixture(final_fixture)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            launcher = root / "guest-launcher"
            elf = root / "proposal.elf"
            vk = root / "proposal.vk"
            trace = root / "reconstruct.rs"
            for path, value in (
                (launcher, b"launcher"),
                (elf, b"elf"),
                (vk, b"vk"),
                (trace, b"trace"),
            ):
                path.write_bytes(value)
            with self.assertRaisesRegex(ValueError, "parity row semantics"):
                production.prepare_production_context_run(
                    manifest=manifest,
                    row_requests=(final_fixture,),
                    run=root / "run",
                    launcher=launcher,
                    production_elf=elf,
                    production_vk=vk,
                    trace_source=trace,
                    implementation_revision="1" * 40,
                    source_hashes={"operation_coverage_v5": "2" * 64},
                    parity_identity=self._parity_identity(
                        row_id=final_fixture.row_id,
                        launcher=launcher,
                        elf=elf,
                    ),
                )
            self.assertFalse((root / "run").exists())

    def test_run_and_fit_reject_rehashed_frozen_observation_substitution(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        source = self._parity_fixture(manifest)
        builder = copy.deepcopy(source.builder_input)
        builder.update(
            {
                "expected_final_state_root": "0x" + "3" * 64,
                "expected_raw_gas_by_key": {"opcode:0x60": 3},
                "expected_operation_event_count_by_key": {"opcode:0x60": 1},
                "expected_context_features": {"context_fixed:opcode:0x30": 1},
                "expected_features": {
                    "proposal_startup": 1,
                    "block_base": 1,
                    "tx_base": 1,
                    "native_value_transfer": 0,
                },
                "expected_diagnostics": {"bytecode_length": 256},
                "expected_backend_input_sha256": "a" * 64,
                "expected_host_trace_sha256": "b" * 64,
            }
        )
        fixture = production.ProductionContextFixtureRequest(
            scenario=source.scenario,
            split=source.split,
            count=source.count,
            lane=source.lane,
            repeat_index=source.repeat_index,
            workload_id=source.workload_id,
            row_id=source.row_id,
            builder_input=builder,
        )
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            launcher = root / "guest-launcher"
            elf = root / "proposal.elf"
            vk = root / "proposal.vk"
            trace = root / "reconstruct.rs"
            for path in (launcher, elf, vk, trace):
                path.write_bytes(path.name.encode())
            run = root / "run"
            production.prepare_production_context_run(
                manifest=manifest,
                row_requests=(fixture,),
                run=run,
                launcher=launcher,
                production_elf=elf,
                production_vk=vk,
                trace_source=trace,
                implementation_revision="1" * 40,
                source_hashes={"operation_coverage_v5": "2" * 64},
                parity_identity=self._parity_identity(
                    row_id=fixture.row_id, launcher=launcher, elf=elf
                ),
            )
            row_input = production._load_canonical_json(
                run / "row-inputs" / f"{fixture.row_id}.json",
                label="test prepared row",
            )
            report = {
                "stage": "controlled-block",
                "mode": "execute",
                "gas": 123,
                "public_values": "0x1234",
                "exit_code": 0,
                "sp1_execution_engine": "gas-estimator",
                "sp1_proposal_elf_sha256": production._sha256_file(elf),
                "guest_launcher_sha256": production._sha256_file(launcher),
                "guest_input_sha256": "0x" + "a" * 64,
                "controlled_block": {
                    "status": "accepted",
                    "row_id": fixture.row_id,
                    "observation": {
                        "backend_input_sha256": "a" * 64,
                        "host_trace_sha256": "b" * 64,
                        "public_output": "0x1234",
                        "actual_final_state_root": "0x" + "3" * 64,
                        "actual_raw_gas_by_key": {"opcode:0x60": 3},
                        "actual_operation_event_count_by_key": {"opcode:0x60": 1},
                        "actual_context_features": {
                            "context_fixed:opcode:0x30": 1
                        },
                        "actual_features": dict(builder["expected_features"]),
                        "actual_diagnostics": {"bytecode_length": 256},
                    },
                },
            }
            valid = production._normalize_execution_report(
                row_input,
                report,
                production_elf_sha256=production._sha256_file(elf),
                guest_launcher_sha256=production._sha256_file(launcher),
            )
            forged = copy.deepcopy(valid)
            forged.update(
                {
                    "actual_final_state_root": "0x" + "4" * 64,
                    "actual_raw_gas_by_key": {"opcode:0x61": 6},
                    "actual_operation_event_count_by_key": {"opcode:0x61": 2},
                    "actual_context_features": {"context_fixed:opcode:0x33": 1},
                    "actual_features": {
                        **builder["expected_features"],
                        "block_base": 2,
                    },
                    "actual_diagnostics": {"bytecode_length": 257},
                }
            )
            forged_unhashed = dict(forged)
            forged_unhashed.pop("evidence_sha256")
            forged["evidence_sha256"] = production.sha256_bytes(
                production.canonical_json(forged_unhashed)
            )
            production._write_json_create_only(
                run / "rows" / f"{fixture.row_id}.json", forged
            )
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ), self.assertRaisesRegex(ValueError, "frozen observation"):
                production.run_production_context_campaign(run)
            self.assertFalse((run / "execution-complete.json").exists())

            identity = production._load_canonical_json(
                run / "calibration-identity.json", label="test identity"
            )
            completion = {
                "schema_version": 1,
                "status": "execution_complete",
                "identity_sha256": identity["identity_sha256"],
                "row_hashes": [
                    {
                        "row_id": fixture.row_id,
                        "evidence_sha256": forged["evidence_sha256"],
                    }
                ],
            }
            completion["terminal_sha256"] = production.sha256_bytes(
                production.canonical_json(completion)
            )
            production._write_json_create_only(
                run / "execution-complete.json", completion
            )
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ), self.assertRaisesRegex(ValueError, "frozen observation"):
                production.fit_production_context_run(
                    run,
                    manifest=manifest,
                    subtotal_model=self._subtotal_model(),
                )
            self.assertFalse((run / "campaign-decisions.json").exists())
            self.assertFalse((run / "terminal.json").exists())

    def test_subprocess_failure_preserves_rows_without_terminal(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        all_fixtures = production.production_context_fixture_requests(manifest)
        parity_fixture = self._parity_fixture(manifest)
        fixtures = (
            self._frozen_fixture(parity_fixture),
            self._frozen_fixture(next(row for row in all_fixtures if row != parity_fixture)),
        )
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            files = []
            for name in ("guest-launcher", "proposal.elf", "proposal.vk", "reconstruct.rs"):
                path = root / name
                path.write_bytes(name.encode())
                files.append(path)
            run = root / "run"
            production.prepare_production_context_run(
                manifest=manifest,
                row_requests=fixtures,
                run=run,
                launcher=files[0],
                production_elf=files[1],
                production_vk=files[2],
                trace_source=files[3],
                implementation_revision="1" * 40,
                source_hashes={"operation_coverage_v5": "2" * 64},
                parity_identity=self._parity_identity(
                    row_id=fixtures[0].row_id, launcher=files[0], elf=files[1]
                ),
            )
            calls = 0

            def execute(row_input):
                nonlocal calls
                calls += 1
                if calls == 2:
                    raise subprocess.TimeoutExpired(["guest-launcher"], 10)
                return {
                    "stage": "controlled-block",
                    "mode": "execute",
                    "gas": 1,
                    "public_values": "0x01",
                    "exit_code": 0,
                    "sp1_execution_engine": "gas-estimator",
                    "sp1_proposal_elf_sha256": production._sha256_file(files[1]),
                    "guest_launcher_sha256": production._sha256_file(files[0]),
                    "guest_input_sha256": "0x" + "a" * 64,
                    "controlled_block": {
                        "status": "accepted",
                        "row_id": row_input["row_id"],
                        "observation": {
                            "backend_input_sha256": "a" * 64,
                            "host_trace_sha256": "b" * 64,
                            "public_output": "0x01",
                            "actual_final_state_root": "0x" + "00" * 32,
                            "actual_raw_gas_by_key": {},
                            "actual_operation_event_count_by_key": {},
                            "actual_context_features": {},
                            "actual_features": {},
                            "actual_diagnostics": {},
                        },
                    },
                }

            with self.assertRaises(subprocess.TimeoutExpired):
                with mock.patch.object(
                    production, "_require_current_implementation_revision"
                ):
                    production.run_production_context_campaign(run, executor=execute)
            self.assertEqual(len(list((run / "rows").glob("*.json"))), 1)
            self.assertFalse((run / "execution-complete.json").exists())
            self.assertFalse((run / "campaign-decisions.json").exists())
            self.assertFalse((run / "terminal.json").exists())

    def test_fit_recovers_terminal_from_valid_existing_decisions(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        fixture = self._frozen_fixture(
            self._parity_fixture(manifest)
        )
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            launcher = root / "guest-launcher"
            elf = root / "proposal.elf"
            vk = root / "proposal.vk"
            trace = root / "reconstruct.rs"
            for path in (launcher, elf, vk, trace):
                path.write_bytes(path.name.encode())
            run = root / "run"
            production.prepare_production_context_run(
                manifest=manifest,
                row_requests=(fixture,),
                run=run,
                launcher=launcher,
                production_elf=elf,
                production_vk=vk,
                trace_source=trace,
                implementation_revision="1" * 40,
                source_hashes={"operation_coverage_v5": "2" * 64},
                parity_identity=self._parity_identity(
                    row_id=fixture.row_id, launcher=launcher, elf=elf
                ),
            )
            report = {
                "stage": "controlled-block",
                "mode": "execute",
                "gas": 1,
                "public_values": "0x01",
                "exit_code": 0,
                "sp1_execution_engine": "gas-estimator",
                "sp1_proposal_elf_sha256": production._sha256_file(elf),
                "guest_launcher_sha256": production._sha256_file(launcher),
                "guest_input_sha256": "0x" + "a" * 64,
                "controlled_block": {
                    "status": "accepted",
                    "row_id": fixture.row_id,
                    "observation": {
                        "backend_input_sha256": "a" * 64,
                        "host_trace_sha256": "b" * 64,
                        "public_output": "0x01",
                        "actual_final_state_root": "0x" + "00" * 32,
                        "actual_raw_gas_by_key": {},
                        "actual_operation_event_count_by_key": {},
                        "actual_context_features": {},
                        "actual_features": {},
                        "actual_diagnostics": {},
                    },
                },
            }
            decisions = {
                "schema_version": 1,
                "purpose": "production_context_fit_decisions",
                "manifest_identity_sha256": manifest.identity_sha256,
                "families": {},
                "all_families_accepted": True,
            }
            decisions["decision_sha256"] = production.sha256_bytes(
                production.canonical_json(decisions)
            )
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ):
                production.run_production_context_campaign(
                    run, executor=lambda _row: report
                )
            with mock.patch.object(
                production,
                "_require_current_implementation_revision",
                side_effect=ValueError("production context implementation revision differs"),
            ), self.assertRaisesRegex(ValueError, "revision differs"):
                production.fit_production_context_run(
                    run, manifest=manifest, subtotal_model=self._subtotal_model()
                )
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ):
                with mock.patch.object(
                    production,
                    "fit_production_context_rows",
                    return_value=decisions,
                ):
                    first = production.fit_production_context_run(
                        run, manifest=manifest, subtotal_model=self._subtotal_model()
                    )
                    (run / "terminal.json").unlink()
                    recovered = production.fit_production_context_run(
                        run, manifest=manifest, subtotal_model=self._subtotal_model()
                    )
            self.assertEqual(recovered, first)
            self.assertTrue((run / "terminal.json").is_file())
            tampered = json.loads((run / "campaign-decisions.json").read_text())
            tampered["all_families_accepted"] = False
            (run / "campaign-decisions.json").write_bytes(
                production.canonical_json(tampered) + b"\n"
            )
            with mock.patch.object(
                production, "_require_current_implementation_revision"
            ), mock.patch.object(
                production,
                "fit_production_context_rows",
                return_value=decisions,
            ), self.assertRaisesRegex(ValueError, "existing decisions differ"):
                production.fit_production_context_run(
                    run, manifest=manifest, subtotal_model=self._subtotal_model()
                )

    def test_atomic_create_only_never_exposes_partial_json(self):
        payload = {"schema_version": 1, "value": "x" * 100}
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            output = root / "result.json"
            real_write = production.os.write

            def partial_write(descriptor, data):
                return real_write(descriptor, data[:3])

            with mock.patch.object(production.os, "write", side_effect=partial_write):
                production._write_json_create_only(output, payload)
            self.assertEqual(output.read_bytes(), production.canonical_json(payload) + b"\n")
            self.assertEqual(list(root.glob(".*.tmp")), [])

            for failure in (
                OSError(errno.ENOSPC, "no space"),
                RuntimeError("injected pre-publication crash"),
            ):
                candidate = root / f"failure-{type(failure).__name__}.json"
                with mock.patch.object(
                    production.os,
                    "write" if isinstance(failure, OSError) else "fsync",
                    side_effect=failure,
                ), self.assertRaises(type(failure)):
                    production._write_json_create_only(candidate, payload)
                self.assertFalse(candidate.exists())
                self.assertEqual(list(root.glob(f".{candidate.name}.*.tmp")), [])

    def test_resume_requires_exact_clean_implementation_revision(self):
        recorded = "1" * 40

        def completed(command, stdout):
            return subprocess.CompletedProcess(command, 0, stdout, "")

        with mock.patch.object(
            production.subprocess,
            "run",
            side_effect=[
                completed(["git", "rev-parse", "HEAD"], "2" * 40 + "\n"),
                completed(["git", "status"], ""),
            ],
        ), self.assertRaisesRegex(ValueError, "revision differs"):
            production._require_current_implementation_revision(recorded)
        with mock.patch.object(
            production.subprocess,
            "run",
            side_effect=[
                completed(["git", "rev-parse", "HEAD"], recorded + "\n"),
                completed(["git", "status"], " M changed.py\n"),
            ],
        ), self.assertRaisesRegex(ValueError, "worktree is dirty"):
            production._require_current_implementation_revision(recorded)
        with mock.patch.object(
            production.subprocess,
            "run",
            side_effect=FileNotFoundError("git"),
        ), self.assertRaisesRegex(ValueError, "unavailable"):
            production._require_current_implementation_revision(recorded)


class ProductionContextSealTests(unittest.TestCase):
    def test_seal_source_check_rejects_an_explicit_helper_hash_mismatch(self):
        manifest = production.load_production_context_manifest(MANIFEST)
        discovery = ROOT / manifest.sources["discovery"]["path"]
        source_identity = json.loads(
            (discovery.parent / "source-identity.json").read_bytes()
        )
        helper = ROOT / source_identity["launcher_path"]
        helper_mismatch = b"explicit-test-helper-hash-mismatch"
        self.assertNotEqual(
            production.sha256_bytes(helper_mismatch),
            source_identity["launcher_sha256"],
        )
        real_read_bytes = pathlib.Path.read_bytes

        def read_with_helper_mismatch(path):
            if path == helper:
                return helper_mismatch
            return real_read_bytes(path)

        with mock.patch.object(
            pathlib.Path,
            "read_bytes",
            autospec=True,
            side_effect=read_with_helper_mismatch,
        ), self.assertRaisesRegex(
            ValueError, "native identity helper differs from sealed calibration"
        ):
            production.validate_production_context_sources(manifest, ROOT)

    def _build_run(self, root):
        manifest = production.load_production_context_manifest(MANIFEST)
        fit_helpers = ProductionContextFitTests()
        rows_by_identity = {
            (
                row["scenario"],
                row["count"],
                row["lane"],
                row["repeat_index"],
            ): row
            for row in fit_helpers._complete_rows()
        }
        launcher = root / "guest-launcher"
        elf = root / "proposal.elf"
        vk = root / "proposal.vk"
        trace = root / "reconstruct.rs"
        for path, data in (
            (launcher, b"launcher"),
            (elf, b"elf"),
            (vk, b"vk"),
            (trace, b"trace"),
        ):
            path.write_bytes(data)
        run = root / "run"
        (run / "row-inputs").mkdir(parents=True)
        (run / "rows").mkdir()
        entries = []
        evidence_rows = []
        for request in production.production_context_fixture_requests(manifest):
            measured = copy.deepcopy(
                rows_by_identity[
                    (
                        request.scenario,
                        request.count,
                        request.lane,
                        request.repeat_index,
                    )
                ]
            )
            builder = copy.deepcopy(request.builder_input)
            builder.update(
                {
                    "expected_final_state_root": "0x" + "0" * 64,
                    "expected_raw_gas_by_key": measured["actual_raw_gas_by_key"],
                    "expected_operation_event_count_by_key": measured[
                        "actual_operation_event_count_by_key"
                    ],
                    "expected_context_features": measured[
                        "actual_context_features"
                    ],
                    "expected_features": measured["actual_features"],
                    "expected_diagnostics": measured["actual_diagnostics"],
                    "expected_backend_input_sha256": measured[
                        "backend_input_sha256"
                    ],
                    "expected_host_trace_sha256": measured["host_trace_sha256"],
                }
            )
            row_input = {
                "row_id": request.row_id,
                "workload_id": request.workload_id,
                "scenario": request.scenario,
                "split": request.split,
                "count": request.count,
                "lane": request.lane,
                "repeat_index": request.repeat_index,
                "builder_input": builder,
            }
            row_input["input_sha256"] = production.sha256_bytes(
                production.canonical_json(row_input)
            )
            input_path = run / "row-inputs" / f"{request.row_id}.json"
            input_path.write_bytes(production.canonical_json(row_input) + b"\n")
            entries.append(
                {
                    "row_id": request.row_id,
                    "path": f"row-inputs/{request.row_id}.json",
                    "input_sha256": row_input["input_sha256"],
                }
            )
            evidence = {
                "row_id": request.row_id,
                "input_sha256": row_input["input_sha256"],
                "workload_id": request.workload_id,
                "scenario": request.scenario,
                "split": request.split,
                "count": request.count,
                "lane": request.lane,
                "repeat_index": request.repeat_index,
                "prover_gas": measured["prover_gas"],
                "public_output": measured["public_output"],
                "backend_input_sha256": measured["backend_input_sha256"],
                "host_trace_sha256": measured["host_trace_sha256"],
                "sp1_proposal_elf_sha256": production._sha256_file(elf),
                "guest_launcher_sha256": production._sha256_file(launcher),
                "actual_final_state_root": builder["expected_final_state_root"],
                "actual_raw_gas_by_key": measured["actual_raw_gas_by_key"],
                "actual_operation_event_count_by_key": measured[
                    "actual_operation_event_count_by_key"
                ],
                "actual_context_features": measured["actual_context_features"],
                "actual_features": measured["actual_features"],
                "actual_diagnostics": measured["actual_diagnostics"],
            }
            evidence["evidence_sha256"] = production.sha256_bytes(
                production.canonical_json(evidence)
            )
            (run / "rows" / f"{request.row_id}.json").write_bytes(
                production.canonical_json(evidence) + b"\n"
            )
            evidence_rows.append(evidence)
        revision = "1" * 40
        parity_row = next(
            row
            for row in evidence_rows
            if {
                "scenario": row["scenario"],
                "split": row["split"],
                "count": row["count"],
                "lane": row["lane"],
                "repeat_index": row["repeat_index"],
            }
            == production.PARITY_ROW_CONTRACT
        )
        parity = fit_helpers._parity_identity(
            row_id=parity_row["row_id"], launcher=launcher, elf=elf
        )
        identity = {
            "schema_version": 1,
            "purpose": "production_context_run_identity",
            "manifest_identity_sha256": manifest.identity_sha256,
            "implementation_revision": revision,
            "source_hashes": {
                name: source["file_sha256"]
                for name, source in manifest.sources.items()
            },
            "assets": {
                "launcher": {
                    "basename": launcher.name,
                    "sha256": production._sha256_file(launcher),
                },
                "production_elf": {
                    "basename": elf.name,
                    "sha256": production._sha256_file(elf),
                },
                "production_vk": {
                    "basename": vk.name,
                    "sha256": production._sha256_file(vk),
                },
                "trace_source": {
                    "basename": trace.name,
                    "sha256": production._sha256_file(trace),
                },
            },
            "parity_identity": parity,
            "rows": entries,
        }
        identity["identity_sha256"] = production.sha256_bytes(
            production.canonical_json(identity)
        )
        (run / "calibration-identity.json").write_bytes(
            production.canonical_json(identity) + b"\n"
        )
        completion = {
            "schema_version": 1,
            "status": "execution_complete",
            "identity_sha256": identity["identity_sha256"],
            "row_hashes": [
                {
                    "row_id": row["row_id"],
                    "evidence_sha256": row["evidence_sha256"],
                }
                for row in evidence_rows
            ],
        }
        completion["terminal_sha256"] = production.sha256_bytes(
            production.canonical_json(completion)
        )
        (run / "execution-complete.json").write_bytes(
            production.canonical_json(completion) + b"\n"
        )
        subtotal = production.load_production_v5_subtotal_model(manifest, ROOT)
        decisions = production.fit_production_context_rows(
            manifest, evidence_rows, subtotal
        )
        (run / "campaign-decisions.json").write_bytes(
            production.canonical_json(decisions) + b"\n"
        )
        terminal = {
            "schema_version": 1,
            "status": "accepted" if decisions["all_families_accepted"] else "rejected",
            "identity_sha256": identity["identity_sha256"],
            "decision_sha256": decisions["decision_sha256"],
        }
        terminal["terminal_sha256"] = production.sha256_bytes(
            production.canonical_json(terminal)
        )
        (run / "terminal.json").write_bytes(
            production.canonical_json(terminal) + b"\n"
        )
        return manifest, run, revision

    @staticmethod
    def _reseal_result_directory(candidate):
        result_path = candidate / "result.json"
        result = json.loads(result_path.read_bytes())
        calibration = json.loads((candidate / "calibration-identity.json").read_bytes())
        result["result_identity"]["calibration_identity_sha256"] = calibration[
            "identity_sha256"
        ]
        result["result_identity"]["implementation_revision"] = calibration[
            "implementation_revision"
        ]
        result["result_identity"]["file_sha256s"] = {
            name: production.sha256_bytes((candidate / name).read_bytes())
            for name in production.PRODUCTION_CONTEXT_RESULT_INVENTORY
            if name != "result.json"
        }
        identity = production.sha256_bytes(
            production.canonical_json(result["result_identity"])
        )
        result["result_identity_sha256"] = identity
        result["result_id"] = identity[:24]
        result.pop("artifact_sha256")
        result["artifact_sha256"] = production.sha256_bytes(
            production.canonical_json(result)
        )
        result_path.write_bytes(production.canonical_json(result) + b"\n")
        renamed = candidate.with_name(identity[:24])
        candidate.rename(renamed)
        return renamed

    def test_seal_is_create_only_and_replay_uses_result_plus_source_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            _manifest, run, revision = self._build_run(root)
            with self.assertRaisesRegex(ValueError, "overlap"):
                production.seal_production_context_result(
                    run=run,
                    manifest_path=MANIFEST,
                    out_root=run / "result",
                    repo_root=ROOT,
                )
            out_root = root / "results"
            with mock.patch.object(
                production,
                "_require_current_implementation_revision",
                return_value=revision,
            ), mock.patch.object(
                discovery_campaign,
                "_validated_context_identity_helper",
                return_value=ROOT / "target/release/guest-launcher",
            ):
                sealed = production.seal_production_context_result(
                    run=run,
                    manifest_path=MANIFEST,
                    out_root=out_root,
                    repo_root=ROOT,
                )
            result_dir = pathlib.Path(sealed["directory"])
            sealed_files = {
                path.name: path.read_bytes() for path in result_dir.iterdir()
            }
            with self.assertRaisesRegex(ValueError, "already exists"):
                production._publish_production_context_result(
                    out_root, sealed["result_id"], sealed_files
                )
            self.assertEqual(
                {path.name for path in result_dir.iterdir()},
                production.PRODUCTION_CONTEXT_RESULT_INVENTORY,
            )
            run.rename(root / "removed-run")
            with mock.patch.object(
                production,
                "_require_current_implementation_revision",
                return_value=revision,
            ), mock.patch.object(
                production.subprocess,
                "run",
                side_effect=AssertionError("portable replay invoked an unexpected subprocess"),
            ):
                replay = production.verify_production_context_result(result_dir)
            self.assertEqual(replay["result_id"], sealed["result_id"])
            self.assertEqual(replay["family_statuses"], sealed["family_statuses"])

    def test_atomic_publication_cleans_up_after_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            files = {
                name: b"{}\n"
                for name in production.PRODUCTION_CONTEXT_RESULT_INVENTORY
            }
            with mock.patch.object(
                production.os, "write", side_effect=OSError("injected write failure")
            ), self.assertRaises(OSError):
                production._publish_production_context_result(
                    root, "a" * 24, files
            )
            self.assertFalse((root / ("a" * 24)).exists())
            self.assertEqual([path for path in root.iterdir() if path.is_dir()], [])

    def test_bounded_readers_reject_same_size_in_place_mutation(self):
        real_read = os.read
        real_fstat = os.fstat
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            for helper in (
                production._read_bounded_regular_file,
                production._sha256_bounded_regular_file,
            ):
                for attempt in range(8):
                    with self.subTest(helper=helper.__name__, attempt=attempt):
                        path = root / f"{helper.__name__}-{attempt}.bin"
                        path.write_bytes(b"a" * (1024 * 1024 + 16))
                        mutated = False
                        frozen_times = {}

                        def mutate_after_first_chunk(descriptor, size):
                            nonlocal mutated
                            chunk = real_read(descriptor, size)
                            if not mutated:
                                mutated = True
                                with path.open("r+b") as target:
                                    target.seek(-1, os.SEEK_END)
                                    target.write(b"b")
                                    target.flush()
                                    os.fsync(target.fileno())
                            return chunk

                        def stable_timestamps(descriptor):
                            current = real_fstat(descriptor)
                            mtime_ns, ctime_ns = frozen_times.setdefault(
                                descriptor, (current.st_mtime_ns, current.st_ctime_ns)
                            )
                            return types.SimpleNamespace(
                                st_dev=current.st_dev,
                                st_ino=current.st_ino,
                                st_mode=current.st_mode,
                                st_nlink=current.st_nlink,
                                st_size=current.st_size,
                                st_mtime_ns=mtime_ns,
                                st_ctime_ns=ctime_ns,
                            )

                        with mock.patch.object(
                            production.os, "read", side_effect=mutate_after_first_chunk
                        ), mock.patch.object(
                            production.os, "fstat", side_effect=stable_timestamps
                        ), self.assertRaisesRegex(ValueError, "changed while it was read"):
                            helper(path, 2 * 1024 * 1024, label="mutating test file")

    def test_bounded_readers_reject_truncated_grown_and_shrunk_files(self):
        real_read = os.read
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            for helper in (
                production._read_bounded_regular_file,
                production._sha256_bounded_regular_file,
            ):
                empty = root / f"{helper.__name__}-empty.bin"
                empty.touch()
                with self.subTest(helper=helper.__name__, mutation="truncated"), self.assertRaisesRegex(
                    ValueError, "size limit"
                ):
                    helper(empty, 2 * 1024 * 1024, label="truncated test file")

                for mutation in ("grown", "shrunk"):
                    with self.subTest(helper=helper.__name__, mutation=mutation):
                        path = root / f"{helper.__name__}-{mutation}.bin"
                        path.write_bytes(b"a" * (1024 * 1024 + 16))
                        changed = False

                        def change_after_first_chunk(descriptor, size):
                            nonlocal changed
                            chunk = real_read(descriptor, size)
                            if not changed:
                                changed = True
                                if mutation == "grown":
                                    with path.open("ab") as target:
                                        target.write(b"b")
                                else:
                                    with path.open("r+b") as target:
                                        target.truncate(1024 * 1024)
                            return chunk

                        with mock.patch.object(
                            production.os, "read", side_effect=change_after_first_chunk
                        ), self.assertRaisesRegex(ValueError, "changed while it was read"):
                            helper(path, 2 * 1024 * 1024, label=f"{mutation} test file")

    def test_bounded_readers_fail_closed_on_seek_or_second_pass_failure(self):
        real_read = os.read
        real_lseek = os.lseek
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            for helper in (
                production._read_bounded_regular_file,
                production._sha256_bounded_regular_file,
            ):
                path = root / f"{helper.__name__}.bin"
                path.write_bytes(b"bounded source")
                with self.subTest(helper=helper.__name__, failure="seek"), mock.patch.object(
                    production.os, "lseek", side_effect=OSError("seek unavailable")
                ), self.assertRaisesRegex(ValueError, "reread is unavailable"):
                    helper(path, 1024, label="seek failure test file")

                second_pass = False

                def begin_second_pass(descriptor, offset, whence):
                    nonlocal second_pass
                    second_pass = True
                    return real_lseek(descriptor, offset, whence)

                def fail_second_pass(descriptor, size):
                    if second_pass:
                        raise OSError("second pass failed")
                    return real_read(descriptor, size)

                with self.subTest(helper=helper.__name__, failure="second-pass"), mock.patch.object(
                    production.os, "lseek", side_effect=begin_second_pass
                ), mock.patch.object(
                    production.os, "read", side_effect=fail_second_pass
                ), self.assertRaisesRegex(ValueError, "reread is unavailable"):
                    helper(path, 1024, label="second-pass failure test file")

    def test_mutation_watch_setup_failure_closes_descriptor(self):
        watcher = os.open("/dev/null", os.O_RDONLY)
        libc = mock.Mock()
        libc.inotify_init1 = mock.Mock(return_value=watcher)
        libc.inotify_add_watch = mock.Mock(return_value=-1)
        ctypes_errno = errno.ENOSPC
        try:
            with mock.patch.object(
                production.ctypes, "CDLL", return_value=libc
            ), mock.patch.object(
                production.ctypes, "get_errno", return_value=ctypes_errno
            ), self.assertRaisesRegex(ValueError, "watch is unavailable"):
                production._establish_linux_mutation_watch(
                    pathlib.Path("unwatchable"), label="watch setup test file"
                )
            with self.assertRaises(OSError):
                os.fstat(watcher)
        finally:
            try:
                os.close(watcher)
            except OSError:
                pass

    def test_mutation_watch_rejects_read_overflow_invalidation_and_remove_failure(self):
        watch_id = 17
        cases = (
            ("read", OSError("watch read failed")),
            ("overflow", [(-1, 0x00004000)]),
            ("invalidation", [(watch_id, 0x00008000)]),
        )
        for label, outcome in cases:
            with self.subTest(failure=label), mock.patch.object(
                production,
                "_drain_linux_mutation_watch",
                side_effect=outcome if isinstance(outcome, OSError) else None,
                return_value=None if isinstance(outcome, OSError) else outcome,
            ), self.assertRaises(ValueError):
                production._finish_linux_mutation_watch(99, watch_id, label="watch test file")

        with mock.patch.object(
            production, "_drain_linux_mutation_watch", return_value=[]
        ), mock.patch.object(
            production,
            "_remove_linux_mutation_watch",
            side_effect=OSError("watch remove failed"),
        ), self.assertRaisesRegex(ValueError, "watch is unavailable"):
            production._finish_linux_mutation_watch(99, watch_id, label="watch test file")

    def test_bounded_readers_close_watch_after_check_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            for helper in (
                production._read_bounded_regular_file,
                production._sha256_bounded_regular_file,
            ):
                path = root / f"{helper.__name__}.bin"
                path.write_bytes(b"bounded source")
                watcher = os.open("/dev/null", os.O_RDONLY)
                try:
                    with self.subTest(helper=helper.__name__), mock.patch.object(
                        production,
                        "_establish_linux_mutation_watch",
                        return_value=(watcher, 19),
                    ), mock.patch.object(
                        production,
                        "_finish_linux_mutation_watch",
                        side_effect=ValueError("mutation watch failed"),
                    ), self.assertRaisesRegex(ValueError, "mutation watch failed"):
                        helper(path, 1024, label="watch cleanup test file")
                    with self.assertRaises(OSError):
                        os.fstat(watcher)
                finally:
                    try:
                        os.close(watcher)
                    except OSError:
                        pass

    def test_source_code_hashing_rejects_unavailable_and_unsafe_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            revision = "1" * 40
            with mock.patch.object(
                production, "PRODUCTION_CONTEXT_SEAL_SOURCE_PATHS", ("source.py",)
            ):
                with self.assertRaisesRegex(ValueError, "repository is unavailable"):
                    production._source_code_hashes(root / "missing", revision)
                with self.assertRaisesRegex(ValueError, "source code is missing"):
                    production._source_code_hashes(root, revision)

                target = root / "target.py"
                target.write_text("source")
                source = root / "source.py"
                source.symlink_to(target)
                with self.assertRaisesRegex(ValueError, "not a regular file"):
                    production._source_code_hashes(root, revision)

                source.unlink()
                source.mkdir()
                with self.assertRaisesRegex(ValueError, "size limit"):
                    production._source_code_hashes(root, revision)

                source.rmdir()
                with source.open("wb") as oversized:
                    oversized.truncate(32 * 1024 * 1024 + 1)
                with self.assertRaisesRegex(ValueError, "size limit"):
                    production._source_code_hashes(root, revision)

    def test_replay_rejects_extra_symlink_fifo_and_oversize_members(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            _manifest, run, revision = self._build_run(root)
            with mock.patch.object(
                production,
                "_require_current_implementation_revision",
                return_value=revision,
            ), mock.patch.object(
                discovery_campaign,
                "_validated_context_identity_helper",
                return_value=ROOT / "target/release/guest-launcher",
            ):
                sealed = production.seal_production_context_result(
                    run=run,
                    manifest_path=MANIFEST,
                    out_root=root / "results",
                    repo_root=ROOT,
                )
            original = pathlib.Path(sealed["directory"])
            for label, mutation in (
                ("extra", lambda path: (path / "extra.json").write_text("{}")),
                ("missing", lambda path: (path / "model-report.json").unlink()),
                (
                    "symlink",
                    lambda path: (
                        (path / "result.json").unlink(),
                        (path / "result.json").symlink_to("model-report.json"),
                    ),
                ),
                (
                    "fifo",
                    lambda path: (
                        (path / "result.json").unlink(),
                        os.mkfifo(path / "result.json"),
                    ),
                ),
                (
                    "oversize",
                    lambda path: (path / "result.json").write_bytes(
                        b"x" * (production.PRODUCTION_CONTEXT_RESULT_LIMITS["result.json"] + 1)
                    ),
                ),
            ):
                candidate = root / label
                shutil.copytree(original, candidate)
                for member in candidate.iterdir():
                    member.chmod(0o600)
                mutation(candidate)
                with self.subTest(label=label), self.assertRaises(ValueError):
                    production.verify_production_context_result(candidate)

            def mutate_rows(candidate, transform):
                path = candidate / "rows.jsonl"
                lines = path.read_bytes().splitlines(keepends=True)
                path.write_bytes(b"".join(transform(lines)))

            caller_resealed_mutations = {
                "row-deleted": lambda path: mutate_rows(path, lambda lines: lines[:-1]),
                "row-reordered": lambda path: mutate_rows(
                    path, lambda lines: [lines[1], lines[0], *lines[2:]]
                ),
                "row-duplicated": lambda path: mutate_rows(
                    path, lambda lines: [*lines, lines[-1]]
                ),
                "model": lambda path: self._mutate_json(
                    path / "model-report.json",
                    lambda payload: payload["families"]["opcode:0x30"][
                        "coefficients"
                    ].update({"address_constant": "999"}),
                ),
                "gate": lambda path: self._mutate_json(
                    path / "campaign-decisions.json",
                    lambda payload: payload["families"]["opcode:0x30"][
                        "rejection_reasons"
                    ].append("forged_gate"),
                ),
                "source-path-traversal": lambda path: self._mutate_json(
                    path / "source-coverage-v5.json",
                    lambda payload: payload.update({"source_path": "../coverage.json"}),
                ),
                "row-path-traversal": lambda path: self._mutate_calibration_identity(
                    path, lambda payload: payload["rows"][0].update({"path": "../row.json"})
                ),
                "wrong-revision": lambda path: self._mutate_calibration_identity(
                    path,
                    lambda payload: payload.update(
                        {"implementation_revision": "2" * 40}
                    ),
                ),
                "source-forgery": lambda path: self._mutate_source_bundle(path),
                "discovery-promotion": lambda path: self._mutate_discovery_bundle(path),
                "source-code-digest-forgery": lambda path: self._mutate_source_code_hashes(
                    path, revision=None
                ),
                "source-code-missing-key": lambda path: self._mutate_source_code_keys(
                    path, extra=False
                ),
                "source-code-extra-key": lambda path: self._mutate_source_code_keys(
                    path, extra=True
                ),
                "source-code-revision-forgery": lambda path: self._mutate_source_code_hashes(
                    path, revision="f" * 40
                ),
            }
            for label, mutation in caller_resealed_mutations.items():
                candidate = root / f"resealed-{label}"
                shutil.copytree(original, candidate)
                for member in candidate.iterdir():
                    member.chmod(0o600)
                mutation(candidate)
                candidate = self._reseal_result_directory(candidate)
                def require_sealed_revision(recorded):
                    if recorded != revision:
                        raise ValueError("production context implementation revision differs")
                    return recorded

                with self.subTest(label=label), mock.patch.object(
                    production,
                    "_require_current_implementation_revision",
                    side_effect=require_sealed_revision,
                ), self.assertRaises(ValueError):
                    production.verify_production_context_result(candidate)

    @staticmethod
    def _mutate_json(path, mutation):
        payload = json.loads(path.read_bytes())
        mutation(payload)
        path.write_bytes(production.canonical_json(payload) + b"\n")

    @classmethod
    def _mutate_calibration_identity(cls, candidate, mutation):
        path = candidate / "calibration-identity.json"
        payload = json.loads(path.read_bytes())
        mutation(payload)
        payload.pop("identity_sha256")
        payload["identity_sha256"] = production.sha256_bytes(
            production.canonical_json(payload)
        )
        path.write_bytes(production.canonical_json(payload) + b"\n")

    @classmethod
    def _mutate_source_code_hashes(cls, candidate, revision):
        if revision is not None:
            cls._mutate_calibration_identity(
                candidate,
                lambda payload: payload.update({"implementation_revision": revision}),
            )
        path = candidate / "source-code-sha256s.json"
        payload = json.loads(path.read_bytes())
        if revision is not None:
            payload["implementation_revision"] = revision
        payload["files"] = {
            source_path: "f" * 64 for source_path in payload["files"]
        }
        path.write_bytes(production.canonical_json(payload) + b"\n")

    @classmethod
    def _mutate_source_code_keys(cls, candidate, extra):
        path = candidate / "source-code-sha256s.json"
        payload = json.loads(path.read_bytes())
        if extra:
            payload["files"]["extra.py"] = "f" * 64
        else:
            payload["files"].pop(next(iter(payload["files"])))
        path.write_bytes(production.canonical_json(payload) + b"\n")

    @classmethod
    def _mutate_source_bundle(cls, candidate):
        path = candidate / "source-coverage-v5.json"
        bundle = json.loads(path.read_bytes())
        source = json.loads(bundle["source_json"])
        source["status"] = "caller_resealed"
        unsigned = dict(source)
        unsigned.pop("artifact_sha256")
        source["artifact_sha256"] = production.sha256_bytes(
            production.canonical_json(unsigned)
        )
        raw = production.canonical_json(source) + b"\n"
        bundle["source_json"] = raw.decode()
        bundle["source_file_sha256"] = production.sha256_bytes(raw)
        bundle["source_artifact_sha256"] = source["artifact_sha256"]
        path.write_bytes(production.canonical_json(bundle) + b"\n")

    @classmethod
    def _mutate_discovery_bundle(cls, candidate):
        path = candidate / "source-discovery.json"
        bundle = json.loads(path.read_bytes())
        source = json.loads(bundle["source_json"])
        source["candidate_eligible"] = True
        unsigned = dict(source)
        unsigned.pop("artifact_sha256")
        source["artifact_sha256"] = production.sha256_bytes(
            production.canonical_json(unsigned)
        )
        raw = production.canonical_json(source) + b"\n"
        bundle["source_json"] = raw.decode()
        bundle["source_file_sha256"] = production.sha256_bytes(raw)
        bundle["source_artifact_sha256"] = source["artifact_sha256"]
        path.write_bytes(production.canonical_json(bundle) + b"\n")


if __name__ == "__main__":
    unittest.main()
