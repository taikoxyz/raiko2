#!/usr/bin/env python3
"""Check the Gitleaks exception against real scans of temporary Git repositories."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


CONFIG = Path(__file__).resolve().parents[2] / ".gitleaks.toml"
GITLEAKS = os.environ.get("GITLEAKS_BIN", "gitleaks")
ROWS = "experiments/opcode-gas/derivations/64065fa462311bdc1848e9d0/rows.jsonl"
DIGEST = hashlib.sha256(b"[]").hexdigest()


class GitleaksConfigTests(unittest.TestCase):
    def scan(self, path, value):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / path
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(json.dumps(value) + "\n", encoding="utf-8")
            for args in (
                ["git", "init", "--quiet"],
                ["git", "add", "."],
                ["git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture"],
            ):
                subprocess.run(args, cwd=root, check=True, capture_output=True)
            report = root / "report.json"
            command = [
                GITLEAKS, "detect", "--source", ".", "--redact", "--no-banner",
                "--report-format", "json", "--report-path", str(report),
            ]
            if CONFIG.exists():
                command.extend(["--config", str(CONFIG)])
            result = subprocess.run(command, cwd=root, capture_output=True, text=True)
            self.assertIn(result.returncode, (0, 1), result.stderr)
            self.assertTrue(report.exists(), result.stderr)
            findings = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(result.returncode, int(bool(findings)), result.stderr)
            return findings

    def test_generated_access_list_hash_is_ignored(self):
        self.assertEqual(self.scan(ROWS, {"access_list_sha256": DIGEST}), [])

    def test_another_secret_on_the_same_row_is_detected(self):
        findings = self.scan(ROWS, {"access_list_sha256": DIGEST, "api_token": DIGEST})
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["RuleID"], "generic-api-key")
        self.assertIn("api_token", findings[0]["Match"])

    def test_same_hash_outside_generated_rows_is_detected(self):
        findings = self.scan("config.json", {"access_list_sha256": DIGEST})
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["RuleID"], "generic-api-key")

    def test_other_file_in_derivation_is_still_scanned(self):
        findings = self.scan(ROWS.replace("rows.jsonl", "config.json"),
                             {"access_list_sha256": DIGEST})
        self.assertEqual(len(findings), 1)

    def test_non_sha256_value_is_detected(self):
        findings = self.scan(ROWS, {"access_list_sha256": DIGEST[:-1]})
        self.assertEqual(len(findings), 1)

    def test_other_default_rules_remain_enabled(self):
        findings = self.scan(ROWS, {"value": "ghp_" + DIGEST[:36]})
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["RuleID"], "github-pat")


if __name__ == "__main__":
    unittest.main()
