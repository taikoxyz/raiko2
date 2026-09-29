import copy
import functools
import importlib.util
import json
import pathlib
import shutil
import sys
import tempfile
import unittest
from fractions import Fraction
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
MODULE_PATH = ROOT / "experiments/opcode-gas/context_opcode_campaign.py"
MANIFEST_PATH = ROOT / "experiments/opcode-gas/manifests/sp1-context-opcode-v1.json"
REGISTRY_PATH = ROOT / "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json"
CORRECTED_REGISTRY_PATH = ROOT / "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json"
CORRECTED_PACKAGE_PATH = ROOT / "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/augmentation.json"
COVERAGE_V4_PATH = ROOT / "experiments/opcode-gas/manifests/operation-coverage-v4.json"
TEST_CALIBRATION_RUN = ROOT / "experiments/opcode-gas/runs/51f71fde68f378842f872fc1"
TEST_CALIBRATION_IDENTITY = json.loads(
    (TEST_CALIBRATION_RUN / "experiment.json").read_text()
)["calibration_identity"]
HISTORICAL_CONTROL_ELF_SHA256 = TEST_CALIBRATION_IDENTITY["guest_artifacts"][
    "crates/guests/elf/sp1_opcode_lab.elf"
]
HISTORICAL_LEGACY_ELF_SHA256 = TEST_CALIBRATION_IDENTITY["guest_artifacts"][
    "crates/guests/elf/sp1_revm_opcode_lab.elf"
]
HISTORICAL_LAUNCHER_SHA256 = TEST_CALIBRATION_IDENTITY[
    "guest_launcher_sha256"
]
TEST_CONTEXT_ELF_SHA256 = "c" * 64
sys.path.insert(0, str(ROOT / "experiments/opcode-gas"))
import opcode_gas


def load_module():
    spec = importlib.util.spec_from_file_location("context_opcode_campaign", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


context = load_module()
REAL_CONTEXT_IDENTITY_REPLAY = context._replay_context_result_identity


@functools.lru_cache(maxsize=1)
def current_anchor_replay_fixture():
    source = production_campaign_source(
        {
            "legacy_revm_elf_sha256": HISTORICAL_LEGACY_ELF_SHA256,
            "context_elf_sha256": TEST_CONTEXT_ELF_SHA256,
            "control_opcode_lab_elf_sha256": HISTORICAL_CONTROL_ELF_SHA256,
            "launcher_sha256": HISTORICAL_LAUNCHER_SHA256,
        }
    )
    provenance = {
        "calibration_id": source["calibration_id"],
        "calibration_identity_sha256": source["calibration_identity_sha256"],
        "implementation_revision": source["implementation_revision"],
        "sp1_sdk_version": TEST_CALIBRATION_IDENTITY["sp1_sdk_version"],
    }
    historical_fixtures = (
        ROOT
        / "experiments/opcode-gas/runs/09ebb08d76d3f461086b0cf4/generated/anchor-probe"
    )
    manifest = json.loads(
        (historical_fixtures / "anchor-probe-manifest.json").read_text()
    )
    manifest["elf_sha256"] = HISTORICAL_CONTROL_ELF_SHA256
    manifest["guest_launcher_sha256"] = HISTORICAL_LAUNCHER_SHA256
    manifest["run_provenance"] = provenance
    historical_public_values = {
        (row["anchor_key"], row["target_count"], row["lane"]): row[
            "public_values"
        ]
        for row in (
            json.loads(line)
            for line in (
                ROOT
                / "experiments/opcode-gas/runs/09ebb08d76d3f461086b0cf4/raw/anchor-probe.jsonl"
            )
            .read_text()
            .splitlines()
        )
        if row["repeat_index"] == 0
    }
    inputs = []
    identities = []
    for fixture in manifest["fixtures"]:
        relative = fixture["guest_input_path"]
        value = json.loads((historical_fixtures / relative).read_text())
        fixture["elf_sha256"] = HISTORICAL_CONTROL_ELF_SHA256
        fixture["anchor_pair_id"] = opcode_gas._anchor_probe_pair_id(
            fixture["anchor_key"], fixture["target_count"], HISTORICAL_CONTROL_ELF_SHA256
        )
        fixture["anchor_sample_id"] = opcode_gas._anchor_probe_sample_id(
            anchor_key=fixture["anchor_key"],
            target_count=fixture["target_count"],
            lane=fixture["lane"],
            elf_sha256=HISTORICAL_CONTROL_ELF_SHA256,
            fixture_sha256=fixture["fixture_sha256"],
        )
        inputs.append({"guest_input_path": relative, "input": value})
        identities.append(
            {
                "guest_input_path": relative,
                "backend_input_sha256": context.sha256_bytes(
                    context.canonical_json(
                        {"schema": "current-test", "input": value}
                    )
                ),
                "backend_input_len": 153,
                "expected_public_values": historical_public_values[
                    (
                        fixture["anchor_key"],
                        fixture["target_count"],
                        fixture["lane"],
                    )
                ],
            }
        )
    manifest.pop("manifest_sha256")
    manifest["manifest_sha256"] = context.sha256_bytes(
        context.canonical_json(manifest)
    )
    return {
        "calibration_identity": copy.deepcopy(source["calibration_identity"]),
        "calibration_identity_sha256": context.sha256_bytes(
            context.canonical_json(source["calibration_identity"])
        ),
        "fixture_manifest": manifest,
        "fixture_inputs": inputs,
        "native_input_identities": identities,
    }


def passing_production_canary(*, source_identity=None):
    if source_identity is None:
        source_identity = {
            "legacy_revm_elf_sha256": HISTORICAL_LEGACY_ELF_SHA256,
            "context_elf_sha256": TEST_CONTEXT_ELF_SHA256,
            "control_opcode_lab_elf_sha256": HISTORICAL_CONTROL_ELF_SHA256,
            "launcher_sha256": HISTORICAL_LAUNCHER_SHA256,
        }
    campaign_source = production_campaign_source(source_identity)
    legacy_root = TEST_CALIBRATION_RUN / "osaka-opcode-supplement"
    legacy = json.loads((legacy_root / "compatibility-canary.json").read_text())
    baseline = opcode_gas.validate_historical_core_opcode_baseline(
        ROOT / "experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02",
        ROOT
        / "experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml",
    )
    historical_observations = opcode_gas._historical_osaka_canary_observations(
        baseline
    )
    legacy_replay = context._capture_legacy_osaka_replay(
        output_root=legacy_root,
        controlled_manifest=TEST_CALIBRATION_RUN / "controlled-manifest.toml",
        historical_observations=historical_observations,
        workload_identity_schema_version=1,
    )
    legacy_provenance = {
        "calibration_id": campaign_source["calibration_id"],
        "calibration_identity_sha256": campaign_source[
            "calibration_identity_sha256"
        ],
        "implementation_revision": campaign_source["implementation_revision"],
        "controlled_manifest_sha256": campaign_source["calibration_identity"][
            "controlled_manifest_sha256"
        ],
        "controlled_manifest_rows_sha256": campaign_source[
            "calibration_identity"
        ]["controlled_manifest_rows_sha256"],
        "complete_schedule_sha256": campaign_source["calibration_identity"][
            "complete_schedule_sha256"
        ],
        "guest_elf_sha256": campaign_source["legacy_revm_elf_sha256"],
        "version_identity": opcode_gas.validate_calibration_version_identity(
            campaign_source["calibration_identity"]
        ),
    }
    legacy_replay = rebind_legacy_replay_to_current_identity(
        legacy_replay, legacy_provenance
    )
    replay_historical, replay_current = context._replay_legacy_osaka_evidence(
        legacy_replay, legacy_provenance
    )
    legacy = opcode_gas.build_osaka_compatibility_canary(
        replay_historical,
        replay_current,
        baseline_artifact_sha256=(
            context.HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256
        ),
        expected_baseline_artifact_sha256=(
            context.HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256
        ),
    )
    legacy["provenance"] = legacy_provenance
    legacy["artifact_sha256"] = context.sha256_bytes(
        context.canonical_json(
            {
                key: value
                for key, value in legacy.items()
                if key != "artifact_sha256"
            }
        )
    )
    anchor_root = ROOT / "experiments/opcode-gas/runs/09ebb08d76d3f461086b0cf4"
    historical = json.loads((anchor_root / "anchor-probe-fit.json").read_text())
    historical_rows = [
        json.loads(line)
        for line in (anchor_root / "raw/anchor-probe.jsonl").read_text().splitlines()
    ]
    current_rows = copy.deepcopy(historical_rows)
    run_provenance = {
        "calibration_id": campaign_source["calibration_id"],
        "calibration_identity_sha256": campaign_source[
            "calibration_identity_sha256"
        ],
        "implementation_revision": campaign_source["implementation_revision"],
        "sp1_sdk_version": campaign_source["calibration_identity"][
            "sp1_sdk_version"
        ],
    }
    for row in current_rows:
        replay = current_anchor_replay_fixture()
        fixtures = {
            (
                fixture["anchor_key"],
                fixture["target_count"],
                fixture["lane"],
            ): fixture
            for fixture in replay["fixture_manifest"]["fixtures"]
        }
        native = {
            record["guest_input_path"]: record
            for record in replay["native_input_identities"]
        }
        fixture = fixtures[(row["anchor_key"], row["target_count"], row["lane"])]
        for field in opcode_gas._ANCHOR_PROBE_FIXTURE_FIELDS:
            row[field] = fixture[field]
        input_identity = native[fixture["guest_input_path"]]
        row["anchor_probe_manifest_sha256"] = replay["fixture_manifest"][
            "manifest_sha256"
        ]
        row["guest_input_sha256"] = (
            f"0x{input_identity['backend_input_sha256']}"
        )
        row["guest_input_bincode_length"] = input_identity["backend_input_len"]
        row["guest_launcher_sha256"] = source_identity["launcher_sha256"]
        row["run_provenance"] = run_provenance
        row["anchor_execution_row_id"] = opcode_gas._anchor_probe_execution_row_id(
            row
        )
    current = opcode_gas.fit_anchor_probe_rows(current_rows)
    replay = current_anchor_replay_fixture()
    return context._build_context_compatibility_canary(
        legacy_osaka_canary=legacy,
        context_elf_sha256=source_identity["context_elf_sha256"],
        historical_anchor_fit=historical,
        current_anchor_fit=current,
        historical_raw_sha256=context.HISTORICAL_ANCHOR_RAW_FILE_SHA256,
        current_raw_sha256=context.sha256_bytes(
            b"".join(
                context.canonical_json(row) + b"\n" for row in current_rows
            )
        ),
        historical_anchor_rows=historical_rows,
        current_anchor_rows=current_rows,
        historical_derivation_artifact_sha256=(
            context.HISTORICAL_ANCHOR_DERIVATION_SHA256
        ),
        legacy_osaka_replay=legacy_replay,
        current_anchor_replay=replay,
        current_anchor_source_ledger=context._current_anchor_source_ledger(
            replay, current_rows, current, "7" * 64
        ),
    )


def rebind_legacy_replay_to_current_identity(replay, provenance):
    replay = copy.deepcopy(replay)
    replay["workload_identity_schema_version"] = 2
    replay["decisions"]["calibration_identity_sha256"] = provenance[
        "calibration_identity_sha256"
    ]
    replay["decisions"]["version_identity"] = copy.deepcopy(
        provenance["version_identity"]
    )
    with tempfile.TemporaryDirectory() as directory:
        manifest_path = pathlib.Path(directory) / "controlled-manifest.toml"
        manifest_path.write_text(replay["controlled_manifest_utf8"])
        manifest = opcode_gas.load_manifest(manifest_path)
        cases = {case.name: case for case in manifest.cases}
        relations = {
            relation.id: relation for relation in manifest.opcode_relations
        }
        formal_provenance = {
            field: provenance[field]
            for field in opcode_gas.FORMAL_RELATION_PROVENANCE_FIELDS
        }
        for embedded, source in zip(
            replay["rounds"], replay["decisions"]["rounds"]
        ):
            for row in embedded["rows"]:
                relation = relations[row["relation_id"]]
                canonical = opcode_gas._canonical_formal_relation_fixture_pair(
                    manifest,
                    cases[relation.case_id],
                    relation,
                    provenance=formal_provenance,
                    generator_max_count=row["generator_max_count"],
                    count=row["diagnostic_count"],
                    placement=row["relation_placement"],
                )[row["lane"]][0]
                row.update(canonical)
                guest_input = opcode_gas._formal_opcode_guest_input(row)
                workload_id = opcode_gas.controlled_workload_id(
                    opcode_gas._formal_opcode_workload_spec(guest_input)
                )
                row["workload_id"] = workload_id
                row["controlled_trace"]["schema_version"] = 3
                row["controlled_trace"]["workload_id"] = workload_id
                row["execution_row_id"] = opcode_gas.controlled_execution_row_id(
                    workload_id,
                    backend="sp1",
                    execution_engine=row["sp1_execution_engine"],
                    run_id=row["calibration_id"],
                    repeat_index=row["repeat_index"],
                    backend_input_sha256=row["backend_input_sha256"],
                )
            embedded["rows_sha256"] = context.sha256_bytes(
                context.canonical_json(embedded["rows"])
            )
            raw_bytes = b"".join(
                context.canonical_json(row) + b"\n" for row in embedded["rows"]
            )
            raw_sha256 = context.sha256_bytes(raw_bytes)
            embedded["source_raw_file_sha256"] = raw_sha256
            source["raw_runs_sha256"] = raw_sha256
    replay["decisions_sha256"] = context.sha256_bytes(
        context.canonical_json(replay["decisions"])
    )
    return replay


def materialize_context_canary_source(calibration, canary):
    root = pathlib.Path(calibration) / "context-compatibility"
    anchor = root / "control-anchors"
    fixtures = anchor / "fixtures"
    fixtures.mkdir(parents=True)
    replay = canary["current_anchor"]["replay"]
    (fixtures / "anchor-probe-manifest.json").write_bytes(
        context.canonical_json(replay["fixture_manifest"]) + b"\n"
    )
    for record in replay["fixture_inputs"]:
        path = fixtures / record["guest_input_path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(context.canonical_json(record["input"]) + b"\n")
    rows = canary["current_anchor"]["rows"]
    (anchor / "raw.jsonl").write_bytes(
        b"".join(context.canonical_json(row) + b"\n" for row in rows)
    )
    (anchor / "fit.json").write_text(
        json.dumps(canary["current_anchor"]["fit"], indent=2, sort_keys=True)
        + "\n"
    )
    reports = []
    for row in rows:
        reports.append(
            {
                "input": str(fixtures / row["guest_input_path"]),
                "exit_code": row["exit_code"],
                "gas": row["prover_gas"],
                "total_instruction_count": row["total_instruction_count"],
                "total_syscall_count": row["total_syscall_count"],
                "public_values": row["public_values"],
                "guest_input_sha256": row["guest_input_sha256"],
                "guest_input_bincode_length": row[
                    "guest_input_bincode_length"
                ],
                "sp1_execution_engine": row["sp1_execution_engine"],
                "sp1_gas_trace_chunk_threshold": row[
                    "sp1_gas_trace_chunk_threshold"
                ],
                "sp1_gas_trace_chunk_slots": row[
                    "sp1_gas_trace_chunk_slots"
                ],
            }
        )
    executor_bytes = b"".join(
        context.canonical_json(report) + b"\n" for report in reports
    )
    (anchor / "raw.guest-launcher.jsonl").write_bytes(executor_bytes)
    canary["current_anchor"]["source_ledger"][
        "executor_reports_file_sha256"
    ] = context.sha256_bytes(executor_bytes)
    canary["artifact_sha256"] = context.sha256_bytes(
        context.canonical_json(
            {key: value for key, value in canary.items() if key != "artifact_sha256"}
        )
    )
    shutil.copytree(
        TEST_CALIBRATION_RUN / "osaka-opcode-supplement",
        root / "legacy-osaka",
    )
    legacy_root = root / "legacy-osaka"
    legacy_replay = canary["legacy_osaka_replay"]
    decisions_bytes = (
        json.dumps(legacy_replay["decisions"], indent=2, sort_keys=True) + "\n"
    ).encode()
    (legacy_root / "canary-decisions.json").write_bytes(decisions_bytes)
    (legacy_root / "canary-decisions.sha256").write_text(
        context.sha256_bytes(decisions_bytes) + "\n"
    )
    for embedded, source in zip(
        legacy_replay["rounds"], legacy_replay["decisions"]["rounds"]
    ):
        raw_path = legacy_root / source["raw_runs"]
        result_path = legacy_root / source["result"]
        raw_path.parent.mkdir(parents=True, exist_ok=True)
        result_path.parent.mkdir(parents=True, exist_ok=True)
        raw_path.write_bytes(
            b"".join(
                context.canonical_json(row) + b"\n"
                for row in embedded["rows"]
            )
        )
        result_path.write_text(
            json.dumps(embedded["result"], indent=2, sort_keys=True) + "\n"
        )
    (root / "compatibility-canary.json").write_text(
        json.dumps(canary, indent=2, sort_keys=True) + "\n"
    )
    return root, reports


def passing_production_adaptive_evidence(manifest, rows, source_identity):
    synthetic = context.synthetic_passing_adaptive_evidence(
        manifest, rows, source_identity
    )
    identity = copy.deepcopy(synthetic["identity"])
    identity["evidence_mode"] = "production_execution"
    identity["runner_contract"] = "canonical_release_guest_launcher_v1"
    identity["source"] = production_campaign_source(source_identity)
    evidence = context.build_context_adaptive_evidence(
        manifest=manifest,
        identity=identity,
        rounds=synthetic["rounds"],
    )
    decisions = {
        "schema_version": 1,
        "identity_sha256": context.sha256_bytes(context.canonical_json(identity)),
        "rounds": [],
    }
    executor_hashes = []
    for round_evidence in evidence["rounds"]:
        raw_bytes = b"".join(
            context.canonical_json(row) + b"\n" for row in round_evidence["rows"]
        )
        fit_bytes = context.canonical_json(round_evidence["fit"]) + b"\n"
        executor_hash = context.sha256_bytes(
            context.canonical_json(
                [row["formal_report"] for row in round_evidence["rows"]]
            )
        )
        executor_hashes.append(executor_hash)
        decisions["rounds"].append(
            {
                "generator_bound": round_evidence["generator_bound"],
                "selected_scenarios": round_evidence["selected_scenarios"],
                "raw_rows": f"raw/context-round-{round_evidence['generator_bound']}.jsonl",
                "raw_rows_sha256": context.sha256_bytes(raw_bytes),
                "fit": f"fit/context-round-{round_evidence['generator_bound']}.json",
                "fit_sha256": context.sha256_bytes(fit_bytes),
                "executor_reports": (
                    f"executor/context-round-{round_evidence['generator_bound']}.jsonl"
                ),
                "executor_reports_sha256": executor_hash,
                "terminal_decisions": [
                    {
                        "scenario": name,
                        "status": round_evidence["fit"][name]["status"],
                    }
                    for name in round_evidence["selected_scenarios"]
                ],
                "next_scenarios": round_evidence["next_scenarios"],
            }
        )
    decisions_bytes = context.canonical_json(decisions) + b"\n"
    rows_bytes = b"".join(
        context.canonical_json(row) + b"\n"
        for row in evidence["terminal"]["rows"]
    )
    terminal = {
        "schema_version": 1,
        "status": "complete",
        "identity_sha256": decisions["identity_sha256"],
        "decisions_sha256": context.sha256_bytes(decisions_bytes),
        "rows_sha256": context.sha256_bytes(rows_bytes),
        "scenario_reports": evidence["terminal"]["scenario_reports"],
    }
    unsigned = dict(evidence)
    unsigned.pop("artifact_sha256")
    unsigned["production_run_ledger"] = {
        "schema_version": 1,
        "purpose": "context_create_only_run_ledger",
        "identity_file_sha256": context.sha256_bytes(
            context.canonical_json(identity) + b"\n"
        ),
        "decisions": decisions,
        "decisions_file_sha256": context.sha256_bytes(decisions_bytes),
        "decisions_seal_sha256": context.sha256_bytes(
            (context.sha256_bytes(decisions_bytes) + "\n").encode()
        ),
        "terminal": terminal,
        "terminal_file_sha256": context.sha256_bytes(
            context.canonical_json(terminal) + b"\n"
        ),
        "rows_file_sha256": context.sha256_bytes(rows_bytes),
        "executor_source_file_sha256s": executor_hashes,
        "portable_verification_scope": (
            "integrity_and_exact_fit_replay_not_prover_gas_reauthentication"
        ),
    }
    unsigned["artifact_sha256"] = context.sha256_bytes(
        context.canonical_json(unsigned)
    )
    return unsigned


def production_campaign_source(source_identity):
    calibration_identity = copy.deepcopy(TEST_CALIBRATION_IDENTITY)
    guest_artifacts = calibration_identity["guest_artifacts"]
    context_elf_sha256 = source_identity["context_elf_sha256"]
    legacy_revm_elf_sha256 = source_identity["legacy_revm_elf_sha256"]
    guest_artifacts["crates/guests/elf/sp1_context_opcode_lab.elf"] = (
        context_elf_sha256
    )
    guest_artifacts["crates/guests/elf/sp1_context_opcode_lab.vk.bin"] = (
        guest_artifacts["crates/guests/elf/sp1_revm_opcode_lab.vk.bin"]
    )
    guest_artifacts[context.FROZEN_LEGACY_REVM_ELF_PATH] = (
        legacy_revm_elf_sha256
    )
    guest_artifacts[context.FROZEN_LEGACY_REVM_VK_PATH] = guest_artifacts[
        "crates/guests/elf/sp1_revm_opcode_lab.vk.bin"
    ]
    guest_artifacts[context.FROZEN_LEGACY_REVM_PROVENANCE_PATH] = (
        context.FROZEN_LEGACY_REVM_PROVENANCE_SHA256
    )
    calibration_identity["guest_artifacts_sha256"] = context.sha256_bytes(
        context.canonical_json(guest_artifacts)
    )
    if (
        context_elf_sha256
        != guest_artifacts["crates/guests/elf/sp1_context_opcode_lab.elf"]
        or legacy_revm_elf_sha256
        != guest_artifacts[context.FROZEN_LEGACY_REVM_ELF_PATH]
        or source_identity["control_opcode_lab_elf_sha256"]
        != guest_artifacts["crates/guests/elf/sp1_opcode_lab.elf"]
        or source_identity["launcher_sha256"]
        != calibration_identity["guest_launcher_sha256"]
    ):
        raise ValueError("test source differs from frozen calibration")
    calibration_hash = context.sha256_bytes(
        context.canonical_json(calibration_identity)
    )
    return {
        "evidence_mode": "production_execution",
        "calibration_id": calibration_hash[:24],
        "calibration_identity_sha256": calibration_hash,
        "calibration_identity": calibration_identity,
        "implementation_revision": calibration_identity["implementation_revision"],
        "launcher_sha256": source_identity["launcher_sha256"],
        "legacy_revm_elf_sha256": legacy_revm_elf_sha256,
        "context_elf_sha256": context_elf_sha256,
        "control_opcode_lab_elf_sha256": source_identity[
            "control_opcode_lab_elf_sha256"
        ],
    }


class ContextFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = context.load_context_manifest(MANIFEST_PATH)

    def test_manifest_freezes_complete_required_siblings(self):
        by_opcode = {}
        for scenario in self.manifest["scenarios"]:
            by_opcode.setdefault(scenario["opcode"], []).append(scenario["name"])
        self.assertEqual(
            by_opcode,
            {
                0x30: ["address_canonical"],
                0x33: ["caller_canonical"],
                0x34: ["callvalue_zero", "callvalue_nonzero"],
                0x35: [
                    "calldataload_empty",
                    "calldataload_in_range",
                    "calldataload_partial",
                    "calldataload_out_of_range",
                ],
                0x36: [
                    "calldatasize_0",
                    "calldatasize_1",
                    "calldatasize_31",
                    "calldatasize_32",
                    "calldatasize_33",
                ],
                0x42: ["timestamp_zero", "timestamp_nonzero"],
            },
        )
        adaptive = self.manifest["adaptive_round_contract"]
        self.assertEqual(adaptive["generator_rounds"], [8, 32, 128, 512, 2048])
        self.assertEqual(adaptive["prefixes"][0], [0, 1, 2, 4])
        self.assertEqual(adaptive["checkpoints"][0], 8)
        self.assertEqual(self.manifest["repeats"], 3)
        canary = self.manifest["compatibility_canary"]
        self.assertEqual(canary["control_anchor_opcodes"], [0x5F, 0x90])
        self.assertNotEqual(
            canary["legacy_revm_opcode_lab_elf_path"],
            canary["control_opcode_lab_elf_path"],
        )
        self.assertNotEqual(
            canary["legacy_revm_opcode_lab_elf_path"],
            canary["context_opcode_lab_elf_path"],
        )
        self.assertEqual(canary["context_transport"], context.CONTEXT_TRANSPORT_STATUS)
        self.assertEqual(
            self.manifest["execution"],
            {
                "elf_path": "crates/guests/elf/sp1_context_opcode_lab.elf",
                "evm_spec": "osaka",
                "mode": "execute",
                "proof_type": "sp1",
                "sp1_execution_engine": "gas-estimator",
                "stage": "context-opcode-lab",
            },
        )

    def test_manifest_rejects_every_frozen_gate_and_scenario_mutation(self):
        original = json.loads(MANIFEST_PATH.read_text())
        mutations = {
            "noise_floor": lambda value: value["controlled_run_noise_floor"].update(
                {"prover_gas": 0}
            ),
            "noise_provenance": lambda value: value[
                "controlled_run_noise_floor"
            ].update({"provenance_contract_id": "forged"}),
            "consistency": lambda value: value.update(
                {"consistency_ape_max": "100"}
            ),
            "tx_gas": lambda value: value.update({"tx_gas_limit": 100_000}),
            "scenario": lambda value: value["scenarios"][6].update(
                {"offset": 18}
            ),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                broken = copy.deepcopy(original)
                mutate(broken)
                path = pathlib.Path(directory) / "manifest.json"
                path.write_text(json.dumps(broken))
                with self.assertRaises(ValueError):
                    context.load_context_manifest(path)

    def test_fixtures_bind_resolved_environment_and_fixed_same_shape_controls(self):
        for scenario in self.manifest["scenarios"]:
            target = context.generate_context_fixture(
                self.manifest, scenario["name"], "target", 8
            )
            control = context.generate_context_fixture(
                self.manifest, scenario["name"], "control", 8
            )
            self.assertEqual(len(bytes.fromhex(target["bytecode"][2:])), len(bytes.fromhex(control["bytecode"][2:])))
            for key in ("tx_value", "calldata", "block_timestamp", "tx_gas_limit"):
                self.assertEqual(target[key], control[key])
            self.assertEqual(target["identity_contract"]["schema_version"], 2)
            self.assertEqual(target["trace_schema_version"], 3)
            self.assertNotEqual(target["bytecode"], control["bytecode"])

        target = context.generate_context_fixture(
            self.manifest, "calldataload_partial", "target", 8
        )
        control = context.generate_context_fixture(
            self.manifest, "calldataload_partial", "control", 8
        )
        self.assertEqual(bytes.fromhex(target["active_program"])[-4], 0x35)
        self.assertEqual(bytes.fromhex(control["active_program"])[-4], 0x90)
        self.assertNotEqual(bytes.fromhex(control["active_program"])[-4], 0x19)
        self.assertTrue(target["active_program"].endswith("505000"))
        self.assertTrue(control["active_program"].endswith("505000"))

    def test_explicit_zero_timestamp_is_not_legacy_omission(self):
        zero = context.generate_context_fixture(
            self.manifest, "timestamp_zero", "target", 1
        )
        nonzero = context.generate_context_fixture(
            self.manifest, "timestamp_nonzero", "target", 1
        )
        self.assertEqual(zero["block_timestamp"], 0)
        self.assertEqual(nonzero["block_timestamp"], 17)
        self.assertNotEqual(zero["environment_sha256"], nonzero["environment_sha256"])

    def test_production_executor_and_identity_use_dedicated_context_stages(self):
        calls = []

        def fake_run(command, *, check):
            self.assertTrue(check)
            calls.append(command)
            if "context-opcode-identity" in command:
                output = pathlib.Path(command[command.index("--json-out") + 1])
                output.write_text("{}\n")

        with tempfile.TemporaryDirectory() as directory, mock.patch.object(
            context.subprocess, "run", side_effect=fake_run
        ):
            root = pathlib.Path(directory)
            fixture = root / "fixture.json"
            fixture.write_text("{}\n")
            context._default_context_executor(
                fixtures=[fixture],
                reports_jsonl=root / "reports.jsonl",
                guest_launcher=root / "guest-launcher",
                elf=root / "sp1_context_opcode_lab.elf",
            )
            context._default_identity_replayer(root / "guest-launcher", fixture)

        self.assertEqual(calls[0][calls[0].index("--stage") + 1], "context-opcode-lab")
        self.assertEqual(
            calls[1][calls[1].index("--stage") + 1], "context-opcode-identity"
        )


class ContextFitTests(unittest.TestCase):
    def test_scaled_body_recovery_is_exact_and_does_not_double_count_dispatch(self):
        recovered = context.recover_scaled_target_body(
            delta_lab=Fraction(10),
            body_scale=Fraction(3),
            target_raw_gas=2,
            control_raw_gas=2,
            stored_control_body=Fraction(20),
        )
        self.assertEqual(recovered, Fraction(35))
        self.assertEqual(
            context.predict_static_event(
                common_dispatch=Fraction(7), raw_gas=2, stored_body=recovered
            ),
            Fraction(77),
        )

    def test_control_must_be_sealed_measured_static_model(self):
        registry = json.loads(REGISTRY_PATH.read_text())
        push0 = context.load_control_reference(registry, 0x5F)
        swap1 = context.load_control_reference(registry, 0x90)
        self.assertEqual(push0["model_kind"], "static_raw_gas")
        self.assertEqual(swap1["model_kind"], "static_raw_gas")
        broken = copy.deepcopy(registry)
        broken["registry"]["models"]["opcode:0x90"] = {
            "kind": "static_raw_gas",
            "parameters": {"body_per_raw_gas": "0"},
        }
        broken["approximation_policy"]["dispatch_only_opcode_keys"].append("opcode:0x90")
        with self.assertRaisesRegex(ValueError, "approximation"):
            context.load_control_reference(broken, 0x90)

    def test_missing_or_failed_sibling_rejects_whole_opcode(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        reports = context.synthetic_passing_scenario_reports(manifest)
        reports["callvalue_nonzero"]["status"] = "failed"
        selected = context.select_promotable_context_keys(manifest, reports)
        self.assertNotIn("opcode:0x34", selected)
        self.assertIn("opcode:0x30", selected)
        del reports["calldataload_partial"]
        selected = context.select_promotable_context_keys(manifest, reports)
        self.assertNotIn("opcode:0x35", selected)

    def test_raw_row_admission_rejects_wrong_scenario_environment_or_identity(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        row = context.synthetic_passing_rows(manifest)[0]
        context.admit_context_row(manifest, row)
        for mutation in ("scenario", "environment", "identity"):
            broken = copy.deepcopy(row)
            if mutation == "scenario":
                broken["scenario"] = "caller_canonical"
            elif mutation == "environment":
                broken["fixture"]["block_timestamp"] = 17
            else:
                broken["identity"]["block_environment_sha256"] = "0" * 64
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                context.admit_context_row(manifest, broken)

    def test_raw_rows_refit_exact_adaptive_slope_with_count_zero_and_checkpoint(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        reports = context.fit_context_campaign_rows(
            manifest, context.synthetic_passing_rows(manifest)
        )
        report = reports["address_canonical"]
        self.assertEqual(report["selected_counts"], [0, 1, 2, 4])
        self.assertEqual(report["checkpoint_count"], 8)
        self.assertEqual(
            context.fraction_from_payload(report["delta_lab_exact"]), Fraction(100)
        )
        self.assertEqual(report["status"], "passed")

    def test_fit_enforces_r2_stderr_repeat_signal_activation_and_tail_gates(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        rows = context.synthetic_passing_rows(manifest)
        scenario = "address_canonical"

        noisy = copy.deepcopy(rows)
        row = next(
            row for row in noisy
            if row["scenario"] == scenario and row["count"] == 2
            and row["repeat_index"] == 1 and row["lane"] == "target"
            and row["placement"] == "active_prefix"
        )
        row["prover_gas"] += 1
        report = context.fit_context_campaign_rows(manifest, noisy)[scenario]
        self.assertEqual(report["quality_gates"]["repeat_noise"], "failed")

        nonlinear = copy.deepcopy(rows)
        for row in nonlinear:
            if (
                row["scenario"] == scenario
                and row["placement"] == "active_prefix"
                and row["lane"] == "target"
                and row["count"] == 2
            ):
                row["prover_gas"] += 400
        report = context.fit_context_campaign_rows(manifest, nonlinear)[scenario]
        self.assertEqual(report["quality_gates"]["r2"], "failed")
        self.assertEqual(report["quality_gates"]["slope_stderr"], "failed")

        tail = copy.deepcopy(rows)
        for row in tail:
            if row["scenario"] != scenario:
                continue
            if (
                row["placement"] == "active_prefix"
                and row["lane"] == "target"
                and row["count"] > 0
            ):
                row["prover_gas"] += 1000
            if row["placement"] == "active_tail" and row["lane"] == "target":
                row["prover_gas"] += 2000
        report = context.fit_context_campaign_rows(manifest, tail)[scenario]
        self.assertTrue(report["tail_holdout"]["triggered"])
        self.assertEqual(report["quality_gates"]["tail_holdout"], "failed")

    def test_adaptive_runner_freezes_passed_scenarios_and_advances_only_failed(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        calls = []

        def executor(*, fixtures, reports_jsonl, **_kwargs):
            calls.append([pathlib.Path(path) for path in fixtures])
            reports = context.synthetic_execution_reports(fixtures, fail_first_round="caller_canonical")
            reports_jsonl.parent.mkdir(parents=True, exist_ok=True)
            reports_jsonl.write_bytes(
                b"".join(context.canonical_json(report) + b"\n" for report in reports)
            )

        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            result = context.run_context_opcode_campaign(
                manifest_path=MANIFEST_PATH,
                run=root / "run",
                fixtures_root=root / "fixtures",
                guest_launcher=ROOT / "target/release/guest-launcher",
                elf=ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf",
                executor=executor,
                identity_replayer=context.synthetic_identity_replayer,
                source_validator=lambda **_kwargs: {"source": "test"},
            )
            self.assertEqual(len(calls), 2)
            self.assertTrue(all("caller_canonical" in str(path) for path in calls[1]))
            self.assertTrue(any("address_canonical" in str(path) for path in calls[0]))
            self.assertFalse(any("address_canonical" in str(path) for path in calls[1]))
            self.assertEqual(result["status"], "complete")
            replay = context.run_context_opcode_campaign(
                manifest_path=MANIFEST_PATH,
                run=root / "run",
                fixtures_root=root / "fixtures",
                guest_launcher=ROOT / "target/release/guest-launcher",
                elf=ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf",
                executor=lambda **_kwargs: self.fail("resume re-executed accepted rows"),
                identity_replayer=context.synthetic_identity_replayer,
                source_validator=lambda **_kwargs: {"source": "test"},
            )
            self.assertEqual(replay, result)
            evidence = context.load_context_adaptive_evidence(
                root / "run", manifest
            )
            self.assertEqual(
                evidence["identity"]["evidence_mode"], "synthetic_test_only"
            )
            self.assertEqual(
                evidence["identity"]["runner_contract"], "injected_test_hooks"
            )
            with self.assertRaisesRegex(ValueError, "synthetic"):
                context.validate_context_adaptive_evidence(
                    manifest, evidence, {}
                )
            self.assertEqual(
                [round_row["generator_bound"] for round_row in evidence["rounds"]],
                [8, 32],
            )
            self.assertIn(
                "caller_canonical", evidence["rounds"][0]["next_scenarios"]
            )
            self.assertNotIn(
                "address_canonical", evidence["rounds"][0]["next_scenarios"]
            )
            self.assertEqual(
                evidence["terminal"]["scenario_reports"],
                result["scenario_reports"],
            )

    def test_adaptive_runner_reuses_complete_orphan_executor_output(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        calls = 0

        def crashing_executor(*, fixtures, reports_jsonl, **_kwargs):
            nonlocal calls
            calls += 1
            reports = context.synthetic_execution_reports(fixtures)
            reports_jsonl.parent.mkdir(parents=True, exist_ok=True)
            reports_jsonl.write_bytes(
                b"".join(context.canonical_json(report) + b"\n" for report in reports)
            )
            raise RuntimeError("crash after executor output")

        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            kwargs = {
                "manifest_path": MANIFEST_PATH,
                "run": root / "run",
                "fixtures_root": root / "fixtures",
                "guest_launcher": ROOT / "target/release/guest-launcher",
                "elf": ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf",
                "control_opcode_lab_elf": ROOT
                / "crates/guests/elf/sp1_opcode_lab.elf",
                "identity_replayer": context.synthetic_identity_replayer,
                "source_validator": lambda **_kwargs: {"source": "test"},
            }
            with self.assertRaisesRegex(RuntimeError, "crash after"):
                context.run_context_opcode_campaign(
                    **kwargs, executor=crashing_executor
                )
            result = context.run_context_opcode_campaign(
                **kwargs,
                executor=lambda **_kwargs: self.fail(
                    "complete orphan output was re-executed"
                ),
            )
            self.assertEqual(result["status"], "complete")
            self.assertEqual(calls, 1)

    def test_adaptive_runner_recovers_each_persisted_round_phase(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        for crash_phase in (
            "after_executor_output",
            "after_round_rows",
            "after_decision_ledger",
            "after_decision_seal",
        ):
            with self.subTest(phase=crash_phase), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                calls = 0

                def executor(*, fixtures, reports_jsonl, **_kwargs):
                    nonlocal calls
                    calls += 1
                    reports_jsonl.parent.mkdir(parents=True, exist_ok=True)
                    reports_jsonl.write_bytes(
                        b"".join(
                            context.canonical_json(report) + b"\n"
                            for report in context.synthetic_execution_reports(fixtures)
                        )
                    )

                def phase_hook(phase):
                    if phase == crash_phase:
                        raise RuntimeError(f"crash at {phase}")

                kwargs = {
                    "manifest_path": MANIFEST_PATH,
                    "run": root / "run",
                    "fixtures_root": root / "fixtures",
                    "guest_launcher": ROOT / "target/release/guest-launcher",
                    "elf": ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf",
                    "control_opcode_lab_elf": ROOT
                    / "crates/guests/elf/sp1_opcode_lab.elf",
                    "identity_replayer": context.synthetic_identity_replayer,
                    "source_validator": lambda **_kwargs: {"source": "test"},
                }
                with self.assertRaisesRegex(RuntimeError, crash_phase):
                    context.run_context_opcode_campaign(
                        **kwargs, executor=executor, phase_hook=phase_hook
                    )
                result = context.run_context_opcode_campaign(
                    **kwargs,
                    executor=lambda **_kwargs: self.fail(
                        f"{crash_phase} recovery re-executed persisted output"
                    ),
                )
                self.assertEqual(result["status"], "complete")
                self.assertEqual(calls, 1)
                context.load_context_adaptive_evidence(root / "run", manifest)

    def test_adaptive_runner_rejects_descendant_symlink_before_any_write(self):
        for escaped_root in ("fixtures", "run"):
            with self.subTest(root=escaped_root), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                outside = root / "outside"
                outside.mkdir()
                run = root / "run"
                fixtures = root / "fixtures"
                run.mkdir()
                fixtures.mkdir()
                if escaped_root == "fixtures":
                    (fixtures / "address_canonical").symlink_to(outside)
                else:
                    (run / "executor").symlink_to(outside)
                executor = mock.Mock()
                with self.assertRaisesRegex(ValueError, "symlink"):
                    context.run_context_opcode_campaign(
                        manifest_path=MANIFEST_PATH,
                        run=run,
                        fixtures_root=fixtures,
                        guest_launcher=ROOT / "target/release/guest-launcher",
                        elf=ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf",
                        control_opcode_lab_elf=ROOT
                        / "crates/guests/elf/sp1_opcode_lab.elf",
                        executor=executor,
                        identity_replayer=context.synthetic_identity_replayer,
                        source_validator=lambda **_kwargs: {"source": "test"},
                    )
                executor.assert_not_called()
                self.assertEqual(list(outside.iterdir()), [])

    def test_terminal_run_never_heals_missing_history_or_reexecutes(self):
        manifest = context.load_context_manifest(MANIFEST_PATH)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            kwargs = {
                "manifest_path": MANIFEST_PATH,
                "run": root / "run",
                "fixtures_root": root / "fixtures",
                "guest_launcher": ROOT / "target/release/guest-launcher",
                "elf": ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf",
                "control_opcode_lab_elf": ROOT
                / "crates/guests/elf/sp1_opcode_lab.elf",
                "identity_replayer": context.synthetic_identity_replayer,
                "source_validator": lambda **_kwargs: {"source": "test"},
            }
            context.run_context_opcode_campaign(
                **kwargs,
                executor=lambda **call: call["reports_jsonl"].parent.mkdir(
                    parents=True, exist_ok=True
                )
                or call["reports_jsonl"].write_bytes(
                    b"".join(
                        context.canonical_json(report) + b"\n"
                        for report in context.synthetic_execution_reports(
                            call["fixtures"]
                        )
                    )
                ),
            )
            (root / "run" / "decisions.json").unlink()
            (root / "run" / "decisions.sha256").unlink()
            executor = mock.Mock()
            with self.assertRaises(ValueError):
                context.run_context_opcode_campaign(**kwargs, executor=executor)
            executor.assert_not_called()
            self.assertFalse((root / "run" / "decisions.json").exists())

    def test_historical_anchor_portable_validation_does_not_require_live_old_head(self):
        baseline = ROOT / "experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02"
        historical = ROOT / "experiments/opcode-gas/runs/09ebb08d76d3f461086b0cf4"
        validated = context.validate_portable_historical_anchor_source(
            historical, baseline
        )
        self.assertEqual(validated["calibration_id"], "09ebb08d76d3f461086b0cf4")
        self.assertIn("opcode:0x5f", validated["costs"])
        self.assertIn("opcode:0x90", validated["costs"])

    def test_canary_runner_rejects_descendant_symlink_before_preflight_or_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            calibration = root / "calibration"
            output = calibration / "context-compatibility"
            outside = root / "outside"
            (output / "legacy-osaka").mkdir(parents=True)
            outside.mkdir()
            (output / "legacy-osaka" / "raw").symlink_to(outside)
            with mock.patch.object(
                opcode_gas,
                "validate_calibration_execution_identity",
                side_effect=AssertionError("preflight must not run"),
            ) as preflight, self.assertRaisesRegex(ValueError, "symlink"):
                context.run_context_compatibility_canary(
                    calibration_run=calibration,
                    controlled_manifest=pathlib.Path("unused"),
                    baseline_derivation=pathlib.Path("unused"),
                    historical_manifest=pathlib.Path("unused"),
                    historical_anchor_run=pathlib.Path("unused"),
                    guest_launcher=pathlib.Path("unused"),
                    legacy_revm_elf=pathlib.Path("unused"),
                    context_elf=pathlib.Path("unused"),
                    control_opcode_lab_elf=pathlib.Path("unused"),
                    output_root=output,
                )
            preflight.assert_not_called()
            self.assertEqual(list(outside.iterdir()), [])

    def test_compatibility_canary_requires_existing_set_and_both_controls(self):
        canary = passing_production_canary()
        context.validate_context_compatibility_canary(canary)
        del canary["control_relations"]["opcode:0x90"]
        with self.assertRaisesRegex(ValueError, "control relation"):
            context.validate_context_compatibility_canary(canary)

    def test_production_source_requires_complete_legacy_package_in_calibration(self):
        source = production_campaign_source(
            {
                "legacy_revm_elf_sha256": HISTORICAL_LEGACY_ELF_SHA256,
                "context_elf_sha256": TEST_CONTEXT_ELF_SHA256,
                "control_opcode_lab_elf_sha256": (
                    HISTORICAL_CONTROL_ELF_SHA256
                ),
                "launcher_sha256": HISTORICAL_LAUNCHER_SHA256,
            }
        )
        context._validate_production_campaign_source(source)
        for missing in (
            context.FROZEN_LEGACY_REVM_VK_PATH,
            context.FROZEN_LEGACY_REVM_PROVENANCE_PATH,
        ):
            with self.subTest(missing=missing):
                forged = copy.deepcopy(source)
                artifacts = forged["calibration_identity"]["guest_artifacts"]
                del artifacts[missing]
                forged["calibration_identity"]["guest_artifacts_sha256"] = (
                    context.sha256_bytes(context.canonical_json(artifacts))
                )
                calibration_hash = context.sha256_bytes(
                    context.canonical_json(forged["calibration_identity"])
                )
                forged["calibration_identity_sha256"] = calibration_hash
                forged["calibration_id"] = calibration_hash[:24]
                with self.assertRaisesRegex(ValueError, "calibration identity"):
                    context._validate_production_campaign_source(forged)

    def test_compatibility_canary_binds_frozen_legacy_provenance(self):
        canary = passing_production_canary()
        self.assertEqual(
            canary["legacy_revm_provenance_sha256"],
            context.FROZEN_LEGACY_REVM_PROVENANCE_SHA256,
        )
        canary["legacy_revm_provenance_sha256"] = "f" * 64
        canary["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {key: value for key, value in canary.items() if key != "artifact_sha256"}
            )
        )
        with self.assertRaisesRegex(ValueError, "legacy compatibility canary"):
            context.validate_context_compatibility_canary(canary)

    def test_production_canary_rejects_legacy_workload_identity_replay(self):
        canary = passing_production_canary()
        canary["legacy_osaka_replay"]["workload_identity_schema_version"] = 1
        canary["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in canary.items()
                    if key != "artifact_sha256"
                }
            )
        )

        with self.assertRaisesRegex(ValueError, "current workload identity"):
            context.validate_context_compatibility_canary(canary)

    def test_compatibility_canary_uses_exact_ape_and_distinct_guest_artifacts(self):
        canary = passing_production_canary()
        self.assertEqual(
            canary["purpose"],
            "legacy_opcode_reuse_canary_with_context_binding",
        )
        row = canary["control_relations"]["opcode:0x5f"]
        self.assertEqual(
            set(row["drift_ape_exact"]), {"numerator", "denominator", "decimal"}
        )
        self.assertNotEqual(
            canary["legacy_revm_elf_sha256"],
            canary["control_opcode_lab_elf_sha256"],
        )
        self.assertNotEqual(
            canary["context_elf_sha256"], canary["legacy_revm_elf_sha256"]
        )
        self.assertNotEqual(
            canary["context_elf_sha256"],
            canary["control_opcode_lab_elf_sha256"],
        )
        self.assertRegex(canary["context_elf_sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual(
            canary["context_transport"],
            {
                "status": "not_evaluated",
                "scope": "cross_elf_context_to_legacy_cost_transport",
                "required_before_candidate_promotion": True,
            },
        )
        transport_forgery = copy.deepcopy(canary)
        transport_forgery["context_transport"]["status"] = "supported"
        transport_forgery["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in transport_forgery.items()
                    if key != "artifact_sha256"
                }
            )
        )
        with self.assertRaisesRegex(ValueError, "transport"):
            context.validate_context_compatibility_canary(transport_forgery)
        aliased_context = copy.deepcopy(canary)
        aliased_context["context_elf_sha256"] = aliased_context[
            "legacy_revm_elf_sha256"
        ]
        aliased_context["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in aliased_context.items()
                    if key != "artifact_sha256"
                }
            )
        )
        with self.assertRaisesRegex(ValueError, "distinct"):
            context.validate_context_compatibility_canary(aliased_context)
        broken = copy.deepcopy(canary)
        broken["control_relations"]["opcode:0x5f"]["drift_ape_exact"] = (
            context.fraction_payload(Fraction(1, 11))
        )
        broken["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {key: value for key, value in broken.items() if key != "artifact_sha256"}
            )
        )
        with self.assertRaisesRegex(ValueError, "control drift"):
            context.validate_context_compatibility_canary(broken)

        forged = copy.deepcopy(canary)
        forged["legacy_osaka_canary"]["relations"][0]["osaka_slope_p"] = "101"
        forged["legacy_osaka_canary"]["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in forged["legacy_osaka_canary"].items()
                    if key != "artifact_sha256"
                }
            )
        )
        forged["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {key: value for key, value in forged.items() if key != "artifact_sha256"}
            )
        )
        with self.assertRaisesRegex(ValueError, "Osaka compatibility canary"):
            context.validate_context_compatibility_canary(forged)

    def test_compatibility_canary_rejects_coherently_rehashed_source_forgery(self):
        canary = passing_production_canary()
        forged = copy.deepcopy(canary)
        historical = forged["legacy_osaka_replay"]["historical_observations"]
        historical[0]["slope_p"] = "999"
        forged["legacy_osaka_replay"]["historical_observations_sha256"] = (
            context.sha256_bytes(context.canonical_json(historical))
        )
        forged["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {key: value for key, value in forged.items() if key != "artifact_sha256"}
            )
        )
        with self.assertRaisesRegex(ValueError, "legacy Osaka replay"):
            context.validate_context_compatibility_canary(forged)

        forged = copy.deepcopy(canary)
        rows = forged["legacy_osaka_replay"]["rounds"][0]["rows"]
        rows[0]["prover_gas"] += 1
        forged["legacy_osaka_replay"]["rounds"][0]["rows_sha256"] = (
            context.sha256_bytes(context.canonical_json(rows))
        )
        forged["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {key: value for key, value in forged.items() if key != "artifact_sha256"}
            )
        )
        with self.assertRaises(ValueError):
            context.validate_context_compatibility_canary(forged)

        forged = copy.deepcopy(canary)
        del forged["current_anchor"]["replay"]
        forged["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {key: value for key, value in forged.items() if key != "artifact_sha256"}
            )
        )
        with self.assertRaisesRegex(ValueError, "current anchor replay"):
            context.validate_context_compatibility_canary(forged)

    def test_seal_rejects_synthetic_compatibility_marker(self):
        with self.assertRaises(ValueError):
            context.validate_context_compatibility_canary(
                context.synthetic_passing_compatibility_canary()
            )

class ContextSealAndPromotionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = context.load_context_manifest(MANIFEST_PATH)
        cls.registry = json.loads(CORRECTED_REGISTRY_PATH.read_text())
        cls.v5 = context.build_operation_coverage_v5(
            json.loads(COVERAGE_V4_PATH.read_text()),
            cls.registry,
            json.loads(CORRECTED_PACKAGE_PATH.read_text()),
        )

    def setUp(self):
        self.identity_replay_patcher = mock.patch.object(
            context, "_replay_context_result_identity", return_value=None
        )
        self.identity_replay = self.identity_replay_patcher.start()
        self.addCleanup(self.identity_replay_patcher.stop)

    @classmethod
    def source_identity_payload(cls):
        payload = {
            "legacy_revm_elf_sha256": HISTORICAL_LEGACY_ELF_SHA256,
            "legacy_revm_elf_path": context.FROZEN_LEGACY_REVM_ELF_PATH,
            "context_elf_sha256": TEST_CONTEXT_ELF_SHA256,
            "context_elf_path": "crates/guests/elf/sp1_context_opcode_lab.elf",
            "control_opcode_lab_elf_sha256": HISTORICAL_CONTROL_ELF_SHA256,
            "control_opcode_lab_elf_path": "crates/guests/elf/sp1_opcode_lab.elf",
            "launcher_sha256": HISTORICAL_LAUNCHER_SHA256,
            "launcher_path": "target/release/guest-launcher",
            "corrected_osaka_core_artifact_sha256": cls.registry["artifact_sha256"],
            "corrected_osaka_core_file_sha256": context.sha256_bytes(CORRECTED_REGISTRY_PATH.read_bytes()),
            "corrected_osaka_core_path": "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json",
            "operation_coverage_v5_artifact_sha256": cls.v5["artifact_sha256"],
            "operation_coverage_v5_file_sha256": context.sha256_bytes(
                (json.dumps(cls.v5, indent=2, sort_keys=True) + "\n").encode()
            ),
            "operation_coverage_v5_path": "experiments/opcode-gas/manifests/operation-coverage-v5.json",
        }
        source = production_campaign_source(payload)
        payload.update(
            {
                "calibration_id": source["calibration_id"],
                "calibration_identity_sha256": source[
                    "calibration_identity_sha256"
                ],
                "execution_revision": source["implementation_revision"],
            }
        )
        return payload

    @staticmethod
    def protected_inputs():
        anchor = ROOT / "experiments/opcode-gas/runs/09ebb08d76d3f461086b0cf4"
        return {
            "calibration_run": anchor,
            "run": anchor,
            "manifest": MANIFEST_PATH,
            "compatibility_canary": anchor / "anchor-probe-fit.json",
            "corrected_core": CORRECTED_REGISTRY_PATH,
            "coverage_v5": ROOT
            / "experiments/opcode-gas/manifests/operation-coverage-v5.json",
        }

    def seal(self, payload, root, *, protected_inputs=None):
        return context._publish_context_result(
            payload,
            root,
            protected_inputs=protected_inputs or self.protected_inputs(),
        )

    def result_payload(self):
        source_identity = self.source_identity_payload()
        rows = context.synthetic_passing_production_rows(self.manifest)
        return context._build_context_result(
            manifest=self.manifest,
            rows=rows,
            source_registry=self.registry,
            compatibility_canary=passing_production_canary(
                source_identity=source_identity
            ),
            source_identity=source_identity,
            adaptive_evidence=passing_production_adaptive_evidence(
                self.manifest, rows, source_identity
            ),
        )

    def test_result_rejects_synthetic_adaptive_evidence(self):
        source_identity = self.source_identity_payload()
        rows = context.synthetic_passing_production_rows(self.manifest)
        evidence = context.synthetic_passing_adaptive_evidence(
            self.manifest, rows, source_identity
        )
        with self.assertRaisesRegex(ValueError, "synthetic"):
            context._build_context_result(
                manifest=self.manifest,
                rows=rows,
                source_registry=self.registry,
                compatibility_canary=passing_production_canary(),
                source_identity=source_identity,
                adaptive_evidence=evidence,
            )

    def test_public_seal_rejects_loose_precomputed_result_mapping(self):
        payload = self.result_payload()
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(TypeError):
                context.seal_context_result(payload, pathlib.Path(directory))
            self.assertEqual(list(pathlib.Path(directory).iterdir()), [])
        self.assertEqual(
            payload["execution_evidence_authority"]["portable_verification_scope"],
            "integrity_and_exact_fit_replay_not_prover_gas_reauthentication",
        )

    def test_canonical_canary_file_without_runner_source_ledger_is_rejected(self):
        canary = passing_production_canary()
        with tempfile.TemporaryDirectory() as directory:
            calibration = pathlib.Path(directory) / "calibration"
            root = calibration / "context-compatibility"
            root.mkdir(parents=True)
            (root / "compatibility-canary.json").write_text(
                json.dumps(canary, indent=2, sort_keys=True) + "\n"
            )
            with self.assertRaisesRegex(ValueError, "persisted source|missing"):
                context._validate_context_canary_run_directory(
                    calibration, canary
                )

    def test_canary_directory_rejects_coherently_rehashed_forged_executor_report(self):
        canary = passing_production_canary()
        with tempfile.TemporaryDirectory() as directory:
            calibration = pathlib.Path(directory) / "calibration"
            root, _reports = materialize_context_canary_source(
                calibration, canary
            )
            context.validate_context_compatibility_canary(canary)
            context._validate_context_canary_run_directory(calibration, canary)

            forged = b'{"forged":true}\n'
            (root / "control-anchors/raw.guest-launcher.jsonl").write_bytes(
                forged
            )
            canary["current_anchor"]["source_ledger"][
                "executor_reports_file_sha256"
            ] = context.sha256_bytes(forged)
            canary["artifact_sha256"] = context.sha256_bytes(
                context.canonical_json(
                    {
                        key: value
                        for key, value in canary.items()
                        if key != "artifact_sha256"
                    }
                )
            )
            context.validate_context_compatibility_canary(canary)
            with self.assertRaisesRegex(ValueError, "executor source"):
                context._validate_context_canary_run_directory(
                    calibration, canary
                )

    def test_canary_seal_replay_rejects_all_consistently_wrong_public_values(self):
        canary = passing_production_canary()
        original_native = copy.deepcopy(
            canary["current_anchor"]["replay"]["native_input_identities"]
        )
        inputs = canary["current_anchor"]["replay"]["fixture_inputs"]
        native_by_input = {
            context.sha256_bytes(context.canonical_json(record["input"])): native
            for record, native in zip(inputs, original_native)
        }
        with tempfile.TemporaryDirectory() as directory:
            calibration = pathlib.Path(directory) / "calibration"
            root, reports = materialize_context_canary_source(
                calibration, canary
            )
            wrong = "0x" + "11" * 32
            rows = canary["current_anchor"]["rows"]
            for row in rows:
                row["public_values"] = wrong
                row["anchor_execution_row_id"] = (
                    opcode_gas._anchor_probe_execution_row_id(row)
                )
            for report in reports:
                report["public_values"] = wrong
            for record in canary["current_anchor"]["replay"][
                "native_input_identities"
            ]:
                record["expected_public_values"] = wrong
            fit = opcode_gas.fit_anchor_probe_rows(rows)
            canary["current_anchor"]["fit"] = fit
            canary["current_anchor"]["fit_artifact_sha256"] = fit[
                "artifact_sha256"
            ]
            canary["current_anchor"]["raw_rows_sha256"] = fit[
                "raw_rows_sha256"
            ]
            raw_bytes = b"".join(
                context.canonical_json(row) + b"\n" for row in rows
            )
            executor_bytes = b"".join(
                context.canonical_json(report) + b"\n" for report in reports
            )
            canary["current_anchor"]["raw_rows_file_sha256"] = (
                context.sha256_bytes(raw_bytes)
            )
            canary["current_anchor"]["source_ledger"] = (
                context._current_anchor_source_ledger(
                    canary["current_anchor"]["replay"],
                    rows,
                    fit,
                    context.sha256_bytes(executor_bytes),
                )
            )
            anchor = root / "control-anchors"
            (anchor / "raw.jsonl").write_bytes(raw_bytes)
            (anchor / "raw.guest-launcher.jsonl").write_bytes(executor_bytes)
            (anchor / "fit.json").write_text(
                json.dumps(fit, indent=2, sort_keys=True) + "\n"
            )
            canary["artifact_sha256"] = context.sha256_bytes(
                context.canonical_json(
                    {
                        key: value
                        for key, value in canary.items()
                        if key != "artifact_sha256"
                    }
                )
            )
            context.validate_context_compatibility_canary(canary)
            context._validate_context_canary_run_directory(calibration, canary)

            def real_anchor_identity(_launcher, input_path):
                value = json.loads(pathlib.Path(input_path).read_text())
                expected = native_by_input[
                    context.sha256_bytes(context.canonical_json(value))
                ]
                return {
                    "schema_version": 1,
                    "guest_input_sha256": "0x"
                    + expected["backend_input_sha256"],
                    "guest_input_bincode_length": expected[
                        "backend_input_len"
                    ],
                    "expected_public_values": expected[
                        "expected_public_values"
                    ],
                }

            with mock.patch.object(
                context,
                "_default_anchor_identity_replayer",
                side_effect=real_anchor_identity,
            ), self.assertRaisesRegex(ValueError, "native identity replay"):
                context._replay_current_anchor_native_inputs(
                    canary, pathlib.Path("unused-launcher")
                )

    def test_native_replay_rejects_coherently_rehashed_identity_forgery(self):
        source_identity = self.source_identity_payload()
        rows = context.synthetic_passing_production_rows(self.manifest)
        rows[0]["identity"]["identity"]["transaction_envelope_sha256"] = "f" * 64
        payload = context._build_context_result(
            manifest=self.manifest,
            rows=rows,
            source_registry=self.registry,
            compatibility_canary=passing_production_canary(
                source_identity=source_identity
            ),
            source_identity=source_identity,
            adaptive_evidence=passing_production_adaptive_evidence(
                self.manifest, rows, source_identity
            ),
        )
        with mock.patch.object(
            context,
            "_validated_context_identity_helper",
            return_value=pathlib.Path("unused-launcher"),
        ), mock.patch.object(
            context,
            "_default_identity_replayer",
            side_effect=lambda _launcher, input_path: context.synthetic_identity_replayer(
                input_path
            ),
        ):
            with self.assertRaisesRegex(ValueError, "native replay"):
                REAL_CONTEXT_IDENTITY_REPLAY(payload)

    def test_sealed_directory_replays_without_guest_execution_and_rejects_tamper(self):
        payload = self.result_payload()
        self.assertEqual(
            payload["context_transport"],
            payload["compatibility_canary"]["context_transport"],
        )
        self.assertFalse(payload["candidate_eligible"])
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            sealed = self.seal(payload, root)
            self.assertEqual(context.verify_context_result(sealed)["result_id"], sealed.name)
            self.assertEqual(self.identity_replay.call_count, 2)
            self.assertEqual(
                {path.name for path in sealed.iterdir()},
                context.CONTEXT_RESULT_INVENTORY,
            )
            (sealed / "rows.jsonl").write_text("{}\n")
            with self.assertRaisesRegex(ValueError, "hash"):
                context.verify_context_result(sealed)

    def test_seal_input_rejects_compatibility_canary_from_another_elf(self):
        foreign_canary = passing_production_canary(
            source_identity=self.source_identity_payload()
        )
        foreign_canary["context_elf_sha256"] = "9" * 64
        foreign_canary["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in foreign_canary.items()
                    if key != "artifact_sha256"
                }
            )
        )
        with self.assertRaises(ValueError):
            context._build_context_result(
                manifest=self.manifest,
                rows=context.synthetic_passing_rows(self.manifest),
                source_registry=self.registry,
                compatibility_canary=foreign_canary,
                source_identity=self.source_identity_payload(),
                adaptive_evidence=self.result_payload()["adaptive_evidence"],
            )

        source_identity = copy.deepcopy(self.result_payload()["source_identity"])
        source_identity["unexpected"] = "field"
        with self.assertRaisesRegex(ValueError, "field inventory"):
            context._build_context_result(
                manifest=self.manifest,
                rows=context.synthetic_passing_rows(self.manifest),
                source_registry=self.registry,
                compatibility_canary=passing_production_canary(),
                source_identity=source_identity,
                adaptive_evidence=self.result_payload()["adaptive_evidence"],
            )

        control_mismatch = passing_production_canary()
        control_mismatch["control_opcode_lab_elf_sha256"] = "9" * 64
        control_mismatch["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in control_mismatch.items()
                    if key != "artifact_sha256"
                }
            )
        )
        with self.assertRaisesRegex(ValueError, "canary.*identity"):
            context._build_context_result(
                manifest=self.manifest,
                rows=context.synthetic_passing_rows(self.manifest),
                source_registry=self.registry,
                compatibility_canary=control_mismatch,
                source_identity=self.result_payload()["source_identity"],
                adaptive_evidence=self.result_payload()["adaptive_evidence"],
            )

    def test_result_rejects_legacy_elf_forged_outside_calibration_identity(self):
        payload = self.result_payload()
        canary = copy.deepcopy(payload["compatibility_canary"])
        canary["legacy_revm_elf_sha256"] = "9" * 64
        canary["legacy_osaka_canary"]["provenance"][
            "guest_elf_sha256"
        ] = "9" * 64
        canary["legacy_osaka_canary"]["artifact_sha256"] = (
            context.sha256_bytes(
                context.canonical_json(
                    {
                        key: value
                        for key, value in canary["legacy_osaka_canary"].items()
                        if key != "artifact_sha256"
                    }
                )
            )
        )
        canary["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in canary.items()
                    if key != "artifact_sha256"
                }
            )
        )
        with self.assertRaisesRegex(ValueError, "legacy compatibility canary"):
            context.validate_context_compatibility_canary(canary)

    def test_result_rejects_legacy_canary_from_another_calibration(self):
        payload = self.result_payload()
        canary = copy.deepcopy(payload["compatibility_canary"])
        provenance = copy.deepcopy(
            canary["legacy_osaka_canary"]["provenance"]
        )
        provenance["calibration_identity_sha256"] = "8" * 64
        provenance["calibration_id"] = provenance[
            "calibration_identity_sha256"
        ][:24]
        replay = rebind_legacy_replay_to_current_identity(
            canary["legacy_osaka_replay"], provenance
        )
        historical, current = context._replay_legacy_osaka_evidence(
            replay, provenance
        )
        legacy = opcode_gas.build_osaka_compatibility_canary(
            historical,
            current,
            baseline_artifact_sha256=(
                context.HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256
            ),
            expected_baseline_artifact_sha256=(
                context.HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256
            ),
        )
        legacy["provenance"] = provenance
        legacy["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in legacy.items()
                    if key != "artifact_sha256"
                }
            )
        )
        canary["legacy_osaka_canary"] = legacy
        canary["legacy_osaka_replay"] = replay
        canary["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(
                {
                    key: value
                    for key, value in canary.items()
                    if key != "artifact_sha256"
                }
            )
        )
        context.validate_context_compatibility_canary(canary)
        with self.assertRaisesRegex(
            ValueError, "legacy compatibility canary calibration"
        ):
            context._build_context_result(
                manifest=self.manifest,
                rows=payload["rows"],
                source_registry=self.registry,
                compatibility_canary=canary,
                source_identity=payload["source_identity"],
                adaptive_evidence=payload["adaptive_evidence"],
            )

    def test_result_replay_rejects_extra_missing_and_symlink_inventory(self):
        payload = self.result_payload()
        for mutation in ("extra", "missing", "symlink"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                sealed = self.seal(payload, pathlib.Path(directory))
                if mutation == "extra":
                    (sealed / "extra").write_text("x")
                elif mutation == "missing":
                    (sealed / "source-identity.json").unlink()
                else:
                    target = pathlib.Path(directory) / "rows-copy"
                    target.write_bytes((sealed / "rows.jsonl").read_bytes())
                    (sealed / "rows.jsonl").unlink()
                    (sealed / "rows.jsonl").symlink_to(target)
                with self.assertRaises(ValueError):
                    context.verify_context_result(sealed)

    def test_same_id_idempotency_rejects_preexisting_child_symlink(self):
        payload = self.result_payload()
        with tempfile.TemporaryDirectory() as source_directory, tempfile.TemporaryDirectory() as output_directory:
            valid = self.seal(payload, pathlib.Path(source_directory))
            destination = pathlib.Path(output_directory) / payload["result_id"]
            destination.mkdir()
            for name in context.CONTEXT_RESULT_INVENTORY:
                (destination / name).symlink_to(valid / name)
            with self.assertRaisesRegex(ValueError, "inventory"):
                self.seal(payload, pathlib.Path(output_directory))

    def test_same_id_dangling_symlink_rejects_before_replay_or_temp_write(self):
        payload = self.result_payload()
        with tempfile.TemporaryDirectory() as output_directory:
            root = pathlib.Path(output_directory)
            destination = root / payload["result_id"]
            destination.symlink_to(root / "missing-target")
            self.identity_replay.reset_mock()
            with self.assertRaisesRegex(ValueError, "destination conflicts"):
                self.seal(payload, root)
            self.identity_replay.assert_not_called()
            self.assertTrue(destination.is_symlink())
            self.assertEqual(
                sorted(path.name for path in root.iterdir()), [payload["result_id"]]
            )

    def test_seal_rejects_output_inside_immutable_input_before_write(self):
        payload = self.result_payload()
        with tempfile.TemporaryDirectory() as directory:
            source = pathlib.Path(directory) / "run"
            source.mkdir()
            output = source / "results"
            with self.assertRaisesRegex(ValueError, "overlaps"):
                self.seal(
                    payload,
                    output,
                    protected_inputs={
                        **self.protected_inputs(),
                        "run": source,
                    },
                )
            self.assertFalse(output.exists())
            self.identity_replay.assert_not_called()

    def test_seal_rejects_output_inside_full_canary_source_before_write(self):
        payload = self.result_payload()
        with tempfile.TemporaryDirectory() as directory:
            calibration = pathlib.Path(directory) / "calibration"
            canary_source = (
                calibration / "context-compatibility/control-anchors/fixtures"
            )
            canary_source.mkdir(parents=True)
            marker = canary_source / "source.json"
            marker.write_bytes(b"immutable\n")
            before = context.sha256_bytes(marker.read_bytes())
            output = canary_source / "sealed"
            with self.assertRaisesRegex(ValueError, "overlaps"):
                self.seal(
                    payload,
                    output,
                    protected_inputs={
                        **self.protected_inputs(),
                        "calibration_run": calibration,
                    },
                )
            self.assertFalse(output.exists())
            self.assertEqual(context.sha256_bytes(marker.read_bytes()), before)
            self.identity_replay.assert_not_called()

    def test_seal_rejects_output_inside_frozen_legacy_package_before_write(self):
        payload = self.result_payload()
        with tempfile.TemporaryDirectory() as directory:
            package = pathlib.Path(directory) / "legacy-revm-v1"
            package.mkdir()
            marker = package / "source.bin"
            marker.write_bytes(b"immutable\n")
            before = context.sha256_bytes(marker.read_bytes())
            output = package / "sealed"
            with mock.patch.object(
                context, "FROZEN_LEGACY_REVM_PACKAGE_PATH", package
            ), self.assertRaisesRegex(ValueError, "overlaps"):
                self.seal(payload, output)
            self.assertFalse(output.exists())
            self.assertEqual(context.sha256_bytes(marker.read_bytes()), before)
            self.identity_replay.assert_not_called()

    def test_public_seal_rejects_frozen_legacy_package_overlap_before_preflight(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            calibration = root / "calibration"
            run = calibration / "context-campaign"
            run.mkdir(parents=True)
            package = root / "legacy-revm-v1"
            package.mkdir()
            marker = package / "source.bin"
            marker.write_bytes(b"immutable\n")
            before = context.sha256_bytes(marker.read_bytes())
            output = package / "sealed"
            with mock.patch.object(
                context, "FROZEN_LEGACY_REVM_PACKAGE_PATH", package
            ), mock.patch.object(
                opcode_gas, "validate_calibration_execution_identity"
            ) as calibration_validator, self.assertRaisesRegex(
                ValueError, "overlaps"
            ):
                context.seal_context_result(
                    manifest_path=MANIFEST_PATH,
                    calibration_run=calibration,
                    run=run,
                    compatibility_canary_path=(
                        calibration / "context-compatibility/compatibility-canary.json"
                    ),
                    corrected_core_path=CORRECTED_REGISTRY_PATH,
                    coverage_v5_path=(
                        ROOT / "experiments/opcode-gas/manifests/operation-coverage-v5.json"
                    ),
                    out_root=output,
                )
            calibration_validator.assert_not_called()
            self.assertFalse(output.exists())
            self.assertEqual(context.sha256_bytes(marker.read_bytes()), before)

    def test_public_seal_rejects_canary_source_overlap_before_preflight(self):
        with tempfile.TemporaryDirectory() as directory:
            calibration = pathlib.Path(directory) / "calibration"
            run = calibration / "context-campaign"
            canary_source = (
                calibration / "context-compatibility/control-anchors/fixtures"
            )
            run.mkdir(parents=True)
            canary_source.mkdir(parents=True)
            marker = canary_source / "source.json"
            marker.write_bytes(b"immutable\n")
            before = context.sha256_bytes(marker.read_bytes())
            output = canary_source / "sealed"
            with mock.patch.object(
                opcode_gas, "validate_calibration_execution_identity"
            ) as calibration_validator, self.assertRaisesRegex(
                ValueError, "overlaps"
            ):
                context.seal_context_result(
                    manifest_path=MANIFEST_PATH,
                    calibration_run=calibration,
                    run=run,
                    compatibility_canary_path=(
                        calibration
                        / "context-compatibility/compatibility-canary.json"
                    ),
                    corrected_core_path=CORRECTED_REGISTRY_PATH,
                    coverage_v5_path=(
                        ROOT
                        / "experiments/opcode-gas/manifests/operation-coverage-v5.json"
                    ),
                    out_root=output,
                )
            calibration_validator.assert_not_called()
            self.assertFalse(output.exists())
            self.assertEqual(context.sha256_bytes(marker.read_bytes()), before)

    def test_non_candidate_context_result_cannot_promote_or_overlay(self):
        payload = self.result_payload()
        v4 = json.loads(COVERAGE_V4_PATH.read_text())
        v5 = copy.deepcopy(self.v5)
        with self.assertRaisesRegex(ValueError, "transport"):
            context.promote_operation_coverage_v6(v5, payload)
        self.assertEqual(json.loads(COVERAGE_V4_PATH.read_text()), v4)
        corrected_registry = copy.deepcopy(self.registry["registry"])
        corrected_registry["models"]["opcode:0x15"] = {"kind": "static_raw_gas", "parameters": {"body_per_raw_gas": "123"}}
        with self.assertRaisesRegex(ValueError, "transport"):
            context.overlay_context_models(corrected_registry, payload)
        model = payload["models"]["opcode:0x30"]
        self.assertEqual(
            model["parameter_basis"],
            "provisional_legacy_projection_unvalidated_cross_elf_transport",
        )
        with self.assertRaisesRegex(ValueError, "basis"):
            context.predict_context_overlay_event(
                common_dispatch=Fraction(13), raw_gas=2, model=model
            )

        forged = copy.deepcopy(payload)
        forged.pop("artifact_sha256")
        forged.pop("result_id")
        forged.pop("result_identity_sha256")
        forged["candidate_eligible"] = True
        forged["context_transport"] = {
            "status": "supported",
            "scope": "self_declared_forgery",
            "required_before_candidate_promotion": False,
        }
        for forged_model in forged["models"].values():
            forged_model["parameter_basis"] = (
                "production_scaled_body_excluding_common_dispatch"
            )
        identity = context.sha256_bytes(context.canonical_json(forged))
        forged["result_identity_sha256"] = identity
        forged["result_id"] = identity[:24]
        forged["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(forged)
        )
        with self.assertRaisesRegex(ValueError, "transport contract"):
            context.promote_operation_coverage_v6(v5, forged)
        with self.assertRaisesRegex(ValueError, "transport contract"):
            context.overlay_context_models(corrected_registry, forged)

    def test_partial_non_candidate_result_still_cannot_promote(self):
        payload = copy.deepcopy(self.result_payload())
        payload.pop("artifact_sha256")
        payload.pop("result_id")
        payload.pop("result_identity_sha256")
        payload["models"].pop("opcode:0x34")
        payload["measured_model_keys"] = sorted(payload["models"])
        identity = context.sha256_bytes(context.canonical_json(payload))
        payload["result_identity_sha256"] = identity
        payload["result_id"] = identity[:24]
        payload["artifact_sha256"] = context.sha256_bytes(
            context.canonical_json(payload)
        )
        with self.assertRaisesRegex(ValueError, "transport"):
            context.promote_operation_coverage_v6(self.v5, payload)
        with self.assertRaisesRegex(ValueError, "transport"):
            context.overlay_context_models(self.registry["registry"], payload)

    def test_v5_rebinds_corrected_core_provenance_then_v6_changes_only_six_rows(self):
        v4 = json.loads(COVERAGE_V4_PATH.read_text())
        corrected = json.loads(CORRECTED_REGISTRY_PATH.read_text())
        package = json.loads(CORRECTED_PACKAGE_PATH.read_text())
        v5 = context.build_operation_coverage_v5(v4, corrected, package)
        self.assertEqual(v5["predecessor_v4_artifact_sha256"], v4["artifact_sha256"])
        for row in v5["execution_coverage"]:
            artifact_ref = row.get("artifact_ref")
            if artifact_ref is not None:
                self.assertEqual(artifact_ref["artifact_sha256"], corrected["artifact_sha256"])
            for evidence in row.get("source_evidence", []):
                if evidence.get("kind") in {"sealed_registry_model", "sealed_registry_unsupported"}:
                    self.assertEqual(evidence["artifact_sha256"], corrected["artifact_sha256"])
        with self.assertRaisesRegex(ValueError, "transport"):
            context.promote_operation_coverage_v6(v5, self.result_payload())


if __name__ == "__main__":
    unittest.main()
