#!/usr/bin/env python3

from pathlib import Path
import unittest


WORKFLOW = (
    Path(__file__).parents[2] / ".github" / "workflows" / "lumi-manual-installer.yml"
)


class LumiManualInstallerWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = WORKFLOW.read_text(encoding="utf-8")

    def test_is_manual_only_and_non_publishing(self) -> None:
        self.assertIn("workflow_dispatch:", self.text)
        for trigger in ("push:", "pull_request:", "schedule:"):
            self.assertNotIn(trigger, self.text)
        self.assertNotIn("contents: write", self.text)
        self.assertNotIn("gh release", self.text)

    def test_builds_only_supported_lumi_targets(self) -> None:
        for target in (
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-musl",
            "aarch64-apple-darwin",
        ):
            self.assertIn(target, self.text)
        self.assertNotIn("x86_64-apple-darwin", self.text)
        self.assertNotIn("pc-windows", self.text)

    def test_kit_is_short_lived_and_carries_takeover_tools(self) -> None:
        self.assertIn("retention-days: 3", self.text)
        self.assertIn("scripts/install/install-kit.sh", self.text)
        self.assertIn("scripts/install/takeover.sh", self.text)
        self.assertIn("codex-package_SHA256SUMS", self.text)
        self.assertIn("lumi_shadow_validate_package.py", self.text)
        self.assertIn("--run", self.text)
        self.assertIn("CODEX_CLI_PATH", self.text)


if __name__ == "__main__":
    unittest.main()
