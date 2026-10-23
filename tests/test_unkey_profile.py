"""Frontline profile structure/preflight and optional real-image missing-key check.

The live-image check uses fixture configuration without sending a Bearer key.
It does not prove authentication against a real Unkey workspace.
"""
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request

from test_foundation import ROOT, environment, free_port


def fixture_environment():
    env = environment()
    env.update(UNKEY_MANAGEMENT_ROOT_KEY="unkey_fixture_management_not_a_real_root_key",
               UNKEY_VERIFY_ROOT_KEY="unkey_fixture_verify_only_not_a_real_root_key",
               UNKEY_API_ID="api_fixture", UNKEY_KEYSPACE_ID="ks_fixture")
    return env


def profile_config(env):
    result = subprocess.run(
        ["docker", "compose", "--env-file", ".env.example", "--profile", "unkey", "config", "--format", "json"],
        cwd=ROOT, env=env, text=True, capture_output=True, check=True,
    )
    return json.loads(result.stdout)


class ProfileTests(unittest.TestCase):
    def test_pinned_gateway_has_private_upstream_and_a_separate_verify_credential(self):
        services = profile_config(fixture_environment())["services"]
        gateway = services["frontline"]
        self.assertIn("v1.0.26-rc.1@sha256:", gateway["image"])
        self.assertEqual(set(gateway["networks"]), {"gateway"})
        self.assertEqual(gateway["environment"]["UNKEY_VERIFY_ROOT_KEY"],
                         "unkey_fixture_verify_only_not_a_real_root_key")
        self.assertNotIn("UNKEY_MANAGEMENT_ROOT_KEY", gateway["environment"])
        self.assertNotIn("UNKEY_VERIFY_ROOT_KEY", services["api"]["environment"])
        self.assertEqual(gateway["depends_on"]["unkey-preflight"]["condition"], "service_completed_successfully")
        for port in gateway["ports"]:
            self.assertEqual(port["host_ip"], "127.0.0.1")
        for service in ("api", "db", "migrate"):
            self.assertFalse(services[service].get("ports"))
        config = (ROOT / "infra/unkey/gateway.toml").read_text()
        self.assertIn('upstream = "api:8081"', config)
        self.assertIn('root_key = "${UNKEY_VERIFY_ROOT_KEY}"', config)
        self.assertIn('key_space_ids = ["${UNKEY_KEYSPACE_ID}"]', config)
        self.assertIn("enabled = true", config)
        self.assertNotIn("enabled = false", config)

    def test_profile_only_preflight_rejects_missing_inputs_wrong_ids_and_shared_credentials(self):
        valid = fixture_environment()
        # `compose config` re-escapes literal dollars for round-trip safety.
        # The container receives single dollars when Compose executes the command.
        script = profile_config(valid)["services"]["unkey-preflight"]["command"][0].replace("$$", "$")
        for name in ("UNKEY_MANAGEMENT_ROOT_KEY", "UNKEY_VERIFY_ROOT_KEY", "UNKEY_API_ID", "UNKEY_KEYSPACE_ID"):
            env = dict(valid)
            env.pop(name)
            result = subprocess.run(["/bin/sh", "-ec", script], env=env, text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(name, result.stderr)
            self.assertNotIn(valid["UNKEY_MANAGEMENT_ROOT_KEY"], result.stderr)
            self.assertNotIn(valid["UNKEY_VERIFY_ROOT_KEY"], result.stderr)
        for changes in (
            {"UNKEY_API_ID": "ks_wrong"},
            {"UNKEY_KEYSPACE_ID": "api_wrong"},
            {"UNKEY_API_ID": "api_invalid/path"},
            {"UNKEY_VERIFY_ROOT_KEY": 'unsafe"\nconfiguration'},
            {"UNKEY_VERIFY_ROOT_KEY": valid["UNKEY_MANAGEMENT_ROOT_KEY"]},
        ):
            result = subprocess.run(["/bin/sh", "-ec", script], env=dict(valid, **changes), capture_output=True)
            self.assertNotEqual(result.returncode, 0)
        self.assertEqual(subprocess.run(["/bin/sh", "-ec", script], env=valid, capture_output=True).returncode, 0)


def live_image_check():
    env = fixture_environment()
    env["COMPOSE_PROJECT_NAME"] = "twitterclone-frontline-check-" + secrets.token_hex(4)
    env["UNKEY_GATEWAY_PORT"] = str(free_port())
    with tempfile.TemporaryDirectory(prefix="twitterclone-frontline-check-") as directory:
        override = Path(directory) / "images.json"
        override.write_text(json.dumps({"services": {
            name: {"image": env["COMPOSE_PROJECT_NAME"] + "-api"} for name in ("api", "migrate", "unkey-preflight")
        }}))
        compose = ["docker", "compose", "--env-file", ".env.example", "-f", "compose.yaml",
                   "-f", str(override), "--profile", "unkey"]
        with tempfile.TemporaryFile(mode="w+") as logs:
            try:
                print("Building isolated API/preflight and starting the pinned real Frontline image.", flush=True)
                result = subprocess.run(compose + ["up", "--build", "--detach", "frontline"],
                                        cwd=ROOT, env=env, stdout=logs, stderr=subprocess.STDOUT)
                if result.returncode:
                    logs.seek(0)
                    text = logs.read()
                    for name in ("POSTGRES_PASSWORD", "CSRF_SECRET", "RATE_LIMIT_SECRET",
                                 "UNKEY_MANAGEMENT_ROOT_KEY", "UNKEY_VERIFY_ROOT_KEY"):
                        text = text.replace(env[name], "[redacted]")
                    raise RuntimeError("Frontline fixture startup failed:\n" + text)
                deadline = time.monotonic() + 30
                while True:
                    try:
                        request = urllib.request.Request(
                            "http://localhost:" + env["UNKEY_GATEWAY_PORT"] + "/v1/me",
                            headers={"X-Unkey-Principal": "forged-fixture", "Cookie": "session=forged"},
                        )
                        urllib.request.urlopen(request, timeout=2)
                        raise AssertionError("Gateway accepted a missing-key request")
                    except urllib.error.HTTPError as response:
                        assert response.code == 401, "Expected gateway rejection before application routing"
                        body = response.read().decode()
                        assert "missing_credentials" in body, "Expected Frontline's missing-credential error"
                        break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise RuntimeError("The pinned Frontline image did not serve within its deadline")
                        time.sleep(0.2)
                missing = dict(env, UNKEY_VERIFY_ROOT_KEY="")
                failed = subprocess.run(compose + ["run", "--rm", "--no-deps", "unkey-preflight"],
                                        cwd=ROOT, env=missing, text=True, capture_output=True)
                assert failed.returncode != 0 and "UNKEY_VERIFY_ROOT_KEY" in failed.stderr
                print("PASS: real pinned Frontline rejects missing keys/forged identity; profile preflight rejects missing verify credential.", flush=True)
                print("NOT RUN: live provider verification, valid-key forwarding, or scoped-credential validation.", flush=True)
            finally:
                subprocess.run(compose + ["down"], cwd=ROOT, env=env, check=True)
                print("Frontline fixture containers/networks removed; named database volume preserved.", flush=True)


if __name__ == "__main__":
    live = "--live" in sys.argv
    if live:
        sys.argv.remove("--live")
    result = unittest.main(exit=False)
    if not result.result.wasSuccessful():
        raise SystemExit(1)
    if live:
        live_image_check()
