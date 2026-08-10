#!/usr/bin/env python3
"""Tests for lumi_shadow_gate.py (allowed/rejected inputs and main-only gate).

Run with: python3 scripts/release/lumi_shadow_gate_test.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent))

from lumi_shadow_gate import GateError  # noqa: E402
from lumi_shadow_gate import require_main_head  # noqa: E402
from lumi_shadow_gate import resolve_commit  # noqa: E402


def run_git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


class SourceGateTest(unittest.TestCase):
    """Fixture: local repo with main, versioned tags, and an unmerged branch."""

    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.repo = Path(self._temp.name) / "repo"
        self.repo.mkdir()
        run_git(self.repo, "init", "-b", "main")
        run_git(self.repo, "config", "user.email", "shadow@example.invalid")
        run_git(self.repo, "config", "user.name", "Shadow Test")

        self.commit1 = self._commit("first", "0.147.0-lumi.4")
        self.commit2 = self._commit("second", "0.147.0-lumi.5")
        self.commit3 = self._commit("third", "0.147.0-lumi.5")

        # Version-matched Lumi tags on main (lightweight and annotated).
        run_git(self.repo, "tag", "rust-v0.147.0-lumi.4", self.commit1)
        run_git(
            self.repo,
            "tag",
            "-a",
            "rust-v0.147.0-lumi.5",
            "-m",
            "annotated canary",
            self.commit2,
        )
        # Version-mismatched Lumi tag on main.
        run_git(self.repo, "tag", "rust-v0.147.0-lumi.9", self.commit1)
        # Non-Lumi tags that must never be accepted.
        run_git(self.repo, "tag", "rust-v0.1.0", self.commit1)
        run_git(self.repo, "tag", "v0.147.0-lumi.4", self.commit1)

        # Unmerged branch: its commit and tags must be rejected.
        run_git(self.repo, "checkout", "-b", "feature/x", self.commit1)
        self.side_commit = self._commit("side work", "0.999.0-lumi.1")
        run_git(self.repo, "tag", "rust-v0.999.0-lumi.1", self.side_commit)
        run_git(self.repo, "checkout", "main")

        # The local branch stands in for refs/remotes/origin/main.
        self.origin_ref = "main"

    def tearDown(self) -> None:
        self._temp.cleanup()

    def _commit(self, message: str, version: str) -> str:
        (self.repo / "file.txt").write_text(f"{message}\n", encoding="utf-8")
        cargo_dir = self.repo / "codex-rs"
        cargo_dir.mkdir(exist_ok=True)
        (cargo_dir / "Cargo.toml").write_text(
            f'[package]\nname = "codex-workspace"\nversion = "{version}"\n',
            encoding="utf-8",
        )
        run_git(self.repo, "add", "file.txt", "codex-rs/Cargo.toml")
        run_git(self.repo, "commit", "-m", message)
        return run_git(self.repo, "rev-parse", "HEAD")

    def _resolve(self, source: str) -> str:
        return resolve_commit(self.repo, source, self.origin_ref)

    def _rejected(self, source: str) -> None:
        with self.assertRaises(GateError):
            self._resolve(source)

    def test_accepts_exact_lightweight_lumi_tag(self) -> None:
        self.assertEqual(self._resolve("rust-v0.147.0-lumi.4"), self.commit1)

    def test_accepts_exact_annotated_lumi_tag(self) -> None:
        self.assertEqual(self._resolve("rust-v0.147.0-lumi.5"), self.commit2)

    def test_accepts_exact_ancestor_sha(self) -> None:
        self.assertEqual(self._resolve(self.commit1), self.commit1)
        self.assertEqual(self._resolve(self.commit3), self.commit3)

    def test_accepts_uppercase_hex_and_normalizes(self) -> None:
        self.assertEqual(self._resolve(self.commit1.upper()), self.commit1)

    def test_rejects_tag_version_mismatch(self) -> None:
        self._rejected("rust-v0.147.0-lumi.9")

    def test_rejects_branch_name(self) -> None:
        self._rejected("main")
        self._rejected("feature/x")

    def test_rejects_pr_ref(self) -> None:
        self._rejected("refs/pull/1/head")

    def test_rejects_prefixed_tag_ref(self) -> None:
        self._rejected("refs/tags/rust-v0.147.0-lumi.4")

    def test_rejects_non_lumi_tag_shapes(self) -> None:
        self._rejected("rust-v0.1.0")
        self._rejected("v0.147.0-lumi.4")
        self._rejected("rust-v0.147.0-lumi")
        self._rejected("rust-v0.147.0-lumi.4.1")
        self._rejected("rust-v0.147.0-lumi.x")
        self._rejected("rust-v0.147.0-lumi.4-rc1")

    def test_rejects_unknown_lumi_tag(self) -> None:
        self._rejected("rust-v9.9.9-lumi.9")

    def test_rejects_short_sha(self) -> None:
        self._rejected(self.commit1[:7])

    def test_rejects_unknown_sha(self) -> None:
        self._rejected("f" * 40)

    def test_rejects_non_ancestor_sha(self) -> None:
        self._rejected(self.side_commit)

    def test_rejects_tag_pointing_at_non_ancestor_commit(self) -> None:
        self._rejected("rust-v0.999.0-lumi.1")

    def test_rejects_invalid_hex_chars(self) -> None:
        self._rejected("z" * 40)

    def test_rejects_empty_and_whitespace_input(self) -> None:
        self._rejected("")
        self._rejected("   ")


class MainHeadGateTest(unittest.TestCase):
    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.repo = Path(self._temp.name) / "repo"
        self.repo.mkdir()
        run_git(self.repo, "init", "-b", "main")
        run_git(self.repo, "config", "user.email", "shadow@example.invalid")
        run_git(self.repo, "config", "user.name", "Shadow Test")
        (self.repo / "file.txt").write_text("main one\n", encoding="utf-8")
        run_git(self.repo, "add", "file.txt")
        run_git(self.repo, "commit", "-m", "main one")
        self.stale_sha = run_git(self.repo, "rev-parse", "HEAD")
        (self.repo / "file.txt").write_text("main two\n", encoding="utf-8")
        run_git(self.repo, "add", "file.txt")
        run_git(self.repo, "commit", "-m", "main two")
        self.main_sha = run_git(self.repo, "rev-parse", "HEAD")
        run_git(self.repo, "checkout", "-b", "feature/y", self.main_sha)
        (self.repo / "file.txt").write_text("feature\n", encoding="utf-8")
        run_git(self.repo, "add", "file.txt")
        run_git(self.repo, "commit", "-m", "feature")
        self.feature_sha = run_git(self.repo, "rev-parse", "HEAD")
        run_git(self.repo, "checkout", "main")

    def tearDown(self) -> None:
        self._temp.cleanup()

    def test_accepts_workflow_head_equal_to_main(self) -> None:
        self.assertEqual(
            require_main_head(self.repo, self.main_sha, "main"), self.main_sha
        )

    def test_rejects_feature_branch_workflow_head(self) -> None:
        with self.assertRaises(GateError):
            require_main_head(self.repo, self.feature_sha, "main")

    def test_rejects_stale_ancestor_workflow_head(self) -> None:
        with self.assertRaises(GateError):
            require_main_head(self.repo, self.stale_sha, "main")

    def test_rejects_missing_origin_ref(self) -> None:
        with self.assertRaises(GateError):
            require_main_head(self.repo, self.main_sha, "origin/absent")

    def test_cli_rejects_feature_workflow_head(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(Path(__file__).resolve().parent / "lumi_shadow_gate.py"),
                "--repo",
                str(self.repo),
                "--source",
                self.main_sha,
                "--main-head",
                self.feature_sha,
                "--origin-ref",
                "main",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not current main", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
