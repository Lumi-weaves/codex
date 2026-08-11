import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "lumi-release.yml"


class LumiReleaseWorkflowTest(unittest.TestCase):
    def test_release_matrix_contains_only_owned_targets(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        matrix_start = text.index("      matrix:\n")
        matrix_end = text.index("    steps:\n", matrix_start)
        matrix = text[matrix_start:matrix_end]
        targets = re.findall(r"^\s+- target: (\S+)$", matrix, flags=re.MULTILINE)

        self.assertEqual(
            targets,
            [
                "aarch64-apple-darwin",
                "x86_64-unknown-linux-musl",
                "aarch64-unknown-linux-musl",
            ],
        )

    def test_release_surface_has_no_retired_platforms(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("pc-windows-msvc", text)
        self.assertNotIn("install.ps1", text)
        self.assertNotIn("x86_64-apple-darwin", text)
        self.assertFalse((ROOT / "scripts" / "install" / "install.ps1").exists())

    def test_release_stages_exactly_five_assets(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn('[[ "$count" -eq 5 ]]', text)
        for name in (
            "codex-package-aarch64-apple-darwin.tar.gz",
            "codex-package-aarch64-unknown-linux-musl.tar.gz",
            "codex-package-x86_64-unknown-linux-musl.tar.gz",
            "codex-package_SHA256SUMS",
            "install.sh",
        ):
            self.assertIn(name, text)


if __name__ == "__main__":
    unittest.main()
