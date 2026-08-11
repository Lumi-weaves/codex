import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"


class LumiCiWorkflowsTest(unittest.TestCase):
    def test_owned_automatic_surface_has_no_retired_runner_or_target(self) -> None:
        paths = (
            WORKFLOWS / "blocking-ci.yml",
            WORKFLOWS / "lumi-ci.yml",
            WORKFLOWS / "v8-canary.yml",
            WORKFLOWS / "lumi-release.yml",
        )
        forbidden = (
            "codex-runners",
            "runner: windows-2025",
            "runner: windows-latest",
            "runner: macos-15-xlarge",
            "x86_64-apple-darwin",
            "pc-windows-msvc",
        )

        for path in paths:
            text = path.read_text(encoding="utf-8")
            for value in forbidden:
                with self.subTest(path=path.name, value=value):
                    self.assertNotIn(value, text)

    def test_blocking_entrypoint_calls_only_owned_contracts(self) -> None:
        text = (WORKFLOWS / "blocking-ci.yml").read_text(encoding="utf-8")
        calls = re.findall(r"^\s+uses: (\./\.github/workflows/\S+)$", text, re.MULTILINE)

        self.assertEqual(
            calls,
            [
                "./.github/workflows/lumi-ci.yml",
            ],
        )
        self.assertIn("name: CI required", text)

    def test_old_postmerge_entrypoint_is_absent(self) -> None:
        self.assertFalse((WORKFLOWS / "postmerge-ci.yml").exists())

    def test_reusable_v8_concurrency_cannot_self_lock_with_caller(self) -> None:
        text = (WORKFLOWS / "v8-canary.yml").read_text(encoding="utf-8")

        self.assertIn("group: v8-canary::", text)
        self.assertNotIn("group: ${{ github.workflow }}::", text)

    def test_manual_v8_canary_verifies_published_artifacts(self) -> None:
        text = (WORKFLOWS / "v8-canary.yml").read_text(encoding="utf-8")

        # Manual-only surface remains.
        self.assertIn("workflow_dispatch:", text)
        self.assertNotIn("workflow_call:", text)

        # The 60-minute bound remains.
        self.assertIn("timeout-minutes: 60", text)

        # Both Linux x86_64 sandbox matrix modes remain.
        self.assertIn("variant: release", text)
        self.assertIn("variant: ptrcomp-sandbox", text)

        # The checksum-pinned setup action receives the matrix sandbox.
        self.assertIn("uses: ./.github/actions/setup-rusty-v8", text)
        self.assertIn("sandbox: ${{ matrix.sandbox }}", text)

        # The cold Bazel build/stage/upload path is gone.
        for value in (
            "setup-bazel-ci",
            "run_bazel_with_buildbuddy.py",
            "stage-release-pair",
            "upload-artifact",
        ):
            with self.subTest(value=value):
                self.assertNotIn(value, text)

        # The focused Cargo smoke test remains.
        self.assertIn("test -p codex-v8-poc", text)
        self.assertIn("--features sandbox", text)

    def test_sdk_build_includes_code_mode_host(self) -> None:
        text = (WORKFLOWS / "lumi-ci.yml").read_text(encoding="utf-8")

        self.assertIn("-p codex-cli", text)
        self.assertIn("-p codex-code-mode-host", text)
        self.assertIn("--bin codex-code-mode-host", text)

    def test_expensive_diagnostics_are_manual_or_opt_in(self) -> None:
        blocking = (WORKFLOWS / "blocking-ci.yml").read_text(encoding="utf-8")
        lumi = (WORKFLOWS / "lumi-ci.yml").read_text(encoding="utf-8")
        v8 = (WORKFLOWS / "v8-canary.yml").read_text(encoding="utf-8")

        self.assertNotIn("./.github/workflows/v8-canary.yml", blocking)
        self.assertNotIn("workflow_call:", v8)
        self.assertIn("inputs.full_nextest == true", lumi)
        self.assertIn("-p codex-core --lib --no-fail-fast", lumi)


if __name__ == "__main__":
    unittest.main()
