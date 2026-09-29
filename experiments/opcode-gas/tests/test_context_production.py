import copy
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[3]
OPCODE_GAS = ROOT / "experiments" / "opcode-gas"
sys.path.insert(0, str(OPCODE_GAS))

import context_production_campaign as production


MANIFEST = OPCODE_GAS / "manifests" / "sp1-context-production-v1.json"


class ProductionContextManifestTests(unittest.TestCase):
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
                    "callvalue_nonzero_4294967297",
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
        self.assertEqual(len(rows), 792)
        self.assertEqual(len({row.row_id for row in rows}), len(rows))
        self.assertEqual({row.repeat_index for row in rows}, {0, 1, 2})

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
            {(0, 0), (4, 64)},
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


class ProductionContextCliTests(unittest.TestCase):
    def test_cli_registers_only_live_task_one_surfaces(self):
        completed = subprocess.run(
            [sys.executable, str(OPCODE_GAS / "opcode_gas.py"), "--help"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn("generate-context-production-manifest", completed.stdout)
        self.assertIn("validate-context-production-manifest", completed.stdout)
        for dead in (
            "prepare-context-production",
            "run-context-production",
            "fit-context-production",
            "verify-context-production-result",
            "seal-context-production-result",
        ):
            self.assertNotIn(dead, completed.stdout)

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


if __name__ == "__main__":
    unittest.main()
