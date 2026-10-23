"""Standard-library checks for the container foundation, not MVP acceptance."""

import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parent.parent


def environment():
    env = os.environ.copy()
    # Override inherited application credentials/settings with the public template.
    for line in (ROOT / ".env.example").read_text().splitlines():
        if line and not line.startswith("#"):
            key, value = line.split("=", 1)
            env[key] = value
    env["POSTGRES_PASSWORD"] = secrets.token_hex(32)
    env["CSRF_SECRET"] = secrets.token_hex(32)
    env["RATE_LIMIT_SECRET"] = secrets.token_hex(32)
    return env


class FoundationTests(unittest.TestCase):
    def test_generated_and_private_files_are_ignored_but_sources_are_not(self):
        ignored = {
            ".env", ".env.local", "apps/api/.env.production",
            "target/debug/api", "apps/api/target/debug/api",
            "node_modules/example/index.js", "apps/web/node_modules/example/index.js",
            "apps/web/dist/index.js", "apps/web/.output/server/index.mjs",
            "apps/web/.tanstack/cache.json", "apps/web/.vite/cache.json",
            "apps/web/.cache/cache.json", "apps/web/tsconfig.tsbuildinfo",
            "coverage/report.json", "playwright-report/index.html",
            "test-results/result.json", "blob-report/report.zip",
            "server.log", "npm-debug.log.1", ".idea/workspace.xml",
            "example.swp", "example.swo", "example~", ".DS_Store",
            "._example", "Thumbs.db", "Desktop.ini",
            ".delta/worktrees/example/README.md",
        }
        kept = {
            ".env.example", ".env.production.example", "apps/api/.env.example",
            ".env.sample", ".env.production.sample", ".env.template", ".env.production.template",
            "Cargo.lock", "apps/api/Cargo.lock", "package-lock.json", "apps/web/package-lock.json",
            "rust-toolchain.toml", "compose.yaml", "apps/api/src/main.rs",
            "apps/web/src/routeTree.gen.ts", ".vscode/settings.json",
            ".agents/skills/example/SKILL.md", ".delta/skills/example/SKILL.md",
            ".delta/prepare", "openspec/config.yaml",
        }
        result = subprocess.run(
            ["git", "check-ignore", "--no-index", "--stdin"],
            input="\n".join(sorted(ignored | kept)) + "\n",
            text=True, capture_output=True, cwd=ROOT, check=False,
        )
        self.assertIn(result.returncode, (0, 1))
        self.assertEqual(set(result.stdout.splitlines()), ignored)

    def test_compose_has_private_backends_and_no_required_unkey(self):
        env = environment()
        result = subprocess.run(
            ["docker", "compose", "--env-file", ".env.example", "config", "--format", "json"],
            cwd=ROOT, env=env, capture_output=True, text=True, check=True,
        )
        config = json.loads(result.stdout)
        services = config["services"]
        self.assertEqual(set(services), {"web", "api", "db", "mail", "migrate"})
        for name in ("api", "db", "migrate"):
            self.assertFalse(services[name].get("ports"))
        for name in ("web", "mail"):
            for port in services[name]["ports"]:
                self.assertEqual(port["host_ip"], "127.0.0.1")
            self.assertTrue(
                any(
                    not config["networks"][network].get("internal", False)
                    for network in services[name]["networks"]
                ),
                f"{name} needs an ingress network for its published loopback port",
            )
        for name in ("web", "api"):
            self.assertTrue(services[name]["read_only"])
            self.assertIn("ALL", services[name]["cap_drop"])
            self.assertIn("no-new-privileges:true", services[name]["security_opt"])
        self.assertTrue(config["networks"]["data"]["internal"])
        self.assertEqual(
            services["api"]["depends_on"]["migrate"]["condition"],
            "service_completed_successfully",
        )
        self.assertEqual(services["api"]["environment"]["UNKEY_MANAGEMENT_ROOT_KEY"], "")
        self.assertEqual(services["api"]["environment"]["UNKEY_API_ID"], "")
        self.assertEqual(services["api"]["environment"]["UNKEY_KEYSPACE_ID"], "")
        for key in ("CSRF_SECRET", "RATE_LIMIT_SECRET"):
            self.assertGreaterEqual(len(env[key]), 32)
            for name in ("api", "migrate"):
                self.assertTrue(services[name]["environment"][key] == env[key])
            self.assertNotIn(key, services["web"]["environment"])
        self.assertTrue(env["CSRF_SECRET"] != env["RATE_LIMIT_SECRET"])
        self.assertTrue(services["db"]["volumes"])

    def test_private_runtime_values_are_required(self):
        for key in ("POSTGRES_PASSWORD", "CSRF_SECRET", "RATE_LIMIT_SECRET"):
            for value in (None, ""):
                with self.subTest(key=key, value=value):
                    env = environment()
                    if value is None:
                        env.pop(key)
                    else:
                        env[key] = value
                    result = subprocess.run(
                        ["docker", "compose", "--env-file", ".env.example", "config", "--quiet"],
                        cwd=ROOT, env=env, capture_output=True, text=True, check=False,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(key, result.stderr)

    def test_build_contexts_are_allowlisted(self):
        for name in ("api", "web"):
            rules = (ROOT / "apps" / name / ".dockerignore").read_text().splitlines()
            effective = [line for line in rules if line and not line.startswith("#")]
            self.assertEqual(effective[0], "**")
            self.assertIn("**/.env", effective)
            self.assertIn("**/.env.*", effective)
            self.assertIn("!src/**", effective)


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def wait_for_http(url, marker=b"", process=None):
    deadline = time.monotonic() + 90
    while True:
        if process is not None and process.poll() is not None:
            raise RuntimeError("Compose exited before the application became available")
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                if response.status == 200 and marker in response.read():
                    return
        except (OSError, urllib.error.URLError):
            pass
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Service did not become ready at {url}")
        time.sleep(0.25)

def verification_email_smoke(env):
    user_id = str(uuid.uuid4())
    suffix = uuid.uuid4().hex[:16]
    recipient = "mail-" + suffix + "@example.test"

    def query(statement):
        result = subprocess.run(
            [
                "docker", "compose", "exec", "-T", "db", "psql",
                "-U", "twitterclone", "-d", "twitterclone",
                "--no-psqlrc", "--tuples-only", "--no-align", "--set", "ON_ERROR_STOP=1",
            ],
            input=statement, cwd=ROOT, env=env, capture_output=True, text=True, check=True,
        )
        return result.stdout.strip()

    # Only UUID-generated identifiers and fixed fixture text enter this test SQL.
    # The account is not a real login credential; no raw token is inserted.
    query(
        "BEGIN;"
        "INSERT INTO users (id, email_normalized, handle, password_hash, display_name) "
        f"VALUES ('{user_id}', '{recipient}', 'mail_{suffix}', "
        "'test-fixture-not-a-password-hash', 'Mail fixture');"
        "INSERT INTO delivery_jobs (id, kind, user_id) "
        f"VALUES ('{uuid.uuid4()}', 'verification_email', '{user_id}');"
        "COMMIT;"
    )
    mail_origin = "http://localhost:" + env["MAIL_UI_PORT"]
    deadline = time.monotonic() + 20
    while True:
        with urllib.request.urlopen(mail_origin + "/api/v1/messages", timeout=2) as response:
            messages = json.loads(response.read())["messages"]
        matching = [
            message for message in messages
            if any(address.get("Address") == recipient for address in message.get("To", []))
        ]
        queued = query(f"SELECT count(*) FROM delivery_jobs WHERE user_id = '{user_id}';")
        if matching and queued == "0":
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("The real SMTP worker did not deliver and acknowledge the fixture")
        time.sleep(0.25)
    with urllib.request.urlopen(
        mail_origin + "/api/v1/message/" + matching[0]["ID"], timeout=2,
    ) as response:
        message = json.loads(response.read())
    links = [
        line.strip() for line in message["Text"].splitlines()
        if line.strip().startswith(env["APP_ORIGIN"] + "/verify-email?")
    ]
    assert len(links) == 1, "Expected one verification link at the configured origin"
    token = urllib.parse.parse_qs(urllib.parse.urlsplit(links[0]).query)["token"][0]
    assert len(token) == 64 and all(c in "0123456789abcdef" for c in token)
    digest = hashlib.sha256(token.encode()).hexdigest()
    # The only token-related SQL value is its digest, never the bearer value.
    stored = query(
        "SELECT count(*) FROM account_tokens "
        f"WHERE user_id = '{user_id}' AND token_digest = decode('{digest}', 'hex') "
        "AND purpose = 'email_verification' AND consumed_at IS NULL "
        "AND expires_at > now() AND expires_at <= created_at + interval '24 hours';"
    )
    assert stored == "1", "Expected only a valid digest-backed verification token"
    query(f"DELETE FROM users WHERE id = '{user_id}';")
    print("PASS: real SMTP verification mail delivered to Compose Mailpit; only a token digest persisted", flush=True)


def live_smoke(acceptance=False):
    env = environment()
    env["COMPOSE_PROJECT_NAME"] = "twitterclone-check-" + secrets.token_hex(4)
    env["WEB_PORT"] = str(free_port())
    env["MAIL_UI_PORT"] = str(free_port())
    env["APP_ORIGIN"] = "http://localhost:" + env["WEB_PORT"]

    subprocess.run(
        ["docker", "compose", "build"], cwd=ROOT, env=env, check=True,
    )
    for name in ("api", "web"):
        image = f"twitterclone-{name}:local"
        result = subprocess.run(
            ["docker", "run", "--rm", "--network", "none", "--entrypoint", "id", image, "-u"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        )
        if int(result.stdout.strip()) == 0:
            raise AssertionError(f"{name} runs as root")
        print(f"PASS: {name} image runs as a non-root user", flush=True)

    # Logs can contain application data. Keep them ephemeral, not in the repo.
    with tempfile.TemporaryFile(mode="w+") as logs:
        process = subprocess.Popen(
            ["docker", "compose", "up", "--build"],
            cwd=ROOT, env=env, stdout=logs, stderr=subprocess.STDOUT,
        )
        try:
            wait_for_http(
                env["APP_ORIGIN"] + "/",
                b"The foundation is in place.",
                process,
            )
            wait_for_http(env["APP_ORIGIN"] + "/healthz", process=process)
            wait_for_http("http://localhost:" + env["MAIL_UI_PORT"], process=process)
            subprocess.run(
                ["docker", "compose", "exec", "-T", "api", "twitter-clone-api", "healthcheck"],
                cwd=ROOT, env=env, check=True,
            )
            verification_email_smoke(env)
            if acceptance:
                browser_env = dict(env)
                browser_env["WEB_BASE_URL"] = env["APP_ORIGIN"]
                browser_env["MAIL_BASE_URL"] = "http://localhost:" + env["MAIL_UI_PORT"]
                subprocess.run(
                    ["npm", "--prefix", "apps/web", "run", "test:e2e"],
                    cwd=ROOT, env=browser_env, check=True,
                )
            print("PASS: exact docker compose up --build serves web, mail, and API health", flush=True)
            print("PASS: no host Rust/Node or Unkey credentials used by the stack", flush=True)
        except BaseException:
            logs.seek(0)
            print(logs.read(), file=sys.stderr)
            raise
        finally:
            subprocess.run(
                ["docker", "compose", "down"], cwd=ROOT, env=env, check=True,
            )
            process.wait(timeout=15)
            print("Stopped isolated test containers; named database volume preserved.", flush=True)


if __name__ == "__main__":
    if not shutil.which("docker"):
        raise SystemExit("Docker CLI and Compose are required for foundation checks.")
    live = "--live" in sys.argv
    acceptance = "--acceptance" in sys.argv
    if acceptance:
        sys.argv.remove("--acceptance")
        if not live:
            raise SystemExit("--acceptance requires --live")
    if live:
        sys.argv.remove("--live")
    result = unittest.main(exit=False)
    if not result.result.wasSuccessful():
        raise SystemExit(1)
    if live:
        live_smoke(acceptance=acceptance)
