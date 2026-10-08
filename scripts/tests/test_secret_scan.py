"""Regression tests for the release gate. All inputs below are synthetic."""

import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("secret_scan", Path(__file__).parents[1] / "scan-secrets.py")
SCAN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCAN)


class SecretScanTests(unittest.TestCase):
    def test_keypairs_and_nested_manifests_are_detected(self):
        self.assertEqual(SCAN.detect("key.json", json.dumps([0] * 64).encode()), {"solana-keypair"})
        self.assertEqual(SCAN.detect("nodes.json", b'{"nodes":[{"evm":{"privateKey":"abcd"}}]}'), {"structured-secret"})
        self.assertEqual(SCAN.detect("fixture.json", b'{"secretKeys":["abcd"]}'), {"structured-secret"})

    def test_public_keys_and_placeholders_are_not_exemptions(self):
        self.assertEqual(SCAN.detect("public.json", b'{"publicKey":"abcd"}'), set())
        self.assertEqual(SCAN.detect("key.json", json.dumps([0] * 32).encode()), set())
        self.assertEqual(SCAN.detect(".env.example", b'TOKEN=<your-token>\nAPI_KEY=${API_KEY}\n'), set())
        self.assertEqual(SCAN.detect("config.json", b'{"privateKey":"<placeholder>"}'), set())

    def test_populated_env_and_pem_are_detected(self):
        self.assertEqual(SCAN.detect(".env.local", b'export TOKEN="real-value"\n'), {"populated-env"})
        marker = b"-----BEGIN " + b"PRIVATE KEY-----"
        self.assertEqual(SCAN.detect("key.pem", marker), {"credential-pattern"})

    def test_labelled_text_keys_are_detected(self):
        key = b"Private Key: " + b"A" * 87 + b"\nPublic Key: " + b"B" * 44 + b"\n"
        self.assertEqual(SCAN.detect("keypair.txt", key), {"labelled-private-key"})
        self.assertEqual(SCAN.detect("notes.txt", b"Public Key: " + b"B" * 44), set())
        self.assertEqual(SCAN.detect("code.js", b'privateKey: evm.privateKey.toString("hex"),'), set())

    def test_provider_token_patterns_are_detected(self):
        token = "ghp_" + "a" * 36
        self.assertEqual(SCAN.detect("config.txt", token.encode()), {"credential-pattern"})

    def test_allowlist_binds_content_category_and_current_path(self):
        raw = b'{"secretKeys":["abcd"]}'
        entry = {"sha256": hashlib.sha256(raw).hexdigest(), "categories": ["structured-secret"], "current_paths": ["fixture.json"]}
        self.assertTrue(SCAN.allowed([entry], "worktree", "fixture.json", raw, "structured-secret"))
        self.assertFalse(SCAN.allowed([entry], "worktree", "new-key.json", raw, "structured-secret"))
        self.assertFalse(SCAN.allowed([entry], "worktree", "fixture.json", raw + b" ", "structured-secret"))
        self.assertFalse(SCAN.allowed([entry], "worktree", "fixture.json", raw, "credential-pattern"))
        self.assertTrue(SCAN.allowed([entry], "history", "old.json", raw, "structured-secret"))

    def test_ignored_forced_addition_is_scanned_from_index(self):
        original_root = SCAN.ROOT
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            (root / ".gitignore").write_text("key.json\n")
            (root / "key.json").write_text(json.dumps([0] * 64))
            SCAN.ROOT = root
            try:
                self.assertNotIn("key.json", [path for path, _ in SCAN.files("worktree")])
                subprocess.run(["git", "-C", str(root), "add", "-f", "key.json"], check=True)
                # Working-tree edits cannot conceal an already staged credential.
                (root / "key.json").write_text('{"publicKey":"abcd"}')
                indexed = list(SCAN.files("staged"))
                self.assertEqual(len(indexed), 1)
                self.assertEqual(SCAN.detect(*indexed[0]), {"solana-keypair"})
            finally:
                SCAN.ROOT = original_root

    def test_deleted_historical_secret_still_fails_without_logging_its_value(self):
        original_root, original_allowlist = SCAN.ROOT, SCAN.ALLOWLIST
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            raw = b'{"privateKey":"synthetic-history-secret"}'
            (root / "old.json").write_bytes(raw)
            subprocess.run(["git", "-C", str(root), "add", "old.json"], check=True)
            subprocess.run(["git", "-C", str(root), "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture"], check=True)
            subprocess.run(["git", "-C", str(root), "rm", "-q", "old.json"], check=True)
            subprocess.run(["git", "-C", str(root), "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "remove fixture"], check=True)
            allowlist = root / "allowlist.json"
            allowlist.write_text('{"version":1,"entries":[]}')
            SCAN.ROOT, SCAN.ALLOWLIST = root, allowlist
            try:
                self.assertEqual(list(SCAN.files("worktree")), [("allowlist.json", allowlist.read_bytes())])
                self.assertIn(("old.json", raw), list(SCAN.files("history")))
                import contextlib
                import io
                from unittest.mock import patch
                output = io.StringIO()
                with patch("sys.argv", ["scan-secrets.py", "--history"]), contextlib.redirect_stdout(output):
                    self.assertEqual(SCAN.main(), 1)
                self.assertIn("structured-secret", output.getvalue())
                self.assertNotIn("synthetic-history-secret", output.getvalue())
            finally:
                SCAN.ROOT, SCAN.ALLOWLIST = original_root, original_allowlist


if __name__ == "__main__":
    unittest.main()
