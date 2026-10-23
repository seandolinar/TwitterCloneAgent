"""Run pinned Rust checks without a host Rust installation or persistent DB."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def main():
    if sys.argv[1:] != ["--inside-fixture"]:
        return subprocess.run([
            sys.executable, str(ROOT / "tests/with_postgres.py"),
            sys.executable, str(Path(__file__).resolve()), "--inside-fixture",
        ], cwd=ROOT).returncode
    env = os.environ.copy()
    env["DATABASE_URL"] = env["DATABASE_URL"].replace("@127.0.0.1:", "@host.docker.internal:")
    checks = [
        "cargo check --quiet --manifest-path apps/api/Cargo.toml --locked",
        "cargo fmt --manifest-path apps/api/Cargo.toml --check",
        "cargo clippy --quiet --manifest-path apps/api/Cargo.toml --all-targets --locked -- -D warnings",
        "cargo test --quiet --manifest-path apps/api/Cargo.toml --lib --locked",
        "cargo test --quiet --manifest-path apps/api/Cargo.toml --tests --locked -- --ignored",
    ]
    with tempfile.TemporaryDirectory(prefix="twitterclone-rust-cache-") as fallback:
        cache = env.get("DELTA_SCRATCH_DIR", fallback)
        return subprocess.run([
            "docker", "run", "--rm", "-v", str(ROOT) + ":/work", "-v", cache + ":/cache",
            "-w", "/work", "-e", "CARGO_HOME=/cache/cargo-home",
            "-e", "CARGO_TARGET_DIR=/tmp/check-target", "-e", "CARGO_BUILD_JOBS=2",
            "-e", "CARGO_PROFILE_DEV_DEBUG=0", "-e", "CARGO_PROFILE_TEST_DEBUG=0",
            "-e", "DATABASE_URL", "-e", "CSRF_SECRET", "-e", "RATE_LIMIT_SECRET",
            "rust:1.99.1-bookworm", "sh", "-c", " && ".join(checks),
        ], cwd=ROOT, env=env).returncode


if __name__ == "__main__":
    raise SystemExit(main())
