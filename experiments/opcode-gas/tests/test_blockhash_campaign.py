import json
import pathlib
import sys
import tempfile
import unittest
from decimal import Decimal
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments/opcode-gas"))

import blockhash_campaign as blockhash


class BlockHashCampaignTests(unittest.TestCase):
    def _source_hashes(self):
        return {
            relative: blockhash.sha256_file(ROOT / relative)
            for relative in blockhash._SEAL_SOURCE_PATHS
        }

    def _prepared_identity(self, *, assets, source_hashes, execution_rows):
        manifest = blockhash.canonical_blockhash_manifest_payload()
        inputs = {row["row_id"]: row["input_sha256"] for row in execution_rows}
        identity = {
            "schema_version": blockhash.SCHEMA_VERSION,
            "purpose": blockhash.PURPOSE,
            "manifest": manifest,
            "manifest_identity_sha256": blockhash.blockhash_manifest_identity(manifest),
            "sources": {
                name: {"artifact_sha256": source["artifact_sha256"]}
                for name, source in blockhash.SOURCES.items()
            },
            "assets": assets,
            "implementation_revision": "d" * 40,
            "source_code_sha256s": source_hashes,
            "rows": [
                {
                    "row_id": row.row_id,
                    "input_sha256": inputs[row.row_id],
                }
                for row in blockhash.blockhash_row_specs(manifest)
            ],
        }
        identity["identity_sha256"] = blockhash.sha256_bytes(
            blockhash.canonical_json(identity)
        )
        return identity

    def _write_resealed(self, root: pathlib.Path, payload):
        payload = dict(payload)
        payload.pop("artifact_sha256", None)
        payload.pop("result_id", None)
        payload.pop("result_identity_sha256", None)
        payload["result_identity_sha256"] = blockhash.sha256_bytes(
            blockhash.canonical_json(payload)
        )
        payload["result_id"] = payload["result_identity_sha256"][:24]
        payload["artifact_sha256"] = blockhash.sha256_bytes(
            blockhash.canonical_json(payload)
        )
        directory = root / payload["result_id"]
        directory.mkdir()
        (directory / "result.json").write_bytes(
            blockhash.canonical_json(payload) + b"\n"
        )
        return directory

    def _bind_prepared_inputs(self, rows, identity):
        inputs = {row["row_id"]: row["input_sha256"] for row in identity["rows"]}
        bound = []
        for source in rows:
            row = dict(source)
            row["input_sha256"] = inputs[row["row_id"]]
            row.pop("evidence_sha256")
            row["evidence_sha256"] = blockhash.sha256_bytes(
                blockhash.canonical_json(row)
            )
            bound.append(row)
        return bound

    def _rows(
        self,
        semantic_class: str,
        event_cost: Decimal,
        *,
        elf_sha256: str = "f" * 64,
        launcher_sha256: str = "e" * 64,
        iszero_event_cost: Decimal = Decimal("16"),
        intercept: Decimal = Decimal("0"),
    ):
        rows = []
        for count in (*blockhash.FIT_COUNTS, blockhash.CHECKPOINT_COUNT):
            for lane in ("control", "target"):
                for repeat_index in range(blockhash.REPEATS):
                    workload_id = blockhash.blockhash_workload_id(
                        blockhash.canonical_blockhash_manifest_payload(),
                        semantic_class,
                        count,
                    )
                    row_id = blockhash.blockhash_row_id(
                        workload_id=workload_id,
                        lane=lane,
                        repeat_index=repeat_index,
                    )
                    raw_gas = (
                        {"opcode:0x15": 3 * count}
                        if lane == "control"
                        else {"opcode:0x40": 20 * count}
                    )
                    events = (
                        {"opcode:0x15": count}
                        if lane == "control"
                        else {"opcode:0x40": count}
                    )
                    control_gas = Decimal("100000") + Decimal(count) * iszero_event_cost
                    gas = (
                        control_gas
                        if lane == "control"
                        else control_gas + intercept + Decimal(count) * (event_cost - iszero_event_cost)
                    )
                    row = {
                        "row_id": row_id,
                        "workload_id": workload_id,
                        "semantic_class": semantic_class,
                        "count": count,
                        "split": "fit"
                        if count in blockhash.FIT_COUNTS
                        else "holdout",
                        "lane": lane,
                        "repeat_index": repeat_index,
                        "input_sha256": blockhash.sha256_bytes(
                            f"{row_id}:input".encode()
                        ),
                        "prover_gas": str(gas),
                        "public_output": "0x" + "11" * 32,
                        "backend_input_sha256": "a" * 64,
                        "host_trace_sha256": "b" * 64,
                        "actual_raw_gas_by_key": raw_gas,
                        "actual_operation_event_count_by_key": events,
                        "sp1_proposal_elf_sha256": elf_sha256,
                        "guest_launcher_sha256": launcher_sha256,
                    }
                    row["evidence_sha256"] = blockhash.sha256_bytes(
                        blockhash.canonical_json(row)
                    )
                    rows.append(row)
        return rows

    def test_manifest_freezes_closed_classes_rows_and_v7_source(self):
        manifest = blockhash.canonical_blockhash_manifest_payload()
        self.assertEqual(manifest["schema_version"], 2)
        self.assertEqual(manifest["purpose"], "production_blockhash_calibration")
        self.assertEqual(manifest["operation_ownership_schema_version"], 4)
        self.assertEqual(manifest["fit_counts"], [0, 128, 256, 512])
        self.assertEqual(manifest["checkpoint_count"], 1024)
        self.assertEqual(manifest["repeats"], 3)
        self.assertEqual(
            [entry["semantic_class"] for entry in manifest["semantic_classes"]],
            [
                "recent_ancestor_hit_1",
                "recent_ancestor_hit_256",
                "out_of_range_zero",
            ],
        )
        self.assertEqual(
            manifest["sources"]["operation_coverage_v7"]["path"],
            "experiments/opcode-gas/manifests/operation-coverage-v7.json",
        )
        self.assertEqual(len(blockhash.blockhash_row_specs(manifest)), 90)
        self.assertEqual(
            manifest["exploratory_predecessor"]["result_id"],
            "742514d26f94a9b53337a3af",
        )
        self.assertEqual(
            manifest["exploratory_predecessor"]["artifact_sha256"],
            "34f3e95b07223ec4b6d466ab90ef30707b00e141fd35f1b413c75fee916704c2",
        )

    def test_fit_requires_every_class_and_uses_conservative_maximum(self):
        rows = []
        rows.extend(self._rows("recent_ancestor_hit_1", Decimal("80")))
        rows.extend(self._rows("recent_ancestor_hit_256", Decimal("100")))
        rows.extend(self._rows("out_of_range_zero", Decimal("90")))
        registry = {
            "common_dispatch": "10",
            "models": {
                "opcode:0x15": {
                    "kind": "static_raw_gas",
                    "parameters": {"body_per_raw_gas": "2"},
                }
            },
        }
        result = blockhash.fit_blockhash_rows(
            rows,
            manifest=blockhash.canonical_blockhash_manifest_payload(),
            registry=registry,
        )
        self.assertEqual(result["status"], "accepted")
        self.assertEqual(result["selected"]["event_cost"], "100")
        self.assertEqual(result["selected"]["body_per_raw_gas"], "4.5")

    def test_fit_fails_closed_when_zero_or_checkpoint_delta_is_invalid(self):
        rows = []
        for semantic_class in blockhash.SEMANTIC_CLASSES:
            rows.extend(self._rows(semantic_class, Decimal("80")))
        rows[0]["prover_gas"] = "100001"
        result = blockhash.fit_blockhash_rows(
            rows,
            manifest=blockhash.canonical_blockhash_manifest_payload(),
            registry={
                "common_dispatch": "10",
                "models": {
                    "opcode:0x15": {
                        "kind": "static_raw_gas",
                        "parameters": {"body_per_raw_gas": "2"},
                    }
                },
            },
        )
        self.assertEqual(result["status"], "unmeasured")
        self.assertTrue(result["rejection_reasons"])

    def test_affine_fit_accepts_negative_slope_when_reconstructed_event_cost_is_positive(self):
        rows = []
        for semantic_class in blockhash.SEMANTIC_CLASSES:
            rows.extend(
                self._rows(
                    semantic_class,
                    Decimal("10"),
                    iszero_event_cost=Decimal("16"),
                    intercept=Decimal("-7"),
                )
            )
        result = blockhash.fit_blockhash_rows(
            rows,
            manifest=blockhash.canonical_blockhash_manifest_payload(),
            registry={
                "common_dispatch": "10",
                "models": {
                    "opcode:0x15": {
                        "kind": "static_raw_gas",
                        "parameters": {"body_per_raw_gas": "2"},
                    }
                },
            },
        )
        self.assertEqual(result["status"], "accepted")
        self.assertEqual(result["classes"][0]["intercept"], "-7")
        self.assertEqual(result["classes"][0]["paired_slope"], "-6")
        self.assertEqual(result["selected"]["event_cost"], "10")

    def test_affine_fit_rejects_invalid_candidate_costs_and_checkpoint_residual(self):
        registry = {
            "common_dispatch": "10",
            "models": {
                "opcode:0x15": {
                    "kind": "static_raw_gas",
                    "parameters": {"body_per_raw_gas": "2"},
                }
            },
        }
        nonpositive = []
        for semantic_class in blockhash.SEMANTIC_CLASSES:
            nonpositive.extend(
                self._rows(
                    semantic_class,
                    Decimal("0"),
                    iszero_event_cost=Decimal("16"),
                )
            )
        self.assertEqual(
            blockhash.fit_blockhash_rows(
                nonpositive,
                manifest=blockhash.canonical_blockhash_manifest_payload(),
                registry=registry,
            )["status"],
            "unmeasured",
        )

        negative_body = []
        for semantic_class in blockhash.SEMANTIC_CLASSES:
            negative_body.extend(
                self._rows(
                    semantic_class,
                    Decimal("5"),
                    iszero_event_cost=Decimal("16"),
                )
            )
        negative_body_result = blockhash.fit_blockhash_rows(
            negative_body,
            manifest=blockhash.canonical_blockhash_manifest_payload(),
            registry=registry,
        )
        self.assertEqual(negative_body_result["status"], "unmeasured")
        self.assertTrue(
            any(
                "reconstructed body per raw gas is negative" in reason
                for reason in negative_body_result["rejection_reasons"]
            )
        )

        residual = []
        for semantic_class in blockhash.SEMANTIC_CLASSES:
            residual.extend(
                self._rows(
                    semantic_class,
                    Decimal("10"),
                    iszero_event_cost=Decimal("16"),
                    intercept=Decimal("-7"),
                )
            )
        for row in residual:
            if row["count"] == blockhash.CHECKPOINT_COUNT and row["lane"] == "target":
                row["prover_gas"] = str(Decimal(row["prover_gas"]) + Decimal("1000"))
                row.pop("evidence_sha256")
                row["evidence_sha256"] = blockhash.sha256_bytes(
                    blockhash.canonical_json(row)
                )
        result = blockhash.fit_blockhash_rows(
            residual,
            manifest=blockhash.canonical_blockhash_manifest_payload(),
            registry=registry,
        )
        self.assertEqual(result["status"], "unmeasured")
        self.assertTrue(
            any("checkpoint residual exceeds tolerance" in reason for reason in result["rejection_reasons"])
        )

    def test_seal_binds_v7_sources_assets_and_rejects_partial_candidates(self):
        sources = blockhash.validate_blockhash_sources(ROOT)
        iszero_event_cost = blockhash._iszero_event_cost(
            sources["production_registry"]["registry"]
        )
        rows = []
        for semantic_class, event_cost in zip(
            blockhash.SEMANTIC_CLASSES,
            (Decimal("80"), Decimal("100"), Decimal("90")),
        ):
            rows.extend(
                self._rows(
                    semantic_class,
                    event_cost,
                    iszero_event_cost=iszero_event_cost,
                )
            )
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source_hashes = self._source_hashes()
            with mock.patch.object(
                blockhash,
                "_git_source_identity",
                return_value=source_hashes,
            ):
                sealed = blockhash.seal_blockhash_result(
                    repo_root=ROOT,
                    out_root=root,
                    run_identity=self._prepared_identity(
                        assets={
                            "launcher": {
                                "path": "target/release/guest-launcher",
                                "sha256": "e" * 64,
                            },
                            "production_elf": {
                                "path": "crates/guests/elf/sp1_shasta_proposal.elf",
                                "sha256": "f" * 64,
                            },
                            "production_vk": {
                                "path": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
                                "sha256": "a" * 64,
                            },
                        },
                        source_hashes=source_hashes,
                        execution_rows=rows,
                    ),
                    rows=rows,
                )
            self.assertEqual(sealed["status"], "accepted")
            result_directory = root / sealed["result_id"]
            self.assertTrue((result_directory / "result.json").is_file())
            self.assertEqual(
                blockhash.verify_blockhash_result(result_directory)["result_id"],
                sealed["result_id"],
            )

            payload = json.loads((result_directory / "result.json").read_text())
            payload["decision"]["selected"]["event_cost"] = "999"
            with self.assertRaises(ValueError):
                blockhash.verify_blockhash_result(self._write_resealed(root, payload))

            payload = json.loads((result_directory / "result.json").read_text())
            payload["production_table_updated"] = True
            with self.assertRaises(ValueError):
                blockhash.verify_blockhash_result(self._write_resealed(root, payload))

            payload = json.loads((result_directory / "result.json").read_text())
            source = next(iter(payload["source_code_sha256s"]))
            payload["source_code_sha256s"][source] = "0" * 64
            payload["run_identity"]["source_code_sha256s"][source] = "0" * 64
            with self.assertRaises(ValueError):
                blockhash.verify_blockhash_result(self._write_resealed(root, payload))

            payload = json.loads((result_directory / "result.json").read_text())
            payload["run_identity"].pop("prepared_identity_sha256")
            with self.assertRaises(ValueError):
                blockhash.verify_blockhash_result(self._write_resealed(root, payload))

            payload = json.loads((result_directory / "result.json").read_text())
            payload["execution_rows"][0]["input_sha256"] = "0" * 64
            payload["execution_rows"][0].pop("evidence_sha256")
            payload["execution_rows"][0]["evidence_sha256"] = blockhash.sha256_bytes(
                blockhash.canonical_json(payload["execution_rows"][0])
            )
            payload["execution_rows_sha256"] = blockhash.sha256_bytes(
                blockhash.canonical_json(payload["execution_rows"])
            )
            with self.assertRaises(ValueError):
                blockhash.verify_blockhash_result(self._write_resealed(root, payload))

            with self.assertRaises(TypeError):
                blockhash.seal_blockhash_result(
                    repo_root=ROOT,
                    out_root=root / "injected-registry",
                    run_identity=self._prepared_identity(
                        assets={
                            "launcher": {
                                "path": "target/release/guest-launcher",
                                "sha256": "e" * 64,
                            },
                            "production_elf": {
                                "path": "crates/guests/elf/sp1_shasta_proposal.elf",
                                "sha256": "f" * 64,
                            },
                            "production_vk": {
                                "path": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
                                "sha256": "a" * 64,
                            },
                        },
                        source_hashes=source_hashes,
                        execution_rows=rows,
                    ),
                    rows=rows,
                    registry={"common_dispatch": "0", "models": {}},
                )

    def test_prepare_accepts_git_revision_and_records_shared_launcher_logically(self):
        calls = []

        def identity_runner(request):
            calls.append(request["row_id"])
            return {
                "spec": request,
                "observation": {"row_id": request["row_id"]},
                "fixture_spec_sha256": "b" * 64,
            }

        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            launcher = temporary / "guest-launcher"
            launcher.write_bytes(b"shared release launcher")
            source_hashes = self._source_hashes()
            with mock.patch.object(
                blockhash,
                "_git_source_identity",
                return_value=source_hashes,
            ):
                identity = blockhash.prepare_blockhash_run(
                    repo_root=ROOT,
                    run=temporary / "run",
                    launcher=launcher,
                    production_elf=ROOT / "crates/guests/elf/sp1_shasta_proposal.elf",
                    production_vk=ROOT / "crates/guests/elf/sp1_shasta_proposal.vk.bin",
                    implementation_revision="d" * 40,
                    identity_runner=identity_runner,
                )

            self.assertEqual(len(calls), 90)
            self.assertEqual(identity["implementation_revision"], "d" * 40)
            self.assertEqual(
                identity["assets"]["launcher"]["path"],
                "target/release/guest-launcher",
            )
            self.assertNotIn(str(temporary), str(identity))
            self.assertEqual(identity["source_code_sha256s"], source_hashes)
            self.assertEqual(
                blockhash._validate_prepared_run_identity_for_seal(identity)[
                    "prepared_identity_sha256"
                ],
                identity["identity_sha256"],
            )
            rows = []
            sources = blockhash.validate_blockhash_sources(ROOT)
            iszero_event_cost = blockhash._iszero_event_cost(
                sources["production_registry"]["registry"]
            )
            for semantic_class, event_cost in zip(
                blockhash.SEMANTIC_CLASSES,
                (Decimal("80"), Decimal("100"), Decimal("90")),
            ):
                rows.extend(
                    self._rows(
                        semantic_class,
                        event_cost,
                        elf_sha256=identity["assets"]["production_elf"]["sha256"],
                        launcher_sha256=identity["assets"]["launcher"]["sha256"],
                        iszero_event_cost=iszero_event_cost,
                    )
                )
            rows = self._bind_prepared_inputs(rows, identity)
            with mock.patch.object(
                blockhash,
                "_git_source_identity",
                return_value=source_hashes,
            ):
                sealed = blockhash.seal_blockhash_result(
                    repo_root=ROOT,
                    out_root=temporary / "sealed",
                    run_identity=identity,
                    rows=rows,
                )
            self.assertEqual(sealed["status"], "accepted")

    def test_prepare_rejects_launcher_replaced_during_native_freeze(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            launcher = temporary / "guest-launcher"
            launcher.write_bytes(b"initial launcher")
            calls = 0

            def replacing_runner(request):
                nonlocal calls
                calls += 1
                if calls == 90:
                    launcher.write_bytes(b"replacement launcher")
                return {
                    "spec": request,
                    "observation": {"row_id": request["row_id"]},
                    "fixture_spec_sha256": "b" * 64,
                }

            with mock.patch.object(
                blockhash,
                "_git_source_identity",
                return_value=self._source_hashes(),
            ):
                with self.assertRaises(ValueError):
                    blockhash.prepare_blockhash_run(
                        repo_root=ROOT,
                        run=temporary / "run",
                        launcher=launcher,
                        production_elf=ROOT
                        / "crates/guests/elf/sp1_shasta_proposal.elf",
                        production_vk=ROOT
                        / "crates/guests/elf/sp1_shasta_proposal.vk.bin",
                        implementation_revision="d" * 40,
                        identity_runner=replacing_runner,
                    )

    def test_execution_evidence_accepts_real_report_shape_and_binds_launcher(self):
        row = {
            "row_id": "1" * 64,
            "workload_id": "2" * 64,
            "semantic_class": "recent_ancestor_hit_1",
            "count": 1,
            "split": "fit",
            "lane": "target",
            "repeat_index": 0,
            "input_sha256": "3" * 64,
        }
        report = {
            "stage": "controlled-block",
            "mode": "execute",
            "sp1_execution_engine": "gas-estimator",
            "exit_code": 0,
            "sp1_proposal_elf_sha256": "5" * 64,
            "guest_launcher_sha256": "6" * 64,
            "guest_input_sha256": "0x" + "7" * 64,
            "gas": 123,
            "public_values": "0x" + "8" * 64,
            "controlled_block": {
                "status": "accepted",
                "row_id": row["row_id"],
                "observation": {
                    "backend_input_sha256": "7" * 64,
                    "host_trace_sha256": "9" * 64,
                    "actual_raw_gas_by_key": {"opcode:0x40": 20},
                    "actual_operation_event_count_by_key": {"opcode:0x40": 1},
                },
            },
        }
        evidence = blockhash._normalize_execution_report(
            row,
            report,
            production_elf_sha256="5" * 64,
            launcher_sha256="6" * 64,
        )
        self.assertEqual(evidence["guest_launcher_sha256"], "6" * 64)
        report["guest_launcher_sha256"] = "a" * 64
        with self.assertRaises(ValueError):
            blockhash._normalize_execution_report(
                row,
                report,
                production_elf_sha256="5" * 64,
                launcher_sha256="6" * 64,
            )


if __name__ == "__main__":
    unittest.main()
