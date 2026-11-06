"""Runner checks that require no Solana toolchain or network."""

import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("security_runner", Path(__file__).parents[1] / "security-tests.py")
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class SecurityRunnerTests(unittest.TestCase):
    def copy_review_tree(self, root):
        matrix = json.loads(runner.MATRIX.read_text())
        target = root / runner.MATRIX.relative_to(runner.ROOT)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(matrix, indent=2) + "\n")
        for relative in matrix["source_inventory"]:
            destination = root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(runner.ROOT / relative, destination)
        tests = root / "molpha-tests/tests"
        tests.mkdir(parents=True, exist_ok=True)
        for original in (runner.ROOT / "molpha-tests/tests").glob("*.rs"):
            shutil.copy2(original, tests / original.name)
        return matrix

    def test_all_public_instructions_and_real_test_references_are_reviewed(self):
        summary = runner.validate_matrix()
        self.assertEqual(summary["schema_version"], 2)
        self.assertEqual(summary["instructions"], 28)
        self.assertGreater(summary["program_sources"], 60)
        self.assertGreater(summary["production_errors"], 100)
        self.assertGreater(summary["account_constraints"], 150)
        self.assertGreater(summary["referenced_tests"], 100)
        self.assertEqual(summary["checks_by_status"]["missing"], 0)
        self.assertEqual(summary["checks_by_status"]["finding"], 0)

    def test_changed_instruction_requires_matrix_review(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.copy_review_tree(root)
            runner.validate_matrix(root)
            path = root / "programs/molpha/src/instructions/submit_attestation.rs"
            path.write_text(path.read_text() + "\n// new security decision\n")
            with self.assertRaisesRegex(ValueError, "stale source hash"):
                runner.validate_matrix(root)

    def test_new_program_source_is_unreviewed(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.copy_review_tree(root)
            (root / "programs/molpha/src/new_boundary.rs").write_text("pub fn boundary() {}\n")
            with self.assertRaisesRegex(ValueError, "source inventory mismatch"):
                runner.validate_matrix(root)

    def test_duplicate_obligation_and_unknown_test_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            matrix = self.copy_review_tree(root)
            obligations = matrix["instructions"][0]["obligations"]
            obligations[1]["id"] = obligations[0]["id"]
            path = root / "molpha-tests/security-matrix.json"
            path.write_text(json.dumps(matrix))
            with self.assertRaisesRegex(ValueError, "duplicate security obligation"):
                runner.validate_matrix(root)

            matrix = self.copy_review_tree(root)
            matrix["instructions"][0]["obligations"][0]["tests"] = ["nope::not_a_test"]
            path.write_text(json.dumps(matrix))
            with self.assertRaisesRegex(ValueError, "unknown exact tests"):
                runner.validate_matrix(root)

    def test_unexplained_or_unsafe_dispositions_are_not_a_clean_signoff(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            matrix = self.copy_review_tree(root)
            row = next(r for r in matrix["account_constraints"] if r["disposition"] == "unreachable")
            row["rationale"] = ""
            path = root / "molpha-tests/security-matrix.json"
            path.write_text(json.dumps(matrix))
            with self.assertRaisesRegex(ValueError, "missing obligation rationale"):
                runner.validate_matrix(root)

            matrix = self.copy_review_tree(root)
            matrix["instructions"][0]["obligations"][0]["disposition"] = "finding"
            path.write_text(json.dumps(matrix))
            with self.assertRaisesRegex(ValueError, "not a clean sign-off"):
                runner.validate_matrix(root)

    def test_mutation_controls_match_current_source_once(self):
        for name, path, before, after, _test in runner.MUTATIONS:
            with self.subTest(mutation=name):
                source = (runner.ROOT / path).read_text()
                changed = runner.apply_mutation(source, before, after)
                self.assertNotEqual(source, changed)

    def test_ambiguous_or_stale_mutation_does_not_count_as_a_kill(self):
        for source in ["no matching anchor", "guard guard"]:
            with self.assertRaises(ValueError):
                runner.apply_mutation(source, "guard", "bypass")


if __name__ == "__main__":
    unittest.main()
