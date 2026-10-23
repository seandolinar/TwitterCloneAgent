"""Run a command against an isolated real Compose stack, then stop that stack.

Credentials stay in memory, images are project-specific, and named volumes are
preserved. This wrapper does not run a live Unkey or Compute deployment.
"""
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile

from test_foundation import ROOT, environment, free_port


def main():
    command = sys.argv[1:]
    if not command:
        raise SystemExit("Usage: python3 tests/with_compose.py <command> [arguments...]")
    env = environment()
    for key in list(env):
        if key.startswith("AUTH_") or key == "SOCIAL_WRITES_PER_MINUTE":
            env.pop(key)
    project = "twitterclone-browser-check-" + secrets.token_hex(4)
    env["COMPOSE_PROJECT_NAME"] = project
    env["WEB_PORT"] = str(free_port())
    env["MAIL_UI_PORT"] = str(free_port())
    while env["MAIL_UI_PORT"] == env["WEB_PORT"]:
        env["MAIL_UI_PORT"] = str(free_port())
    env["APP_ORIGIN"] = "http://localhost:" + env["WEB_PORT"]
    env["WEB_BASE_URL"] = env["APP_ORIGIN"]
    env["MAIL_BASE_URL"] = "http://localhost:" + env["MAIL_UI_PORT"]
    with tempfile.TemporaryDirectory(prefix="twitterclone-compose-check-") as directory:
        override = Path(directory) / "images.json"
        override.write_text(json.dumps({"services": {
            service: {"image": project + "-" + ("api" if service == "migrate" else service)}
            for service in ("api", "migrate", "web")
        }}))
        compose = ["docker", "compose", "--env-file", ".env.example",
                   "-f", "compose.yaml", "-f", str(override)]
        with tempfile.TemporaryFile(mode="w+") as output:
            try:
                print("Starting isolated Compose stack " + project + " (credentials not printed).", flush=True)
                startup = subprocess.run(compose + ["up", "--build", "--detach", "--wait", "--wait-timeout", "120"],
                                         cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT, check=False)
                if startup.returncode:
                    # Build output never contains generated runtime credentials;
                    # redact configured values anyway before reporting diagnostics.
                    output.seek(0)
                    text = output.read()
                    for key in ("POSTGRES_PASSWORD", "CSRF_SECRET", "RATE_LIMIT_SECRET"):
                        text = text.replace(env[key], "[redacted]")
                    print(text, file=sys.stderr)
                    return startup.returncode
                print("Real stack ready at " + env["WEB_BASE_URL"], flush=True)
                return subprocess.run(command, cwd=ROOT, env=env, check=False).returncode
            finally:
                subprocess.run(compose + ["down"], cwd=ROOT, env=env, check=True)
                print("Isolated containers/networks removed; named test database volume preserved.", flush=True)


if __name__ == "__main__":
    raise SystemExit(main())
