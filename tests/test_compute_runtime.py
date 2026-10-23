"""Local production-image checks with isolated TLS PostgreSQL/ingress fixtures.

No source upload, deployment, real provider credentials, or outbound network.
Named fixture volumes are preserved; only this script's containers are removed.
"""
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
IMAGE = "twitterclone-combined:check"


def command(args, env=None, text=None, check=True):
    result = subprocess.run(args, input=text, env=env, cwd=ROOT, text=True, capture_output=True)
    if check and result.returncode:
        # No command/env/body/log dump: fixture credentials must remain private.
        raise RuntimeError("Isolated Compute command failed: " + " ".join(args[:3]))
    return result


def docker(*args, **kwargs):
    return command(["docker", *args], **kwargs)


def main():
    tag = "twitterclone-compute-check-" + secrets.token_hex(4)
    containers = []
    volumes = []
    network = tag + "-network"
    # All secrets/certificates live only under the turn's private scratch root.
    scratch = os.environ.get("DELTA_SCRATCH_DIR")
    with tempfile.TemporaryDirectory(prefix="compute-fixture-", dir=scratch) as temporary:
        folder = Path(temporary)
        cert = folder / "server.crt"
        key = folder / "server.key"
        ca = folder / "ca.crt"
        ca_key = folder / "ca.key"
        csr = folder / "server.csr"
        extensions = folder / "server.ext"
        extensions.write_text("subjectAltName=DNS:database.fixture,DNS:preview-database.fixture\n"
                              "basicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
        command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                 "-keyout", str(ca_key), "-out", str(ca), "-days", "1",
                 "-subj", "/CN=Isolated fixture CA"])
        command(["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes",
                 "-keyout", str(key), "-out", str(csr), "-subj", "/CN=database.fixture"])
        command(["openssl", "x509", "-req", "-in", str(csr), "-CA", str(ca), "-CAkey", str(ca_key),
                 "-CAcreateserial", "-out", str(cert), "-days", "1", "-extfile", str(extensions)])
        # Public certificate only; the app never receives the server private key.
        docker("build", "-f", "infra/compute/Dockerfile", "-t", IMAGE, ".")
        docker("network", "create", "--internal", network)
        records = {}
        try:
            for environment, host, port in [
                ("production", "database.fixture", "8089"),
                ("preview", "preview-database.fixture", "8090"),
            ]:
                db = tag + "-" + environment + "-db"
                volume = tag + "-" + environment + "-data"
                app = tag + "-" + environment
                db_password = secrets.token_hex(32)
                env = os.environ.copy()
                env.update(POSTGRES_PASSWORD=db_password, POSTGRES_USER="fixture",
                           POSTGRES_DB="twitterclone_compute_" + environment)
                volumes.append(volume)
                docker("volume", "create", volume)
                containers.append(db)
                docker("run", "-d", "--name", db, "--network", network, "--network-alias", host,
                       "--network-alias", "wrong-" + environment + ".fixture",
                       "-v", volume + ":/var/lib/postgresql",
                       "-v", str(folder) + ":/tls:ro", "-e", "POSTGRES_PASSWORD",
                       "-e", "POSTGRES_USER", "-e", "POSTGRES_DB", "--entrypoint", "sh",
                       "postgres:18.0-bookworm", "-ec",
                       "install -o postgres -g postgres -m 600 /tls/server.key /tmp/server.key; "
                       "install -o postgres -g postgres -m 644 /tls/server.crt /tmp/server.crt; "
                       "exec docker-entrypoint.sh postgres -c ssl=on "
                       "-c ssl_cert_file=/tmp/server.crt -c ssl_key_file=/tmp/server.key",
                       env=env)
                for _ in range(100):
                    if docker("exec", db, "pg_isready", "-h", "127.0.0.1", "-U", "fixture",
                              "-d", env["POSTGRES_DB"], check=False).returncode == 0:
                        break
                    time.sleep(0.1)
                else:
                    raise RuntimeError("Isolated TLS PostgreSQL did not become ready")
                app_env = os.environ.copy()
                app_env.update({
                    "PORT": port, "APP_ORIGIN": "https://" + environment + ".example.com",
                    "DATABASE_URL": "postgres://fixture:" + db_password + "@" + host + "/" +
                                    env["POSTGRES_DB"] + "?sslmode=verify-full&sslrootcert=/certs/ca.crt",
                    "CSRF_SECRET": secrets.token_hex(32), "RATE_LIMIT_SECRET": secrets.token_hex(32),
                    "SMTP_HOST": "smtp.fixture.invalid", "SMTP_PORT": "587", "SMTP_TLS": "true",
                    "SMTP_USERNAME": environment + "-fixture", "SMTP_PASSWORD": secrets.token_hex(32),
                    "EMAIL_FROM": "noreply@" + environment + ".example.com",
                    "UNKEY_MANAGEMENT_ROOT_KEY": secrets.token_hex(32),
                    "UNKEY_API_ID": "api_fixture_" + environment,
                    "UNKEY_KEYSPACE_ID": "ks_fixture_" + environment,
                })
                runtime_names = ["PORT", "APP_ORIGIN", "DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET",
                                 "SMTP_HOST", "SMTP_PORT", "SMTP_TLS", "SMTP_USERNAME", "SMTP_PASSWORD",
                                 "EMAIL_FROM", "UNKEY_MANAGEMENT_ROOT_KEY", "UNKEY_API_ID", "UNKEY_KEYSPACE_ID"]
                args = ["run", "-d", "--name", app, "--network", network,
                        "--network-alias", "combined-" + environment,
                        "--read-only", "--tmpfs", "/tmp", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges:true", "-v", str(ca) + ":/certs/ca.crt:ro"]
                for name in runtime_names:
                    args += ["-e", name]
                containers.append(app)
                docker(*args, IMAGE, env=app_env)

                def ready_status(name=app, ingress_port=port):
                    result = docker("exec", name, "node", "-e",
                                    "fetch('http://127.0.0.1:" + ingress_port +
                                    "/readyz').then(r=>process.exit(r.status===200?0:1)).catch(()=>process.exit(2))",
                                    check=False)
                    return result.returncode

                for _ in range(100):
                    if ready_status() == 0:
                        break
                    if docker("inspect", "-f", "{{.State.Running}}", app).stdout.strip() != "true":
                        logs = docker("logs", app, check=False)
                        safe = logs.stdout + logs.stderr
                        for private in [db_password, *(app_env[name] for name in [
                            "DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET",
                            "SMTP_PASSWORD", "UNKEY_MANAGEMENT_ROOT_KEY"])]:
                            safe = safe.replace(private, "[REDACTED]")
                        raise RuntimeError("Combined image exited before readiness:\n" + safe)
                    time.sleep(0.1)
                else:
                    raise RuntimeError("Combined image did not become ready")
                user = str(uuid.uuid4())
                target = str(uuid.uuid4())
                target_post = str(uuid.uuid4())
                session = secrets.token_hex(32)
                digest = hashlib.sha256(session.encode()).hexdigest()
                sql = (
                    f"INSERT INTO users(id,email_normalized,handle,password_hash,verified_at,display_name) VALUES "
                    f"('{user}','owner@example.test','compute_{environment}','not-a-login-hash',now(),'Owner'),"
                    f"('{target}','target@example.test','target_{environment}','not-a-login-hash',now(),'Target');"
                    f"INSERT INTO sessions(token_digest,user_id,expires_at) VALUES(decode('{digest}','hex'),'{user}',now()+interval '7 days');"
                    f"INSERT INTO api_keys(key_id,user_id,keyspace_id,label,masked_hint,permissions,expires_at) VALUES("
                    f"'key_compute_fixture','{user}','ks_fixture_{environment}','Fixture','masked',"
                    "ARRAY['profiles.read','posts.read','feed.read','posts.write','follows.write','likes.write'],now()+interval '1 day');"
                )
                if environment == "production":
                    sql += f"INSERT INTO posts(id,author_id,body) VALUES('{target_post}','{target}','Target fixture post');"
                docker("exec", "-i", db, "psql", "-U", "fixture", "-d", env["POSTGRES_DB"],
                       "-v", "ON_ERROR_STOP=1", text=sql)
                principal = json.dumps({"version": "v1", "type": "API_KEY", "subject": user,
                    "identity": {"externalId": user}, "source": {"key": {
                        "keyId": "key_compute_fixture", "keySpaceId": "ks_fixture_" + environment,
                        "permissions": ["profiles.read", "posts.read", "feed.read", "posts.write", "follows.write", "likes.write"]}}})
                records[environment] = dict(app=app, db=db, env=app_env, db_name=env["POSTGRES_DB"],
                    port=port, user=user, target=target, target_post=target_post,
                    principal=principal, session=session, runtime_args=args, ready=ready_status)

            # Real configuration/data isolation assertions, using fixtures only.
            for name in ["DATABASE_URL", "CSRF_SECRET", "RATE_LIMIT_SECRET", "UNKEY_API_ID",
                         "UNKEY_KEYSPACE_ID", "UNKEY_MANAGEMENT_ROOT_KEY", "SMTP_PASSWORD", "APP_ORIGIN"]:
                assert records["production"]["env"][name] != records["preview"]["env"][name]
            prod, preview = records["production"], records["preview"]
            client_env = os.environ.copy()
            client_env.update(PROD_URL="http://combined-production:" + prod["port"],
                PREVIEW_URL="http://combined-preview:" + preview["port"], PROD_PRINCIPAL=prod["principal"],
                PREVIEW_PRINCIPAL=preview["principal"], PROD_USER=prod["user"],
                SESSION_COOKIE="__Host-session=" + prod["session"], TARGET_USER=prod["target"],
                TARGET_POST=prod["target_post"])

            def client(phase):
                client_env["PHASE"] = phase
                args = ["run", "--rm", "--network", network,
                        "-v", str(ROOT / "tests/compute-client.mjs") + ":/client.mjs:ro"]
                for name in ["PROD_URL", "PREVIEW_URL", "PROD_PRINCIPAL", "PREVIEW_PRINCIPAL",
                             "PROD_USER", "SESSION_COOKIE", "TARGET_USER", "TARGET_POST", "PHASE"]:
                    args += ["-e", name]
                result = docker(*args, "node:24.18.1-bookworm-slim", "node", "/client.mjs", env=client_env)
                print(result.stdout.strip(), flush=True)

            client("initial")
            # Restart uses the same volume and reruns migrations, without resets.
            docker("restart", "-t", "15", prod["app"])
            for _ in range(100):
                if prod["ready"]() == 0:
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError("Combined restart failed readiness")
            client("persisted")
            docker("exec", "-i", prod["db"], "psql", "-U", "fixture", "-d", prod["db_name"],
                   "-v", "ON_ERROR_STOP=1", text="UPDATE api_keys SET revoked_at=now();")
            docker("restart", "-t", "15", prod["app"])
            for _ in range(100):
                if prod["ready"]() == 0:
                    break
                time.sleep(0.1)
            client("revoked")
            # Working production configuration differs only in trust/hostname:
            # neither a missing trusted CA nor a mismatched DNS name may connect.
            for bad_url in [
                prod["env"]["DATABASE_URL"].replace("&sslrootcert=/certs/ca.crt", ""),
                prod["env"]["DATABASE_URL"].replace("@database.fixture/", "@wrong-production.fixture/"),
            ]:
                invalid_env = prod["env"].copy()
                invalid_env["DATABASE_URL"] = bad_url
                args = ["run", "--rm", "--network", network, "-v", str(ca) + ":/certs/ca.crt:ro"]
                for name in runtime_names:
                    args += ["-e", name]
                failure = docker(*args, IMAGE, env=invalid_env, check=False)
                assert failure.returncode != 0, "TLS trust/hostname failure must prevent startup"
                assert "Database connection failed" in failure.stderr
            print("PASS: untrusted database CA and hostname mismatch prevent serving", flush=True)
            # Inspect actual runtime assets/binary and image metadata, not merely
            # .dockerignore patterns. No fixture credential value is printed.
            docker("exec", prod["app"], "node", "-e",
                   "const fs=require('fs');const secrets=['CSRF_SECRET','RATE_LIMIT_SECRET','DATABASE_URL',"
                   "'SMTP_PASSWORD','UNKEY_MANAGEMENT_ROOT_KEY'].map(k=>process.env[k]);"
                   "function scan(p){const s=fs.lstatSync(p);if(s.isDirectory()){for(const n of fs.readdirSync(p))scan(p+'/'+n)}"
                   "else if(s.isFile()){const b=fs.readFileSync(p);if(secrets.some(v=>v&&b.includes(Buffer.from(v))))process.exit(1)}}"
                   "scan('/app');scan('/usr/local/bin/twitter-clone-api');")
            image = docker("image", "inspect", IMAGE).stdout
            for name in ["CSRF_SECRET", "RATE_LIMIT_SECRET", "DATABASE_URL", "SMTP_PASSWORD",
                         "UNKEY_MANAGEMENT_ROOT_KEY"]:
                assert prod["env"][name] not in image
                assert preview["env"][name] not in image
            print("PASS: inspected image metadata/assets/binary contain no fixture credentials", flush=True)
            # A deliberate checksum failure in the isolated preview database must
            # stop migration before either application server can serve.
            docker("exec", "-i", preview["db"], "psql", "-U", "fixture", "-d", preview["db_name"],
                   "-v", "ON_ERROR_STOP=1", text="UPDATE _sqlx_migrations SET checksum=decode('00','hex');")
            docker("restart", "-t", "15", preview["app"])
            docker("wait", preview["app"])
            assert docker("inspect", "-f", "{{.State.ExitCode}}", preview["app"]).stdout.strip() == "1"
            print("PASS: migration checksum failure stops the actual combined image", flush=True)
            docker("stop", "-t", "1", prod["db"])
            assert prod["ready"]() != 0, "database failure must remove readiness"
            print("PASS: TLS PostgreSQL, migrations/restarts, isolation and database-failure readiness", flush=True)
            # Actual Rust child termination must take down the combined container.
            docker("exec", prod["app"], "node", "-e",
                   "const fs=require('fs');for(const p of fs.readdirSync('/proc')){"
                   "if(!/^\\d+$/.test(p))continue;try{const a=fs.readFileSync('/proc/'+p+'/cmdline','utf8').split('\\0');"
                   "if(a[0]==='twitter-clone-api'||a[0]==='/usr/local/bin/twitter-clone-api')process.kill(Number(p),'SIGTERM')}catch{}}")
            docker("wait", prod["app"])
            assert docker("inspect", "-f", "{{.State.ExitCode}}", prod["app"]).stdout.strip() == "1"
            print("PASS: real combined-container sibling termination on Rust exit", flush=True)
        finally:
            for container in reversed(containers):
                docker("rm", "-f", container, check=False)
            docker("network", "rm", network, check=False)
            print("Isolated Compute containers/network removed; named fixture volumes preserved.", flush=True)


if __name__ == "__main__":
    main()
