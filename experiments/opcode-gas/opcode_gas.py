#!/usr/bin/env python3
"""SP1 opcode prover-gas experiment scaffold."""

from __future__ import annotations

import argparse
import json
import pathlib
import statistics
import subprocess
import tomllib
from dataclasses import asdict, dataclass
from typing import Any, Iterable


@dataclass(frozen=True)
class CaseSpec:
    name: str
    scenario: str
    template: str
    target_raw_gas: int
    kind: str = "opcode"
    opcode: int | None = None
    address: int | None = None
    input_size: int | None = None


@dataclass(frozen=True)
class Manifest:
    name: str
    backend: str
    variants: list[int]
    cases: list[CaseSpec]


@dataclass(frozen=True)
class GeneratedBytecode:
    bytes_hex: str
    opcode_counts: dict[int, int]


@dataclass(frozen=True)
class FitResult:
    case: str
    sample_count: int
    slope_per_operation: float
    slope_per_raw_gas: float
    intercept: float
    r2: float


def load_manifest(path: pathlib.Path) -> Manifest:
    data = tomllib.loads(path.read_text())
    cases = [
        CaseSpec(
            name=item["name"],
            scenario=item["scenario"],
            template=item["template"],
            target_raw_gas=int(item["target_raw_gas"]),
            kind=item.get("kind", "opcode"),
            opcode=parse_opcode(item["opcode"]) if "opcode" in item else None,
            address=parse_opcode(item["address"]) if "address" in item else None,
            input_size=int(item["input_size"]) if "input_size" in item else None,
        )
        for item in data.get("cases", [])
    ]
    return Manifest(
        name=data["name"],
        backend=data.get("backend", "sp1"),
        variants=[int(value) for value in data.get("variants", [])],
        cases=cases,
    )


def parse_opcode(value: int | str) -> int:
    if isinstance(value, int):
        return value
    return int(value, 16 if value.lower().startswith("0x") else 10)


def build_bytecode(case: CaseSpec, target_count: int) -> GeneratedBytecode:
    if target_count < 0:
        raise ValueError("target_count must be non-negative")
    if case.opcode is None:
        raise ValueError(f"opcode case {case.name} is missing opcode")
    if case.template == "stack_binary":
        bytecode = build_stack_binary_bytecode(case.opcode, target_count)
    elif case.template == "keccak_32":
        bytecode = build_keccak_32_bytecode(target_count)
    else:
        raise ValueError(f"unknown template: {case.template}")
    return GeneratedBytecode(bytes_hex=bytecode.hex(), opcode_counts=count_opcodes(bytecode))


def build_stack_binary_bytecode(opcode: int, target_count: int) -> bytes:
    out = bytearray()
    if target_count == 0:
        out.extend([0x60, 0x01, 0x50, 0x00])  # PUSH1 1; POP; STOP
        return bytes(out)

    out.extend([0x60, 0x01, 0x60, 0x02, opcode])
    for _ in range(target_count - 1):
        out.extend([0x60, 0x01, opcode])
    out.extend([0x50, 0x00])  # POP; STOP
    return bytes(out)


def build_keccak_32_bytecode(target_count: int) -> bytes:
    out = bytearray([0x60, 0x00, 0x60, 0x00, 0x52])  # zero one memory word
    for _ in range(target_count):
        out.extend([0x60, 0x20, 0x60, 0x00, 0x20, 0x50])
    out.append(0x00)
    return bytes(out)


def count_opcodes(bytecode: bytes) -> dict[int, int]:
    counts: dict[int, int] = {}
    i = 0
    while i < len(bytecode):
        opcode = bytecode[i]
        counts[opcode] = counts.get(opcode, 0) + 1
        i += 1
        if 0x60 <= opcode <= 0x7F:
            i += opcode - 0x5F
    return counts


def build_precompile_input(case: CaseSpec) -> str:
    if case.input_size is None:
        raise ValueError(f"precompile case {case.name} is missing input_size")
    payload = bytes((index % 251 for index in range(case.input_size)))
    return "0x" + payload.hex()


def generate_cases(manifest: Manifest, out_dir: pathlib.Path) -> list[pathlib.Path]:
    written = []
    for case in manifest.cases:
        for variant in manifest.variants:
            case_dir = out_dir / manifest.name / case.name / f"count-{variant}"
            case_dir.mkdir(parents=True, exist_ok=True)
            if case.kind == "opcode":
                if case.opcode is None:
                    raise ValueError(f"opcode case {case.name} is missing opcode")
                generated = build_bytecode(case, variant)
                payload = {
                    "suite": manifest.name,
                    "backend": manifest.backend,
                    "kind": case.kind,
                    "case": case.name,
                    "opcode": f"0x{case.opcode:02x}",
                    "scenario": case.scenario,
                    "template": case.template,
                    "target_count": variant,
                    "target_raw_gas": case.target_raw_gas,
                    "target_feature": variant * case.target_raw_gas,
                    "bytecode": "0x" + generated.bytes_hex,
                    "opcode_counts": {
                        f"0x{k:02x}": v for k, v in sorted(generated.opcode_counts.items())
                    },
                    "guest_input_status": "opcode_lab_guest_input",
                }
                guest_input = {
                    "case": case.name,
                    "scenario": case.scenario,
                    "opcode": case.opcode,
                    "target_count": variant,
                    "target_raw_gas": case.target_raw_gas,
                    "bytecode": "0x" + generated.bytes_hex,
                }
            elif case.kind == "precompile":
                if case.address is None:
                    raise ValueError(f"precompile case {case.name} is missing address")
                precompile_input = build_precompile_input(case)
                payload = {
                    "suite": manifest.name,
                    "backend": manifest.backend,
                    "kind": case.kind,
                    "case": case.name,
                    "address": f"0x{case.address:02x}",
                    "scenario": case.scenario,
                    "template": case.template,
                    "target_count": variant,
                    "input_size": case.input_size,
                    "target_raw_gas": case.target_raw_gas,
                    "target_feature": variant * case.target_raw_gas,
                    "input": precompile_input,
                    "guest_input_status": "precompile_lab_guest_input",
                }
                guest_input = {
                    "case": case.name,
                    "scenario": case.scenario,
                    "address": case.address,
                    "target_count": variant,
                    "input_size": case.input_size,
                    "target_raw_gas": case.target_raw_gas,
                    "input": precompile_input,
                }
            else:
                raise ValueError(f"unknown case kind: {case.kind}")
            path = case_dir / "case.json"
            path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            (case_dir / "guest-input.json").write_text(
                json.dumps(guest_input, indent=2, sort_keys=True) + "\n"
            )
            written.append(path)
    return written


def run_guest_input(
    guest_launcher: pathlib.Path,
    elf_path: pathlib.Path,
    input_path: pathlib.Path,
    json_out: pathlib.Path,
) -> None:
    cmd = [
        str(guest_launcher),
        "--stage",
        "opcode-lab",
        "--proof-type",
        "sp1",
        "--mode",
        "execute",
        "--sp1-prover",
        "local",
        "--elf",
        str(elf_path),
        "--input",
        str(input_path),
        "--json-out",
        str(json_out),
    ]
    subprocess.run(cmd, check=True)


def run_guest_inputs(
    guest_launcher: pathlib.Path,
    elf_path: pathlib.Path,
    input_paths: list[pathlib.Path],
    reports_jsonl: pathlib.Path,
    stage: str = "opcode-lab",
) -> pathlib.Path:
    reports_jsonl.parent.mkdir(parents=True, exist_ok=True)
    input_list_path = reports_jsonl.with_name("opcode-lab-inputs.json")
    input_list_path.write_text(
        json.dumps([str(path) for path in input_paths], indent=2, sort_keys=True) + "\n"
    )
    cmd = [
        str(guest_launcher),
        "--stage",
        stage,
        "--proof-type",
        "sp1",
        "--mode",
        "execute",
        "--sp1-prover",
        "local",
        "--elf",
        str(elf_path),
        "--input-list",
        str(input_list_path),
        "--jsonl-out",
        str(reports_jsonl),
    ]
    subprocess.run(cmd, check=True)
    return input_list_path


def raw_run_from_report(case: dict[str, Any], report: dict[str, Any]) -> dict[str, Any]:
    raw_run = {**case, **report}
    if "prover_gas" not in raw_run and "gas" in raw_run:
        raw_run["prover_gas"] = raw_run["gas"]
    return raw_run


def iter_jsonl(path: pathlib.Path) -> Iterable[dict[str, Any]]:
    with path.open() as fh:
        for line in fh:
            line = line.strip()
            if line:
                yield json.loads(line)


def fit_case(runs: list[dict[str, Any]]) -> FitResult:
    if len(runs) < 2:
        raise ValueError("at least two runs are required")
    case_name = str(runs[0]["case"])
    raw_gas = float(runs[0]["target_raw_gas"])
    xs = [float(run["target_count"]) for run in runs]
    ys = [float(run["prover_gas"]) for run in runs]
    mean_x = statistics.fmean(xs)
    mean_y = statistics.fmean(ys)
    denom = sum((x - mean_x) ** 2 for x in xs)
    if denom == 0:
        raise ValueError("target_count must vary")
    slope = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys)) / denom
    intercept = mean_y - slope * mean_x
    predicted = [intercept + slope * x for x in xs]
    ss_res = sum((y - y_hat) ** 2 for y, y_hat in zip(ys, predicted))
    ss_tot = sum((y - mean_y) ** 2 for y in ys)
    r2 = 1.0 if ss_tot == 0 else 1.0 - (ss_res / ss_tot)
    return FitResult(
        case=case_name,
        sample_count=len(runs),
        slope_per_operation=slope,
        slope_per_raw_gas=slope / raw_gas,
        intercept=intercept,
        r2=r2,
    )


def fit_report(runs_path: pathlib.Path, out_dir: pathlib.Path) -> list[FitResult]:
    grouped: dict[str, list[dict[str, Any]]] = {}
    for run in iter_jsonl(runs_path):
        grouped.setdefault(str(run["case"]), []).append(run)
    results = [fit_case(sorted(items, key=lambda item: item["target_count"])) for items in grouped.values()]
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "fit.json").write_text(
        json.dumps([asdict(result) for result in results], indent=2, sort_keys=True) + "\n"
    )
    (out_dir / "coefficients.json").write_text(
        json.dumps(
            {result.case: result.slope_per_raw_gas for result in results},
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )
    write_markdown_report(out_dir / "uzen-vs-fit.md", results)
    return results


def write_markdown_report(path: pathlib.Path, results: list[FitResult]) -> None:
    lines = [
        "# Uzen Vs Fitted SP1 Prover Gas",
        "",
        "| Case | Samples | Slope/op | Slope/raw-gas | R2 |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for result in sorted(results, key=lambda item: item.case):
        lines.append(
            f"| {result.case} | {result.sample_count} | "
            f"{result.slope_per_operation:.6g} | {result.slope_per_raw_gas:.6g} | {result.r2:.6g} |"
        )
    path.write_text("\n".join(lines) + "\n")


def cmd_generate(args: argparse.Namespace) -> None:
    manifest = load_manifest(args.manifest)
    written = generate_cases(manifest, args.out)
    print(f"wrote {len(written)} case metadata files")


def cmd_fit(args: argparse.Namespace) -> None:
    results = fit_report(args.runs, args.out)
    print(f"fit {len(results)} case(s)")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    generate = subcommands.add_parser("generate", help="generate opcode case metadata")
    generate.add_argument("--manifest", type=pathlib.Path, required=True)
    generate.add_argument("--out", type=pathlib.Path, required=True)
    generate.set_defaults(func=cmd_generate)

    run = subcommands.add_parser("run", help="run generated guest-input cases")
    run.add_argument("--fixtures", type=pathlib.Path, required=True)
    run.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    run.add_argument(
        "--elf",
        type=pathlib.Path,
        default=pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"),
        help="SP1 opcode-lab guest ELF",
    )
    run.add_argument(
        "--precompile-elf",
        type=pathlib.Path,
        default=pathlib.Path("crates/guests/elf/sp1_precompile_lab.elf"),
        help="SP1 precompile-lab guest ELF",
    )
    run.add_argument("--out", type=pathlib.Path, required=True)
    run.set_defaults(func=cmd_run)

    fit = subcommands.add_parser("fit", help="fit marginal prover-gas coefficients")
    fit.add_argument("--runs", type=pathlib.Path, required=True)
    fit.add_argument("--out", type=pathlib.Path, required=True)
    fit.set_defaults(func=cmd_fit)
    return parser


def cmd_run(args: argparse.Namespace) -> None:
    args.out.parent.mkdir(parents=True, exist_ok=True)
    cases_by_kind: dict[str, list[tuple[dict[str, Any], pathlib.Path]]] = {}
    for case_path in sorted(args.fixtures.glob("**/case.json")):
        case = json.loads(case_path.read_text())
        input_path = case_path.with_name("guest-input.json")
        if input_path.exists():
            cases_by_kind.setdefault(case.get("kind", "opcode"), []).append((case, input_path))
    report_paths = []
    for kind, cases in sorted(cases_by_kind.items()):
        stage = "opcode-lab" if kind == "opcode" else "precompile-lab"
        elf_path = args.elf if kind == "opcode" else args.precompile_elf
        report_path = args.out.with_name(f"{args.out.stem}.{stage}.jsonl")
        run_guest_inputs(
            guest_launcher=args.guest_launcher,
            elf_path=elf_path,
            input_paths=[input_path for _, input_path in cases],
            reports_jsonl=report_path,
            stage=stage,
        )
        report_paths.append(report_path)
    case_by_input = {
        str(input_path): case for cases in cases_by_kind.values() for case, input_path in cases
    }
    with args.out.open("w") as out:
        ran = 0
        for report_path in report_paths:
            for report in iter_jsonl(report_path):
                case = case_by_input[report["input"]]
                out.write(json.dumps(raw_run_from_report(case, report), sort_keys=True) + "\n")
                ran += 1
    print(f"ran {ran} executable case(s)")


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
