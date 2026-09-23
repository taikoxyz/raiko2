import hashlib
import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class AnchorProbeTests(unittest.TestCase):
    def test_generator_freezes_counts_and_changes_only_typed_workload_fields(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            elf = root / "sp1_opcode_lab.elf"
            elf.write_bytes(b"anchor-probe-elf")
            out = root / "fixtures"

            manifest = opcode_gas.generate_anchor_probe_fixtures(elf, out)

            self.assertTrue(manifest["synthetic_prior_only"])
            self.assertFalse(manifest["candidate_eligible"])
            self.assertEqual(manifest["fit_counts"], [0, 1024, 4096, 16384, 65536])
            self.assertEqual(manifest["checkpoint_count"], 131072)
            self.assertEqual(manifest["repeats"], 3)
            fixtures = manifest["fixtures"]
            self.assertEqual(len(fixtures), 4 * 6 * 2)

            pop = [fixture for fixture in fixtures if fixture["anchor_key"] == "opcode:0x50"]
            target_inputs = []
            for fixture in pop:
                guest = json.loads((out / fixture["guest_input_path"]).read_text())
                self.assertEqual(guest["bytecode"], "0x00")
                self.assertEqual(guest["case"], "synthetic_anchor_probe_pop")
                self.assertEqual(
                    guest["scenario"], opcode_gas.ANCHOR_PROBE_SCENARIOS[fixture["lane"]]
                )
                if fixture["lane"] == "target":
                    target_inputs.append(guest)
            reference = target_inputs[0]
            for guest in target_inputs[1:]:
                differing = {key for key in guest if guest[key] != reference[key]}
                self.assertEqual(differing, {"target_count"})

            prefix = [fixture for fixture in pop if fixture["target_count"] == 1024]
            target = json.loads((out / prefix[0]["guest_input_path"]).read_text())
            control = json.loads((out / prefix[1]["guest_input_path"]).read_text())
            differing = {key for key in target if target[key] != control[key]}
            self.assertEqual(differing, {"scenario"})
            self.assertEqual(len(target["scenario"]), len(control["scenario"]))

    def test_fit_recovers_positive_anchor_ratios_with_free_intercepts(self):
        slopes = {
            "opcode:0x50": 5,
            "opcode:0x5f": 7,
            "opcode:0x80": 11,
            "opcode:0x90": 13,
        }
        rows = self._synthetic_rows(slopes)

        artifact = opcode_gas.fit_anchor_probe_rows(rows)

        self.assertTrue(artifact["synthetic_prior_only"])
        self.assertFalse(artifact["candidate_eligible"])
        by_key = {row["anchor_key"]: row for row in artifact["anchors"]}
        self.assertEqual(by_key["opcode:0x50"]["prover_gas_per_operation"], "5")
        self.assertEqual(by_key["opcode:0x50"]["instruction_count_per_operation"], "5")
        self.assertEqual(by_key["opcode:0x50"]["prover_gas_per_raw_gas"], "2.5")
        self.assertEqual(by_key["opcode:0x80"]["prover_gas_per_raw_gas"], "3.6666666666666666666666666666666666666666666666666666666666666666666666666666667")
        self.assertEqual(by_key["opcode:0x90"]["fitted_intercept_p"], "100")
        self.assertEqual(by_key["opcode:0x90"]["count0_intercept_residual_ratio"], "0")
        self.assertEqual(by_key["opcode:0x90"]["checkpoint_ape"], "0")

    def test_fit_rejects_nondeterministic_repeats_and_nonzero_exit(self):
        rows = self._synthetic_rows({key: 5 for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS})
        rows[1]["prover_gas"] += 1
        with self.assertRaisesRegex(ValueError, "deterministic"):
            opcode_gas.fit_anchor_probe_rows(rows)

        rows = self._synthetic_rows({key: 5 for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS})
        rows[0]["exit_code"] = 3
        with self.assertRaisesRegex(ValueError, "exit code"):
            opcode_gas.fit_anchor_probe_rows(rows)

    def test_fit_rejects_failed_signal_and_extrapolation_gates(self):
        anchor_keys = [key for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS]

        rows = self._synthetic_rows({key: 0 for key in anchor_keys})
        with self.assertRaisesRegex(ValueError, "positive_signal"):
            opcode_gas.fit_anchor_probe_rows(rows)

        rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        self._shift_target_delta(rows, "opcode:0x50", 4096, 1_000_000)
        with self.assertRaisesRegex(ValueError, "r2|slope_stderr|residual_signal"):
            opcode_gas.fit_anchor_probe_rows(rows)

        rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        self._shift_target_delta(rows, "opcode:0x50", 0, 100_000)
        with self.assertRaisesRegex(ValueError, "count0_intercept_residual"):
            opcode_gas.fit_anchor_probe_rows(rows)

        rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        self._shift_target_delta(rows, "opcode:0x50", 131072, 1_000_000)
        with self.assertRaisesRegex(ValueError, "checkpoint_ape"):
            opcode_gas.fit_anchor_probe_rows(rows)

    def test_checkpoint_gate_uses_marginal_signal_not_fixed_lane_overhead(self):
        anchor_keys = [key for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS]
        rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        for row in rows:
            if row["anchor_key"] != "opcode:0x50" or row["lane"] != "target":
                continue
            row["prover_gas"] += 1_000_000_000_000
            if row["target_count"] == opcode_gas.ANCHOR_PROBE_CHECKPOINT_COUNT:
                row["prover_gas"] += 5 * row["target_count"]

        with self.assertRaisesRegex(ValueError, "checkpoint_ape"):
            opcode_gas.fit_anchor_probe_rows(rows)

    def test_probe_consumer_binds_the_frozen_calibration_elf(self):
        anchor_keys = [key for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS]
        rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        artifact = opcode_gas.fit_anchor_probe_rows(rows)
        identity = {
            "calibration_id": "b" * 24,
            "calibration_identity_sha256": "b" * 64,
            "implementation_revision": "a" * 40,
            "sp1_sdk_version": "test-sdk",
            "guest_launcher_sha256": artifact["guest_launcher_sha256"],
            "guest_artifacts": {
                "crates/guests/elf/sp1_opcode_lab.elf": artifact["elf_sha256"]
            }
        }

        costs = opcode_gas.validated_anchor_probe_for_execution_identity(
            artifact, rows, identity
        )
        self.assertEqual(set(costs), set(anchor_keys))

        identity["guest_artifacts"]["crates/guests/elf/sp1_opcode_lab.elf"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "frozen calibration identity"):
            opcode_gas.validated_anchor_probe_for_execution_identity(
                artifact, rows, identity
            )

        identity["guest_artifacts"][
            "crates/guests/elf/sp1_opcode_lab.elf"
        ] = artifact["elf_sha256"]
        identity["guest_launcher_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "guest-launcher"):
            opcode_gas.validated_anchor_probe_for_execution_identity(
                artifact, rows, identity
            )

    def test_probe_consumer_replays_raw_rows_instead_of_trusting_a_self_hash(self):
        anchor_keys = [key for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS]
        rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        artifact = opcode_gas.fit_anchor_probe_rows(rows)
        artifact["anchors"][0]["prover_gas_per_operation"] = "5000000"
        artifact["anchors"][0]["prover_gas_per_raw_gas"] = "2500000"
        artifact.pop("artifact_sha256")
        artifact["artifact_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(artifact)
        )

        with self.assertRaisesRegex(ValueError, "exact raw-row replay"):
            opcode_gas.validated_anchor_probe_costs(artifact, rows)

    def test_instruction_diagnostic_failure_does_not_reject_primary_probe(self):
        anchor_keys = [key for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS]
        rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        for row in rows:
            row["total_instruction_count"] = 20_000_000 + 2 * row["target_count"]

        artifact = opcode_gas.fit_anchor_probe_rows(rows)

        self.assertTrue(all(row["accepted"] for row in artifact["anchors"]))
        self.assertTrue(
            all(
                row["instruction_fit"]["status"] == "unavailable"
                for row in artifact["anchors"]
            )
        )
        self.assertEqual(
            set(opcode_gas.validated_anchor_probe_costs(artifact, rows)),
            set(anchor_keys),
        )
        with self.assertRaisesRegex(ValueError, "instruction diagnostic is unavailable"):
            opcode_gas.validated_anchor_probe_costs(
                artifact, rows, metric="sp1_instruction_count"
            )

    def test_primary_probe_digest_ignores_instruction_only_changes(self):
        anchor_keys = [key for key, *_ in opcode_gas.ANCHOR_PROBE_ANCHORS]
        base_rows = self._synthetic_rows({key: 5 for key in anchor_keys})
        changed_rows = [dict(row) for row in base_rows]
        for row in changed_rows:
            row["total_instruction_count"] += 7 * row["target_count"]

        base = opcode_gas.fit_anchor_probe_rows(base_rows)
        changed = opcode_gas.fit_anchor_probe_rows(changed_rows)

        self.assertNotEqual(base["artifact_sha256"], changed["artifact_sha256"])
        self.assertNotEqual(base["raw_rows_sha256"], changed["raw_rows_sha256"])
        self.assertEqual(
            base["primary_artifact_sha256"], changed["primary_artifact_sha256"]
        )
        self.assertEqual(
            base["primary_raw_rows_sha256"], changed["primary_raw_rows_sha256"]
        )

    def test_run_rejects_nonzero_guest_exit_before_persisting_raw_rows(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            elf = root / "sp1_opcode_lab.elf"
            elf.write_bytes(b"anchor-probe-elf")
            fixtures = root / "fixtures"
            opcode_gas.generate_anchor_probe_fixtures(elf, fixtures)
            out = root / "raw.jsonl"
            (root / "guest-launcher").write_bytes(b"test launcher")

            def fake_run_guest_inputs(**kwargs):
                reports = []
                for index, input_path in enumerate(kwargs["input_paths"]):
                    reports.append(
                        {
                            "input": str(input_path),
                            "exit_code": 3 if index == 0 else 0,
                            "gas": 5132,
                            "public_values": "0x",
                            "guest_input_sha256": "0x" + "a" * 64,
                            "guest_input_bincode_length": 64,
                            "sp1_execution_engine": "gas-estimator",
                            "sp1_gas_trace_chunk_threshold": opcode_gas.SP1_GAS_TRACE_CHUNK_THRESHOLD,
                            "sp1_gas_trace_chunk_slots": opcode_gas.SP1_GAS_TRACE_CHUNK_SLOTS,
                            "guest_launcher_sha256": "f" * 64,
                            "run_provenance": {
                                "calibration_id": "b" * 24,
                                "calibration_identity_sha256": "b" * 64,
                                "implementation_revision": "a" * 40,
                                "sp1_sdk_version": "test-sdk",
                            },
                        }
                    )
                kwargs["reports_jsonl"].write_text(
                    "".join(json.dumps(report) + "\n" for report in reports)
                )

            with mock.patch.object(
                opcode_gas, "run_guest_inputs", side_effect=fake_run_guest_inputs
            ):
                with self.assertRaisesRegex(ValueError, "exit code"):
                    opcode_gas.run_anchor_probe_fixtures(
                        guest_launcher=root / "guest-launcher",
                        elf_path=elf,
                        fixtures_dir=fixtures,
                        out_path=out,
                    )

            self.assertFalse(out.exists())

    def test_manifest_binds_launcher_and_calibration_provenance_before_execution(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            elf = root / "sp1_opcode_lab.elf"
            elf.write_bytes(b"anchor-probe-elf")
            launcher = root / "guest-launcher"
            launcher.write_bytes(b"frozen launcher")
            fixtures = root / "fixtures"
            provenance = {
                "calibration_id": "b" * 24,
                "calibration_identity_sha256": "c" * 64,
                "implementation_revision": "a" * 40,
                "sp1_sdk_version": "6.3.0",
            }

            manifest = opcode_gas.generate_anchor_probe_fixtures(
                elf,
                fixtures,
                guest_launcher=launcher,
                run_provenance=provenance,
            )

            self.assertEqual(
                manifest["guest_launcher_sha256"], opcode_gas.sha256_file(launcher)
            )
            self.assertEqual(manifest["run_provenance"], provenance)

            launcher.write_bytes(b"different launcher")
            with mock.patch.object(opcode_gas, "run_guest_inputs") as runner:
                with self.assertRaisesRegex(ValueError, "guest-launcher digest"):
                    opcode_gas.run_anchor_probe_fixtures(
                        guest_launcher=launcher,
                        elf_path=elf,
                        fixtures_dir=fixtures,
                        out_path=root / "raw.jsonl",
                    )
            runner.assert_not_called()

    def test_run_rejects_fixture_content_that_disagrees_with_manifest(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            elf = root / "sp1_opcode_lab.elf"
            elf.write_bytes(b"anchor-probe-elf")
            fixtures = root / "fixtures"
            manifest = opcode_gas.generate_anchor_probe_fixtures(elf, fixtures)
            (root / "guest-launcher").write_bytes(b"test launcher")
            fixture = manifest["fixtures"][0]
            input_path = fixtures / fixture["guest_input_path"]
            guest_input = json.loads(input_path.read_text())
            guest_input["opcode"] = 0x01
            input_path.write_bytes(opcode_gas.canonical_json(guest_input) + b"\n")
            fixture["fixture_sha256"] = opcode_gas.sha256_file(input_path)
            fixture["anchor_sample_id"] = opcode_gas._anchor_probe_sample_id(
                anchor_key=fixture["anchor_key"],
                target_count=fixture["target_count"],
                lane=fixture["lane"],
                elf_sha256=fixture["elf_sha256"],
                fixture_sha256=fixture["fixture_sha256"],
            )
            manifest.pop("manifest_sha256")
            manifest["manifest_sha256"] = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(manifest)
            )
            (fixtures / "anchor-probe-manifest.json").write_bytes(
                opcode_gas.canonical_json(manifest) + b"\n"
            )

            with mock.patch.object(opcode_gas, "run_guest_inputs") as runner:
                with self.assertRaisesRegex(ValueError, "fixture declaration mismatch"):
                    opcode_gas.run_anchor_probe_fixtures(
                        guest_launcher=root / "guest-launcher",
                        elf_path=elf,
                        fixtures_dir=fixtures,
                        out_path=root / "raw.jsonl",
                    )
            runner.assert_not_called()

    def test_parser_exposes_anchor_probe_pipeline(self):
        parser = opcode_gas.build_parser()
        for command in ("generate-anchor-probe", "run-anchor-probe", "fit-anchor-probe"):
            with self.subTest(command=command):
                with self.assertRaises(SystemExit) as error:
                    parser.parse_args([command, "--help"])
                self.assertEqual(error.exception.code, 0)

    @staticmethod
    def _synthetic_rows(slopes):
        rows = []
        counts = [*opcode_gas.ANCHOR_PROBE_FIT_COUNTS, opcode_gas.ANCHOR_PROBE_CHECKPOINT_COUNT]
        for anchor_key, _name, opcode, raw_gas in opcode_gas.ANCHOR_PROBE_ANCHORS:
            for count in counts:
                for lane in ("target", "control"):
                    for repeat_index in range(3):
                        common = 20_000_000 + 2 * count
                        delta = 100 + slopes[anchor_key] * count
                        gas = common + (delta if lane == "target" else 0)
                        fixture_sha256 = hashlib.sha256(
                            f"{anchor_key}:{count}:{lane}".encode()
                        ).hexdigest()
                        guest_input_sha256 = "0x" + hashlib.sha256(
                            f"guest:{anchor_key}:{count}:{lane}".encode()
                        ).hexdigest()
                        row = {
                            "purpose": opcode_gas.ANCHOR_PROBE_PURPOSE,
                            "anchor_key": anchor_key,
                            "anchor_name": _name,
                            "opcode": opcode,
                            "target_raw_gas": raw_gas,
                            "target_count": count,
                            "lane": lane,
                            "repeat_index": repeat_index,
                            "prover_gas": gas,
                            "total_instruction_count": gas - 1_000_000,
                            "total_syscall_count": 3,
                            "exit_code": 0,
                            "public_values": "0x"
                            + hashlib.sha256(
                                f"public:{anchor_key}:{count}:{lane}".encode()
                            ).hexdigest(),
                            "guest_input_sha256": guest_input_sha256,
                            "guest_input_bincode_length": 64,
                            "elf_sha256": "e" * 64,
                            "anchor_probe_manifest_sha256": "d" * 64,
                            "fixture_sha256": fixture_sha256,
                            "sp1_execution_engine": "gas-estimator",
                            "sp1_gas_trace_chunk_threshold": opcode_gas.SP1_GAS_TRACE_CHUNK_THRESHOLD,
                            "sp1_gas_trace_chunk_slots": opcode_gas.SP1_GAS_TRACE_CHUNK_SLOTS,
                            "guest_launcher_sha256": "f" * 64,
                            "run_provenance": {
                                "calibration_id": "b" * 24,
                                "calibration_identity_sha256": "b" * 64,
                                "implementation_revision": "a" * 40,
                                "sp1_sdk_version": "test-sdk",
                            },
                            "anchor_pair_id": opcode_gas._anchor_probe_pair_id(
                                anchor_key, count, "e" * 64
                            ),
                            "anchor_sample_id": opcode_gas._anchor_probe_sample_id(
                                anchor_key=anchor_key,
                                target_count=count,
                                lane=lane,
                                elf_sha256="e" * 64,
                                fixture_sha256=fixture_sha256,
                            ),
                        }
                        row["anchor_execution_row_id"] = (
                            opcode_gas._anchor_probe_execution_row_id(row)
                        )
                        rows.append(row)
        return rows

    @staticmethod
    def _shift_target_delta(rows, anchor_key, target_count, amount):
        for row in rows:
            if (
                row["anchor_key"] == anchor_key
                and row["target_count"] == target_count
                and row["lane"] == "target"
            ):
                row["prover_gas"] += amount


if __name__ == "__main__":
    unittest.main()
