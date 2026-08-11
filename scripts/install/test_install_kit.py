#!/usr/bin/env python3

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


KIT_SCRIPT = Path(__file__).with_name("install-kit.sh")


class InstallKitTest(unittest.TestCase):
    def test_script_is_valid_posix_shell(self) -> None:
        subprocess.run(["sh", "-n", str(KIT_SCRIPT)], check=True)

    def test_noninteractive_default_installs_side_by_side_only(self) -> None:
        with kit_fixture() as fixture:
            result = fixture.run()

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("questions were skipped", result.stdout)
            self.assertFalse(fixture.takeover_log.exists())
            self.assertEqual(
                fixture.install_log.read_text(encoding="utf-8").splitlines(),
                [
                    "--release",
                    "0.148.0-alpha.7-lumi.1",
                    "--target",
                    "x86_64-unknown-linux-musl",
                    "--package-archive",
                    str(
                        fixture.kit
                        / "codex-package-x86_64-unknown-linux-musl.tar.gz"
                    ),
                    "--checksum-manifest",
                    str(fixture.kit / "codex-package_SHA256SUMS"),
                ],
            )
            self.assertTrue((fixture.lumi_root / "tools" / "takeover.sh").is_file())

    def test_explicit_cli_takeover_runs_persistent_helper(self) -> None:
        with kit_fixture() as fixture:
            result = fixture.run("--no-prompt", "--takeover-cli")

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                fixture.takeover_log.read_text(encoding="utf-8").splitlines(),
                ["cli"],
            )

    def test_explicit_desktop_takeover_uses_desktop_command(self) -> None:
        with kit_fixture(os_name="Darwin") as fixture:
            result = fixture.run("--no-prompt", "--takeover-desktop")

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                fixture.takeover_log.read_text(encoding="utf-8").splitlines(),
                ["desktop"],
            )

    def test_side_by_side_rejects_takeover_flags_in_any_order(self) -> None:
        for args in (
            ("--side-by-side", "--takeover-cli"),
            ("--takeover-cli", "--side-by-side"),
        ):
            with self.subTest(args=args), kit_fixture() as fixture:
                result = fixture.run(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("cannot be combined", result.stderr)
                self.assertFalse(fixture.install_log.exists())

    def test_foreign_tools_directory_is_not_overwritten(self) -> None:
        with kit_fixture() as fixture:
            tools = fixture.lumi_root / "tools"
            tools.mkdir(parents=True)
            foreign = tools / "takeover.sh"
            foreign.write_text("foreign\n", encoding="utf-8")

            result = fixture.run("--side-by-side")

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unowned", result.stderr)
            self.assertEqual(foreign.read_text(encoding="utf-8"), "foreign\n")

    def test_foreign_helper_in_owned_tools_directory_is_not_overwritten(self) -> None:
        with kit_fixture() as fixture:
            tools = fixture.lumi_root / "tools"
            tools.mkdir(parents=True)
            (tools / ".lumi-owner").write_text(
                "lumi-codex-takeover-tools-v1\n", encoding="utf-8"
            )
            foreign = tools / "takeover.sh"
            foreign.write_text("foreign\n", encoding="utf-8")

            result = fixture.run("--side-by-side")

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("foreign takeover helper", result.stderr)
            self.assertEqual(foreign.read_text(encoding="utf-8"), "foreign\n")


class KitFixture:
    def __init__(self, root: Path, os_name: str) -> None:
        self.root = root
        self.kit = root / "kit"
        self.home = root / "home"
        self.lumi_root = root / "lumi-root"
        self.bin_dir = root / "bin"
        self.install_log = root / "install.log"
        self.takeover_log = root / "takeover.log"
        self.os_name = os_name

    def __enter__(self) -> "KitFixture":
        self.kit.mkdir()
        self.home.mkdir()
        self.bin_dir.mkdir()
        shutil.copyfile(KIT_SCRIPT, self.kit / "install-lumi.sh")
        (self.kit / "VERSION").write_text(
            "0.148.0-alpha.7-lumi.1\n", encoding="utf-8"
        )
        (self.kit / "TARGET").write_text(
            "x86_64-unknown-linux-musl\n", encoding="utf-8"
        )
        (self.kit / "codex-package_SHA256SUMS").write_text(
            "0" * 64 + "  codex-package-x86_64-unknown-linux-musl.tar.gz\n",
            encoding="utf-8",
        )
        (self.kit / "codex-package-x86_64-unknown-linux-musl.tar.gz").write_bytes(
            b"fixture"
        )
        (self.kit / "install.sh").write_text(
            '#!/bin/sh\nprintf "%s\\n" "$@" >"$TEST_INSTALL_LOG"\n',
            encoding="utf-8",
        )
        (self.kit / "takeover.sh").write_text(
            '#!/bin/sh\nprintf "%s\\n" "$@" >>"$TEST_TAKEOVER_LOG"\n',
            encoding="utf-8",
        )
        (self.bin_dir / "uname").write_text(
            f"#!/bin/sh\nprintf '{self.os_name}\\n'\n", encoding="utf-8"
        )
        for path in (
            self.kit / "install-lumi.sh",
            self.kit / "install.sh",
            self.kit / "takeover.sh",
            self.bin_dir / "uname",
        ):
            path.chmod(0o755)
        return self

    def __exit__(self, *_args: object) -> None:
        shutil.rmtree(self.root)

    def run(self, *args: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env.update(
            {
                "HOME": str(self.home),
                "LUMI_ROOT": str(self.lumi_root),
                "LUMI_INSTALL_DIR": str(self.home / ".local" / "bin"),
                "PATH": f"{self.bin_dir}:/usr/bin:/bin",
                "TEST_INSTALL_LOG": str(self.install_log),
                "TEST_TAKEOVER_LOG": str(self.takeover_log),
            }
        )
        return subprocess.run(
            ["sh", str(self.kit / "install-lumi.sh"), *args],
            capture_output=True,
            check=False,
            env=env,
            text=True,
        )


class kit_fixture:
    def __init__(self, os_name: str = "Linux") -> None:
        self.os_name = os_name
        self.fixture: KitFixture | None = None

    def __enter__(self) -> KitFixture:
        root = Path(tempfile.mkdtemp())
        self.fixture = KitFixture(root, self.os_name)
        return self.fixture.__enter__()

    def __exit__(self, *args: object) -> None:
        assert self.fixture is not None
        self.fixture.__exit__(*args)


if __name__ == "__main__":
    unittest.main()
