"""Render SGX Compose configurations without starting containers or loading local env files.

Run with an existing Python environment and Docker Compose installed:
    python -m unittest discover -s docker/tests -p 'test_*.py'
"""

import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
MPTCP_SYSCTL = "net.mptcp.enabled"
COMPOSE_CASES = (
    (
        "docker-compose.sgx.yml",
        "docker-compose.sgx.no-mptcp.yml",
        ("raiko2-sgx-init", "raiko2-sgx"),
    ),
    (
        "docker-compose.sgx.regression.yml",
        "docker-compose.sgx.regression.no-mptcp.yml",
        ("raiko2-sgx-init", "raiko2-sgx", "gaiko2-sgxgeth-init", "gaiko2-sgxgeth"),
    ),
)


class SgxComposeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if shutil.which("docker") is None:
            raise unittest.SkipTest("Docker Compose is required for rendering tests")
        subprocess.run(
            ["docker", "compose", "version"], check=True, capture_output=True, timeout=10
        )

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.env_file = self.directory / "empty.env"
        self.env_file.write_text("")
        key_file = self.directory / "enclave-key.pem"
        key_file.write_text("render-only placeholder\n")
        self.environment = {
            "PATH": os.environ.get("PATH", os.defpath),
            "RAIKO2_SGX_ENCLAVE_KEY_HOST": str(key_file),
            "RAIKO2_L1_RPC": "http://127.0.0.1:18545",
            "RAIKO2_L2_RPC": "http://127.0.0.1:28545",
        }

    def render(self, *files):
        command = [
            "docker", "compose", "--env-file", str(self.env_file),
            "--project-directory", str(REPOSITORY_ROOT / "docker"),
        ]
        for file in files:
            self.assertTrue(file.is_file(), f"Missing Compose configuration: {file.name}")
            command.extend(("-f", str(file)))
        command.extend(("--profile", "*", "config", "--format", "json"))
        result = subprocess.run(
            command, check=True, capture_output=True, text=True,
            env=self.environment, timeout=20,
        )
        return json.loads(result.stdout)

    def test_defaults_disable_mptcp_for_every_provider(self):
        for base, _, providers in COMPOSE_CASES:
            with self.subTest(base=base):
                configuration = self.render(REPOSITORY_ROOT / "docker" / base)
                for name in providers:
                    service = configuration["services"][name]
                    self.assertEqual(service["sysctls"][MPTCP_SYSCTL], "0")
                    if name.startswith("gaiko2-"):
                        self.assertEqual(service["environment"]["GODEBUG"], "multipathtcp=0")
                    else:
                        self.assertNotIn("GODEBUG", service["environment"])

    def test_no_mptcp_overrides_change_only_the_missing_sysctl(self):
        for base, override, providers in COMPOSE_CASES:
            with self.subTest(base=base):
                base_path = REPOSITORY_ROOT / "docker" / base
                expected = self.render(base_path)
                for name in providers:
                    sysctls = expected["services"][name]["sysctls"]
                    sysctls.pop(MPTCP_SYSCTL)
                    if not sysctls:
                        expected["services"][name].pop("sysctls")
                actual = self.render(base_path, REPOSITORY_ROOT / "docker" / override)
                self.assertEqual(actual, expected)

    def test_no_mptcp_overrides_preserve_other_sysctls(self):
        for base, override, providers in COMPOSE_CASES:
            with self.subTest(base=base):
                original = self.render(REPOSITORY_ROOT / "docker" / base)
                for name in providers:
                    original["services"][name]["sysctls"]["net.ipv4.ip_forward"] = "0"
                base_fixture = self.directory / "base-with-other-sysctls.json"
                base_fixture.write_text(json.dumps(original))
                expected = copy.deepcopy(original)
                for name in providers:
                    expected["services"][name]["sysctls"].pop(MPTCP_SYSCTL)
                actual = self.render(base_fixture, REPOSITORY_ROOT / "docker" / override)
                self.assertEqual(actual, expected)

    def test_additional_overlays_follow_the_no_mptcp_override(self):
        for base, override, providers in COMPOSE_CASES:
            with self.subTest(base=base):
                additional = self.directory / "additional-sysctl.yml"
                additional.write_text(
                    "services:\n" + "".join(
                        f"  {name}:\n    sysctls:\n      net.ipv4.ip_forward: '0'\n"
                        for name in providers
                    )
                )
                base_path = REPOSITORY_ROOT / "docker" / base
                original = self.render(base_path, additional)
                expected = copy.deepcopy(original)
                for name in providers:
                    expected["services"][name]["sysctls"].pop(MPTCP_SYSCTL)
                actual = self.render(base_path, REPOSITORY_ROOT / "docker" / override, additional)
                self.assertEqual(actual, expected)


if __name__ == "__main__":
    unittest.main()
