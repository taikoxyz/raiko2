#!/usr/bin/env python3
"""Scan repository files and commits for committed EVM private key material."""

from __future__ import annotations

import argparse
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    from cryptography.hazmat.primitives.asymmetric import ec
    from eth_utils import keccak
except ImportError as exc:
    raise SystemExit(
        "Missing dependency. Run with "
        "`uv run --with cryptography --with eth-utils "
        "python scripts/security/check_evm_private_keys.py`."
    ) from exc


DEFAULT_DENY_ADDRESSES = ("0x382bba7d7bc9ae86c5de3e16c4ca96bcc0a3478e",)
SECP256K1_ORDER = int(
    "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
    16,
)
HEX_PRIVATE_KEY_RE = re.compile(
    r"(?<![0-9a-fA-F])(?:0x)?([0-9a-fA-F]{64})(?![0-9a-fA-F])"
)
SENSITIVE_CONTEXT_RE = re.compile(
    r"networkPrivateKey|NETWORK_PRIVATE_KEY|PRIVATE_KEY|SIGNER_KEY|"
    r"(?:network[-_]?private|private|signer|secret)[-_]?key",
    re.IGNORECASE,
)
SKIP_DIR_NAMES = {
    ".cache",
    ".direnv",
    ".git",
    ".hg",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".venv",
    ".worktrees",
    "node_modules",
    "target",
    "venv",
    "__pycache__",
}


@dataclass(frozen=True)
class Finding:
    location: str
    line_number: int
    reason: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Fail if repository files contain EVM private keys."
    )
    parser.add_argument(
        "root",
        nargs="?",
        type=Path,
        default=Path("."),
        help="Root directory to scan. Defaults to the current directory.",
    )
    parser.add_argument(
        "--deny-address",
        action="append",
        default=list(DEFAULT_DENY_ADDRESSES),
        help="Address that must not be derivable from any committed private key.",
    )
    parser.add_argument(
        "--scan-filesystem",
        action="store_true",
        help="Scan the filesystem instead of git-tracked and unignored files.",
    )
    parser.add_argument(
        "--include-worktrees",
        action="store_true",
        help="Include .worktrees when --scan-filesystem is used.",
    )
    parser.add_argument(
        "--full-derive",
        action="store_true",
        help="Derive every 32-byte hex candidate. Slower on chain fixture-heavy trees.",
    )
    parser.add_argument(
        "--git-commit",
        action="append",
        default=[],
        help="Scan files changed by this commit object instead of the worktree.",
    )
    parser.add_argument(
        "--git-rev-range",
        action="append",
        default=[],
        help="Scan files changed by each commit in this git rev-list range.",
    )
    return parser.parse_args()


def git(
    root: Path, args: list[str], *, check: bool = True
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )


def git_candidate_files(root: Path) -> list[Path] | None:
    result = git(
        root,
        ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        check=False,
    )
    if result.returncode != 0:
        return None

    return [root / Path(item.decode()) for item in result.stdout.split(b"\0") if item]


def should_skip_path(parts: tuple[str, ...], *, include_worktrees: bool) -> bool:
    skip_dirs = (
        SKIP_DIR_NAMES if not include_worktrees else SKIP_DIR_NAMES - {".worktrees"}
    )
    return any(part in skip_dirs for part in parts[:-1])


def filesystem_candidate_files(root: Path, *, include_worktrees: bool) -> list[Path]:
    files: list[Path] = []
    for path in root.rglob("*"):
        if path.is_symlink() or not path.is_file():
            continue

        relative_parts = path.relative_to(root).parts
        if should_skip_path(relative_parts, include_worktrees=include_worktrees):
            continue

        files.append(path)

    return files


def is_text_bytes(data: bytes) -> bool:
    return b"\0" not in data[:8192]


def is_text_file(path: Path) -> bool:
    try:
        chunk = path.read_bytes()[:8192]
    except OSError:
        return False
    return is_text_bytes(chunk)


def derive_address(candidate_hex: str) -> str | None:
    try:
        value = int(candidate_hex, 16)
        if value <= 0 or value >= SECP256K1_ORDER:
            return None

        private_key = ec.derive_private_key(value, ec.SECP256K1())
        public_numbers = private_key.public_key().public_numbers()
        public_key = public_numbers.x.to_bytes(32, "big") + public_numbers.y.to_bytes(
            32, "big"
        )
        return f"0x{keccak(public_key)[-20:].hex()}"
    except ValueError:
        return None


def has_sensitive_context(lines: list[str], line_index: int) -> bool:
    context_start = max(0, line_index - 2)
    context_end = min(len(lines), line_index + 2)
    return any(
        SENSITIVE_CONTEXT_RE.search(line) is not None
        for line in lines[context_start:context_end]
    )


def scan_lines(
    *,
    location: str,
    lines: list[str],
    deny_addresses: set[str],
    full_derive: bool,
) -> list[Finding]:
    findings: list[Finding] = []
    for line_index, line in enumerate(lines):
        line_number = line_index + 1
        if not HEX_PRIVATE_KEY_RE.search(line):
            continue

        is_sensitive = has_sensitive_context(lines, line_index)
        if not is_sensitive and not full_derive:
            continue

        for match in HEX_PRIVATE_KEY_RE.finditer(line):
            address = derive_address(match.group(1))
            if address in deny_addresses:
                findings.append(
                    Finding(
                        location=location,
                        line_number=line_number,
                        reason=f"private key derives to denied address {address}",
                    )
                )
            elif is_sensitive and address is not None:
                findings.append(
                    Finding(
                        location=location,
                        line_number=line_number,
                        reason="valid EVM private key candidate in a sensitive key field",
                    )
                )

    return findings


def scan_file(
    path: Path,
    *,
    root: Path,
    deny_addresses: set[str],
    full_derive: bool,
) -> list[Finding]:
    if not is_text_file(path):
        return []

    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return []

    return scan_lines(
        location=str(path.relative_to(root)),
        lines=lines,
        deny_addresses=deny_addresses,
        full_derive=full_derive,
    )


def scan_worktree_files(
    *,
    root: Path,
    files: list[Path],
    deny_addresses: set[str],
    full_derive: bool,
) -> tuple[list[Finding], int]:
    findings: list[Finding] = []
    scanned_entries = 0
    for path in sorted(set(files)):
        if not path.exists() or not path.is_file():
            continue
        scanned_entries += 1
        findings.extend(
            scan_file(
                path,
                root=root,
                deny_addresses=deny_addresses,
                full_derive=full_derive,
            )
        )
    return findings, scanned_entries


def commit_paths(root: Path, commit: str) -> list[str]:
    result = git(
        root,
        ["diff-tree", "--root", "--no-commit-id", "--name-only", "-r", "-z", commit],
        check=False,
    )
    if result.returncode != 0:
        return []

    return [item.decode() for item in result.stdout.split(b"\0") if item]


def commit_file_bytes(root: Path, commit: str, path: str) -> bytes | None:
    result = git(root, ["show", f"{commit}:{path}"], check=False)
    if result.returncode != 0:
        return None
    return result.stdout


def commits_from_rev_ranges(root: Path, rev_ranges: list[str]) -> list[str]:
    commits: list[str] = []
    for rev_range in rev_ranges:
        result = git(
            root, ["rev-list", "--reverse", *shlex.split(rev_range)], check=False
        )
        if result.returncode == 0:
            commits.extend(line.decode() for line in result.stdout.splitlines() if line)
    return commits


def scan_git_commits(
    *,
    root: Path,
    commits: list[str],
    deny_addresses: set[str],
    full_derive: bool,
) -> tuple[list[Finding], int]:
    findings: list[Finding] = []
    scanned_entries = 0
    for commit in dict.fromkeys(commits):
        short_commit = commit[:12]
        for path in commit_paths(root, commit):
            relative_parts = Path(path).parts
            if should_skip_path(relative_parts, include_worktrees=False):
                continue

            data = commit_file_bytes(root, commit, path)
            if data is None or not is_text_bytes(data):
                continue

            scanned_entries += 1
            findings.extend(
                scan_lines(
                    location=f"{short_commit}:{path}",
                    lines=data.decode("utf-8", errors="replace").splitlines(),
                    deny_addresses=deny_addresses,
                    full_derive=full_derive,
                )
            )

    return findings, scanned_entries


def main() -> int:
    args = parse_args()
    root = args.root.resolve()
    deny_addresses = {address.lower() for address in args.deny_address}
    git_commits = [*args.git_commit, *commits_from_rev_ranges(root, args.git_rev_range)]

    if git_commits:
        findings, scanned_entries = scan_git_commits(
            root=root,
            commits=git_commits,
            deny_addresses=deny_addresses,
            full_derive=args.full_derive,
        )
    else:
        if args.scan_filesystem:
            files = filesystem_candidate_files(
                root, include_worktrees=args.include_worktrees
            )
        else:
            files = git_candidate_files(root)
            if files is None:
                files = filesystem_candidate_files(
                    root, include_worktrees=args.include_worktrees
                )
        findings, scanned_entries = scan_worktree_files(
            root=root,
            files=files,
            deny_addresses=deny_addresses,
            full_derive=args.full_derive,
        )

    if findings:
        print(
            "EVM private key scan failed. No key material is printed.", file=sys.stderr
        )
        for finding in findings:
            print(
                f"- {finding.location}:{finding.line_number}: {finding.reason}",
                file=sys.stderr,
            )
        return 1

    print(f"Scanned {scanned_entries} file entries; no EVM private key leaks detected.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
