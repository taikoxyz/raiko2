import copy
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
OPCODE_GAS = ROOT / "experiments" / "opcode-gas"
sys.path.insert(0, str(OPCODE_GAS))

import context_production_campaign as production


MANIFEST = OPCODE_GAS / "manifests" / "sp1-context-production-v1.json"


class ProductionContextManifestTests(unittest.TestCase):
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
        self.assertEqual(len(fixtures), 792)
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
        self.assertEqual(by_scenario["callvalue_nonzero_4294967297"].builder_input["program"]["profile"], {"kind": "callvalue", "value": 4_294_967_297, "value_class": "nonzero"})
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
        estimator = {**common, "sp1_execution_engine": "gas-estimator"}
        identity = production.production_context_parity_identity(
            row_id="c" * 64, standard=standard, gas_estimator=estimator
        )
        self.assertEqual(identity["kind"], "production_context_parity_v1")
        self.assertFalse(identity["model_sample"])
        self.assertEqual(len(identity["identity_sha256"]), 64)
        drifted = copy.deepcopy(estimator)
        drifted["gas"] += 1
        with self.assertRaisesRegex(ValueError, "parity mismatch"):
            production.production_context_parity_identity(
                row_id="c" * 64, standard=standard, gas_estimator=drifted
            )

    def test_standard_estimator_parity_requires_accepted_row_bound_successes(self):
        common = {
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
