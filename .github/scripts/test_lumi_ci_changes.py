import unittest

from lumi_ci_changes import Areas
from lumi_ci_changes import classify


class LumiCiChangesTest(unittest.TestCase):
    def test_documentation_only_selects_no_compiler_surface(self) -> None:
        self.assertEqual(classify({"docs/example.md"}), Areas())

    def test_rust_and_build_inputs_select_rust(self) -> None:
        for path in (
            "codex-rs/core/src/lib.rs",
            "MODULE.bazel.lock",
            "third_party/v8/BUILD.bazel",
            "tools/argument-comment-lint/src/main.rs",
        ):
            with self.subTest(path=path):
                self.assertEqual(classify({path}), Areas(rust=True))

    def test_sdk_inputs_select_sdk(self) -> None:
        for path in ("sdk/typescript/src/index.ts", "pnpm-lock.yaml"):
            with self.subTest(path=path):
                self.assertEqual(classify({path}), Areas(sdk=True))

    def test_distribution_inputs_select_distribution(self) -> None:
        for path in (
            "scripts/install/install.sh",
            "scripts/release/lumi_shadow_gate.py",
            "scripts/build_codex_package.py",
        ):
            with self.subTest(path=path):
                self.assertEqual(classify({path}), Areas(distribution=True))

    def test_github_change_exercises_every_owned_surface(self) -> None:
        self.assertEqual(
            classify({".github/workflows/lumi-ci.yml"}),
            Areas(rust=True, sdk=True, distribution=True),
        )

    def test_manual_force_exercises_every_owned_surface(self) -> None:
        self.assertEqual(
            classify(set(), force=True),
            Areas(rust=True, sdk=True, distribution=True),
        )


if __name__ == "__main__":
    unittest.main()
