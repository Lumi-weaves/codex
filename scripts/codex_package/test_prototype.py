#!/usr/bin/env python3

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from codex_package.prototype import PrototypeProfile
from codex_package.prototype import builder_command
from codex_package.prototype import export_profile
from codex_package.prototype import load_profile
from codex_package.prototype import model_backend_builder_command
from codex_package.prototype import model_backend_entrypoint
from codex_package.prototype import prototype_entrypoint
from codex_package.prototype import replace_directory
from codex_package.targets import native_target


class LoadProfileTest(unittest.TestCase):
    def test_loads_native_profile(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "profile.json"
            path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "name": "local-test",
                        "variant": "codex",
                        "target": "native",
                        "cargoProfile": "dev-small",
                    }
                ),
                encoding="utf-8",
            )

            profile = load_profile(path)

        self.assertEqual(profile.name, "local-test")
        self.assertEqual(profile.resolved_target(), native_target())

    def test_rejects_unknown_fields(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "profile.json"
            path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "name": "local-test",
                        "variant": "codex",
                        "target": "native",
                        "cargoProfile": "dev-small",
                        "surprise": True,
                    }
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(RuntimeError, "unknown fields: surprise"):
                load_profile(path)


class ExportProfileTest(unittest.TestCase):
    def setUp(self) -> None:
        self.profile = PrototypeProfile(
            name="local-test",
            variant="codex",
            target="x86_64-unknown-linux-gnu",
            cargo_profile="dev-small",
        )

    def test_builder_command_uses_profile_values(self) -> None:
        command = builder_command(self.profile, Path("/tmp/staging"))

        self.assertEqual(
            command[-8:],
            [
                "--target",
                "x86_64-unknown-linux-gnu",
                "--variant",
                "codex",
                "--cargo-profile",
                "dev-small",
                "--package-dir",
                "/tmp/staging",
            ],
        )

    def test_windows_entrypoint_includes_executable_suffix(self) -> None:
        windows_profile = PrototypeProfile(
            name="windows-test",
            variant="codex-app-server",
            target="x86_64-pc-windows-msvc",
            cargo_profile="dev-small",
        )

        self.assertEqual(
            prototype_entrypoint(windows_profile, Path("C:/prototype")),
            Path("C:/prototype/bin/codex-app-server.exe"),
        )
        self.assertEqual(
            model_backend_entrypoint(windows_profile, Path("C:/prototype")),
            Path("C:/prototype/bin/richcodex-model-backend.exe"),
        )

    def test_backend_builder_uses_frozen_package_and_native_output(self) -> None:
        command = model_backend_builder_command(
            PrototypeProfile(
                name="local-test",
                variant="codex",
                target="native",
                cargo_profile="dev-small",
            ),
            Path("/tmp/staging"),
        )

        self.assertEqual(command[1:4], ["build", "--compile", "src/index.ts"])
        self.assertEqual(
            command[-1], "/tmp/staging/bin/richcodex-model-backend"
        )

    @mock.patch(
        "codex_package.prototype.source_provenance",
        return_value={"sourceRevision": "abc123", "sourceDirty": True},
    )
    def test_exports_to_stable_profile_directory(self, _provenance: mock.Mock) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_root = Path(temp_dir)
            old_destination = output_root / self.profile.name
            old_destination.mkdir()
            (old_destination / "old").write_text("old", encoding="utf-8")

            def fake_run(
                command: list[str], **_kwargs: object
            ) -> subprocess.CompletedProcess[str]:
                if "--package-dir" in command:
                    package_dir = Path(command[command.index("--package-dir") + 1])
                    (package_dir / "codex-package.json").write_text(
                        json.dumps({"version": "0.0.0"}), encoding="utf-8"
                    )
                    (package_dir / "bin").mkdir()
                    (package_dir / "bin" / "codex").write_text(
                        "new", encoding="utf-8"
                    )
                else:
                    Path(command[command.index("--outfile") + 1]).write_text(
                        "backend", encoding="utf-8"
                    )
                return subprocess.CompletedProcess(command, 0)

            destination = export_profile(self.profile, output_root, run=fake_run)
            metadata = json.loads(
                (destination / "codex-package.json").read_text(encoding="utf-8")
            )

            self.assertFalse((destination / "old").exists())
            self.assertEqual((destination / "bin" / "codex").read_text(), "new")
            self.assertEqual(
                (destination / "bin" / "richcodex-model-backend").read_text(),
                "backend",
            )
            self.assertEqual(
                metadata["prototype"],
                {
                    "profile": "local-test",
                    "sourceRevision": "abc123",
                    "sourceDirty": True,
                },
            )
            self.assertEqual(
                metadata["richcodexModelBackend"]["kernel"]["sourceCommit"],
                "cbbfdd8773e68a5dc2391ddeb32f33a225373c1a",
            )

    def test_failed_swap_restores_previous_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            staging = root / "staging"
            destination = root / "destination"
            staging.mkdir()
            destination.mkdir()
            (destination / "old").write_text("old", encoding="utf-8")

            original_rename = Path.rename

            def fail_staging_rename(path: Path, target: Path) -> Path:
                if path == staging:
                    raise OSError("boom")
                return original_rename(path, target)

            with mock.patch.object(
                Path, "rename", autospec=True, side_effect=fail_staging_rename
            ):
                with self.assertRaisesRegex(OSError, "boom"):
                    replace_directory(staging, destination)

            self.assertTrue((destination / "old").is_file())

    def test_prior_cleanup_failure_does_not_fail_successful_publish(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            staging = root / "staging"
            destination = root / "destination"
            staging.mkdir()
            destination.mkdir()
            (staging / "new").write_text("new", encoding="utf-8")
            (destination / "old").write_text("old", encoding="utf-8")

            with mock.patch(
                "codex_package.prototype.shutil.rmtree",
                side_effect=OSError("locked"),
            ):
                with self.assertWarnsRegex(UserWarning, "could not remove prior"):
                    replace_directory(staging, destination)

            self.assertTrue((destination / "new").is_file())


if __name__ == "__main__":
    unittest.main()
