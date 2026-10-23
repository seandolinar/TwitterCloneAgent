"""Run a command with a fresh, disposable PostgreSQL container, never a user DB."""

import os
import secrets
import subprocess
import sys
import time


def main():
    command = sys.argv[1:]
    if not command:
        raise SystemExit("Usage: python3 tests/with_postgres.py <command> [arguments...]")
    suffix = secrets.token_hex(6)
    container = "twitterclone-test-" + suffix
    database = "twitterclone_test_" + suffix
    env = dict(
        os.environ,
        POSTGRES_USER="twitterclone_test",
        POSTGRES_DB=database,
        POSTGRES_PASSWORD=secrets.token_hex(32),
        CSRF_SECRET=secrets.token_hex(32),
        RATE_LIMIT_SECRET=secrets.token_hex(32),
    )
    # No bind mount or named developer volume. --rm removes this disposable
    # container's anonymous data volume when the container is removed.
    subprocess.run(
        [
            "docker", "run", "--detach", "--rm", "--name", container,
            "--publish", "127.0.0.1::5432",
            "--env", "POSTGRES_USER", "--env", "POSTGRES_DB", "--env", "POSTGRES_PASSWORD",
            "postgres:18",
        ],
        env=env, check=True, stdout=subprocess.DEVNULL,
    )
    try:
        deadline = time.monotonic() + 45
        while True:
            ready = subprocess.run(
                ["docker", "exec", container, "pg_isready", "-U", env["POSTGRES_USER"], "-d", database],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
            )
            if ready.returncode == 0:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("Disposable PostgreSQL did not become ready")
            time.sleep(0.25)
        port = subprocess.check_output(
            ["docker", "port", container, "5432/tcp"], text=True,
        ).strip().rsplit(":", 1)[1]
        env["DATABASE_URL"] = (
            "postgresql://twitterclone_test:" + env["POSTGRES_PASSWORD"]
            + "@127.0.0.1:" + port + "/" + database
        )
        print("Running against a disposable PostgreSQL database (credentials not printed).", flush=True)
        result = subprocess.run(command, env=env, check=False)
        return result.returncode
    finally:
        subprocess.run(["docker", "stop", container], check=True, stdout=subprocess.DEVNULL)
        print("Disposable PostgreSQL container and its anonymous data volume removed.", flush=True)


if __name__ == "__main__":
    raise SystemExit(main())
