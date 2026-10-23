"""Offline checks against the documented Compute API field/matching contract."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent.parent


class ComputeConfigTests(unittest.TestCase):
    def test_runtime_fields_match_the_combined_delivery(self):
        for environment in ("production", "preview"):
            config = json.loads((ROOT / f"infra/compute/runtime.{environment}.json").read_text())
            self.assertEqual(config["environment"], environment)
            self.assertEqual(config["rootDirectory"], ".")
            self.assertTrue((ROOT / config["dockerfile"]).is_file())
            self.assertFalse(config["autoDeploy"])
            self.assertEqual(config["command"], [])
            self.assertEqual(config["port"], 8080)
            self.assertEqual(config["upstreamProtocol"], "http1")
            self.assertEqual(config["shutdownSignal"], "SIGTERM")
            self.assertEqual(config["openapiSpecPath"], "/docs/openapi.json")
            self.assertEqual(config["healthcheck"]["path"], "/readyz")
            self.assertEqual(config["healthcheck"]["method"], "GET")
            self.assertGreaterEqual(config["memoryMib"], 1024)

    def test_disjoint_policies_cover_every_integration_operation_and_no_browser_route(self):
        keyspaces = []
        contract = json.loads((ROOT / "apps/web/public/docs/openapi.json").read_text())
        for environment in ("production", "preview"):
            config = json.loads((ROOT / f"infra/compute/gateway.{environment}.json").read_text())
            self.assertEqual(config["environment"], environment)
            policies = config["policies"]
            self.assertEqual(len(policies), 2)

            def matches(path, policy):
                matcher = policy["match"][0]["path"]["path"]
                return path == matcher["exact"] if "exact" in matcher else path.startswith(matcher["prefix"])

            for path in ["/v1", *contract["paths"]]:
                self.assertEqual(sum(matches(path, policy) for policy in policies), 1, path)
            for path in ["/", "/api/auth/login", "/api/auth/keys", "/api/v1/feed",
                         "/healthz", "/readyz", "/docs/openapi.json", "/assets/app.js", "/v10/me"]:
                self.assertFalse(any(matches(path, policy) for policy in policies), path)
            for policy in policies:
                self.assertTrue(policy["enabled"])
                self.assertEqual(policy["keyauth"]["locations"], [{"bearer": {}}])
                # Issued keys carry their auto-applied limit; no second bucket.
                self.assertNotIn("ratelimits", policy["keyauth"])
                self.assertNotIn("ratelimit", policy)
                self.assertNotIn("logging", policy)
                self.assertNotIn("permissionQuery", policy["keyauth"])
            keyspaces.append(policies[0]["keyauth"]["keyspaces"])
        self.assertNotEqual(keyspaces[0], keyspaces[1])

    def test_combined_build_context_excludes_local_state(self):
        rules = (ROOT / ".dockerignore").read_text().splitlines()
        self.assertEqual(next(rule for rule in rules if rule and not rule.startswith("#")), "**")
        for rule in ["**/.env", "**/.env.*", "**/*.pem", "**/*.key"]:
            self.assertIn(rule, rules)
        dockerfile = (ROOT / "infra/compute/Dockerfile").read_text()
        self.assertIn("USER node", dockerfile)
        self.assertNotIn("COPY . ", dockerfile)
        self.assertNotIn("ARG ", dockerfile)
        self.assertNotIn("POSTGRES_PASSWORD", dockerfile)


if __name__ == "__main__":
    unittest.main()
