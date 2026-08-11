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
                "./.github/workflows/v8-canary.yml",
            ],
        )
        self.assertIn("name: CI required", text)

    def test_old_postmerge_entrypoint_is_absent(self) -> None:
        self.assertFalse((WORKFLOWS / "postmerge-ci.yml").exists())


if __name__ == "__main__":
    unittest.main()
