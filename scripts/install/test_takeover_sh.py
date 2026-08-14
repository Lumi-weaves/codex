#!/usr/bin/env python3

import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import textwrap
import unittest


SCRIPT = Path(__file__).with_name("takeover.sh")
PLIST_LABEL = "io.lumi.codex-cli-path"
CLI_NAMES = ("codex", "codex-code-mode-host")


class TakeoverShTest(unittest.TestCase):
    def test_takeover_sh_is_valid_shell(self) -> None:
        syntax_check(SCRIPT)

    def test_takeover_sh_never_evals_or_sources_state(self) -> None:
        contents = SCRIPT.read_text(encoding="utf-8")
        self.assertIsNone(re.search(r"(^|[;&|`\s])eval\s", contents, re.M))
        self.assertNotIn('. "$TAKEOVER_DIR', contents)
        self.assertNotIn("source ", contents)

    def test_help_lists_commands(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self.run_takeover(Path(temp_dir), "--help")
            self.assertEqual(result.returncode, 0, result.stderr)
            for command in ("cli", "desktop", "rollback", "status"):
                self.assertIn(command, result.stdout)

    def test_unknown_command_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            result = self.run_takeover(Path(temp_dir), "bogus")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Unknown command: bogus", result.stderr)

    def test_cli_takeover_clean_and_rollback(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.run_takeover(root, "cli").paths

            for name in CLI_NAMES:
                link = paths.install_dir / name
                self.assertTrue(link.is_symlink())
                self.assertEqual(
                    os.readlink(link), str(paths.lumi_root / "current" / "bin" / name)
                )
                receipt = paths.takeover_dir / f"cli.{name}.receipt"
                self.assertTrue(receipt.is_file())
                self.assert_mode(receipt, 0o600)

            result = self.run_takeover(root, "cli")
            self.assertEqual(result.returncode, 0, result.stderr)

            result = self.run_takeover(root, "rollback")
            self.assertEqual(result.returncode, 0, result.stderr)
            for name in CLI_NAMES:
                self.assertFalse((paths.install_dir / name).exists())
            self.assertFalse(paths.takeover_dir.exists())

    def test_cli_takeover_preserves_unrelated_files(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            other = paths.install_dir / "lumi-codex"
            other.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")

            result = self.run_takeover(root, "cli")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(other.is_file())
            self.assertEqual(
                sorted(p.name for p in paths.install_dir.iterdir()),
                ["codex", "codex-code-mode-host", "lumi-codex"],
            )

    def test_cli_takeover_preserves_prior_regular_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            prior = paths.install_dir / "codex"
            prior.write_text("#!/bin/sh\necho official codex\n", encoding="utf-8")
            prior.chmod(0o755)
            prior_bytes = prior.read_bytes()

            result = self.run_takeover(root, "cli")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((paths.install_dir / "codex").is_symlink())
            backup = paths.takeover_dir / "cli.codex.backup"
            self.assertTrue(backup.is_file())
            self.assert_mode(backup, 0o600)
            self.assertEqual(backup.read_bytes(), prior_bytes)

            result = self.run_takeover(root, "rollback")
            self.assertEqual(result.returncode, 0, result.stderr)
            restored = paths.install_dir / "codex"
            self.assertFalse(restored.is_symlink())
            self.assertTrue(restored.is_file())
            self.assertEqual(restored.read_bytes(), prior_bytes)
            self.assert_mode(restored, 0o755)
            self.assertFalse(paths.takeover_dir.exists())

    def test_cli_takeover_preserves_prior_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            foreign_target = root / "elsewhere" / "codex-tool"
            foreign_target.parent.mkdir(parents=True)
            foreign_target.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            (paths.install_dir / "codex").symlink_to(foreign_target)
            broken_target = root / "elsewhere" / "missing-host"
            (paths.install_dir / "codex-code-mode-host").symlink_to(broken_target)

            result = self.run_takeover(root, "cli")
            self.assertEqual(result.returncode, 0, result.stderr)

            result = self.run_takeover(root, "rollback")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                os.readlink(paths.install_dir / "codex"), str(foreign_target)
            )
            # Broken symlinks are restored exactly too.
            self.assertEqual(
                os.readlink(paths.install_dir / "codex-code-mode-host"),
                str(broken_target),
            )

    def test_cli_takeover_idempotent_keeps_original_prior_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            prior = paths.install_dir / "codex"
            prior.write_text("original\n", encoding="utf-8")

            self.assertEqual(self.run_takeover(root, "cli").returncode, 0)
            self.assertEqual(self.run_takeover(root, "cli").returncode, 0)

            receipt = (paths.takeover_dir / "cli.codex.receipt").read_text(
                encoding="utf-8"
            )
            self.assertIn("prior_kind=regular", receipt)

            result = self.run_takeover(root, "rollback")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (paths.install_dir / "codex").read_text(encoding="utf-8"),
                "original\n",
            )

    def test_cli_takeover_records_unrecorded_link_to_target(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            for name in CLI_NAMES:
                (paths.install_dir / name).symlink_to(
                    paths.lumi_root / "current" / "bin" / name
                )

            result = self.run_takeover(root, "cli")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(
                "prior_kind=symlink",
                (paths.takeover_dir / "cli.codex.receipt").read_text(encoding="utf-8"),
            )

            result = self.run_takeover(root, "rollback")
            self.assertEqual(result.returncode, 0, result.stderr)
            # Rollback restores exactly what was recorded: the same links.
            for name in CLI_NAMES:
                self.assertEqual(
                    os.readlink(paths.install_dir / name),
                    str(paths.lumi_root / "current" / "bin" / name),
                )

    def test_cli_takeover_requires_canonical_install(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            result = self.run_takeover(root, "cli", canonical=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("install.sh first", result.stderr)
            self.assertFalse((result.paths.install_dir / "codex").exists())

    def test_cli_takeover_refuses_directory_at_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            (paths.install_dir / "codex").mkdir()

            result = self.run_takeover(root, "cli")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing to replace directory", result.stderr)
            self.assertTrue((paths.install_dir / "codex").is_dir())
            self.assertFalse((paths.takeover_dir / "cli.codex.receipt").exists())

    def test_cli_takeover_refuses_special_file_at_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            os.mkfifo(paths.install_dir / "codex")

            result = self.run_takeover(root, "cli")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing to replace special file", result.stderr)

    def test_cli_takeover_refuses_symlinked_install_dir(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            real_bin = root / "real-bin"
            real_bin.mkdir()
            paths.install_dir.symlink_to(real_bin)

            result = self.run_takeover(root, "cli")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symlinked directory", result.stderr)

    def test_cli_takeover_refuses_symlinked_state_dir(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            real_state = root / "real-state"
            real_state.mkdir()
            paths.state_home.mkdir(parents=True, exist_ok=True)
            paths.state_root.symlink_to(real_state)

            result = self.run_takeover(root, "cli")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symlinked state directory", result.stderr)

    def test_cli_takeover_refuses_foreign_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.takeover_dir.mkdir(parents=True)
            (paths.takeover_dir / "cli.codex.receipt").write_text(
                "this is not a lumi receipt\n", encoding="utf-8"
            )

            result = self.run_takeover(root, "cli")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Foreign or ambiguous takeover receipt", result.stderr)
            self.assertFalse((paths.install_dir / "codex").exists())
            self.assertEqual(
                (paths.takeover_dir / "cli.codex.receipt").read_text(encoding="utf-8"),
                "this is not a lumi receipt\n",
            )

    def test_cli_takeover_refuses_invalid_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            for env_overrides, message in (
                ({"LUMI_INSTALL_DIR": "relative/bin"}, "absolute path"),
                ({"LUMI_ROOT": "/tmp/evil'root"}, "control characters or quotes"),
            ):
                with self.subTest(env_overrides=env_overrides):
                    result = self.run_takeover(root, "cli", env_overrides=env_overrides)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(message, result.stderr)

    def test_rollback_refuses_drifted_link(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.run_takeover(root, "cli").paths
            drifted = root / "elsewhere" / "codex"
            drifted.parent.mkdir(parents=True)
            drifted.write_text("drifted\n", encoding="utf-8")
            (paths.install_dir / "codex").unlink()
            (paths.install_dir / "codex").symlink_to(drifted)

            result = self.run_takeover(root, "rollback")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("drifted", result.stderr)
            self.assertEqual(os.readlink(paths.install_dir / "codex"), str(drifted))
            self.assertTrue((paths.takeover_dir / "cli.codex.receipt").is_file())
            # The other path must not have been rolled back either.
            self.assertTrue((paths.install_dir / "codex-code-mode-host").is_symlink())

    def test_rollback_refuses_replaced_link(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.run_takeover(root, "cli").paths
            (paths.install_dir / "codex").unlink()
            (paths.install_dir / "codex").write_text("replaced\n", encoding="utf-8")

            result = self.run_takeover(root, "rollback")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("drifted", result.stderr)
            self.assertEqual(
                (paths.install_dir / "codex").read_text(encoding="utf-8"),
                "replaced\n",
            )

    def test_rollback_validates_both_links_before_restoring_either(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.run_takeover(root, "cli").paths
            host = paths.install_dir / "codex-code-mode-host"
            host.unlink()
            host.symlink_to(root / "drifted-host")

            result = self.run_takeover(root, "rollback")

            self.assertNotEqual(result.returncode, 0)
            self.assertTrue((paths.install_dir / "codex").is_symlink())
            self.assertEqual(os.readlink(host), str(root / "drifted-host"))

    def test_rollback_without_takeover_is_noop(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            result = self.run_takeover(root, "rollback")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("nothing to roll back", result.stdout)

    def test_rollback_refuses_unrecorded_link_to_target(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root)
            paths.install_dir.mkdir(parents=True, exist_ok=True)
            (paths.install_dir / "codex").symlink_to(
                paths.lumi_root / "current" / "bin" / "codex"
            )

            result = self.run_takeover(root, "rollback")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("without a takeover receipt", result.stderr)
            self.assertTrue((paths.install_dir / "codex").is_symlink())

    def test_takeover_and_rollback_never_mutate_codex_home_or_profiles(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))
            profiles = [".bashrc", ".zshrc", ".profile", ".bash_profile"]
            for profile in profiles:
                (paths.home / profile).write_text(
                    f"# sentinel {profile}\n", encoding="utf-8"
                )
            config = paths.codex_home / "config.toml"
            config.write_text('model = "official"\n', encoding="utf-8")
            config_bytes = config.read_bytes()
            (paths.home / ".env").write_text("SECRET=sentinel\n", encoding="utf-8")

            for command in ("cli", "desktop", "rollback"):
                result = self.run_takeover(root, command, reuse=paths)
                self.assertEqual(result.returncode, 0, result.stderr)

            for profile in profiles:
                self.assertEqual(
                    (paths.home / profile).read_text(encoding="utf-8"),
                    f"# sentinel {profile}\n",
                )
            self.assertEqual(config.read_bytes(), config_bytes)
            self.assertEqual(sorted(os.listdir(paths.codex_home)), ["config.toml"])
            self.assertEqual(
                (paths.home / ".env").read_text(encoding="utf-8"),
                "SECRET=sentinel\n",
            )

    def test_desktop_sets_env_and_owned_plist_then_rollback_restores(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))
            self.launchctl_set(paths, "CODEX_CLI_PATH", "/old/cli")

            result = self.run_takeover(root, "desktop", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)

            cli_link = paths.install_dir / "codex"
            self.assertEqual(self.launchctl_get(paths, "CODEX_CLI_PATH"), str(cli_link))
            plist = paths.home / "Library" / "LaunchAgents" / f"{PLIST_LABEL}.plist"
            self.assertTrue(plist.is_file())
            self.assertFalse(plist.is_symlink())
            self.assert_mode(plist, 0o600)
            contents = plist.read_text(encoding="utf-8")
            self.assertIn(f"<string>{PLIST_LABEL}</string>", contents)
            self.assertIn("<string>/bin/launchctl</string>", contents)
            self.assertIn("<string>setenv</string>", contents)
            self.assertIn("<string>CODEX_CLI_PATH</string>", contents)
            self.assertIn(f"<string>{cli_link}</string>", contents)
            self.assertIn("<key>RunAtLoad</key>\n  <true/>", contents)
            launchctl_log = self.launchctl_log(paths)
            self.assertIn(f"launchctl bootstrap gui/", launchctl_log)
            self.assertIn(str(plist), launchctl_log)
            receipt = paths.takeover_dir / "desktop.receipt"
            self.assertTrue(receipt.is_file())
            self.assert_mode(receipt, 0o600)
            self.assertIn("prior_env_kind=set", receipt.read_text(encoding="utf-8"))
            self.assertIn(
                "prior_env_value=/old/cli", receipt.read_text(encoding="utf-8")
            )

            result = self.run_takeover(root, "rollback", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(self.launchctl_get(paths, "CODEX_CLI_PATH"), "/old/cli")
            self.assertFalse(plist.exists())
            self.assertFalse(receipt.exists())
            for name in CLI_NAMES:
                self.assertFalse((paths.install_dir / name).exists())

    def test_desktop_prior_unset_rollback_unsets(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))

            result = self.run_takeover(root, "desktop", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt_text = (paths.takeover_dir / "desktop.receipt").read_text(
                encoding="utf-8"
            )
            self.assertIn("prior_env_kind=unset", receipt_text)
            self.assertNotIn("prior_env_value=", receipt_text)

            result = self.run_takeover(root, "rollback", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIsNone(self.launchctl_get(paths, "CODEX_CLI_PATH"))
            self.assertIn(
                "launchctl unsetenv CODEX_CLI_PATH",
                self.launchctl_log(paths),
            )

    def test_desktop_bootstrap_failure_is_explicit_and_recoverable(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))

            result = self.run_takeover(
                root,
                "desktop",
                reuse=paths,
                env_overrides={"LUMI_TEST_FAIL_BOOTSTRAP": "1"},
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("remain recoverable", result.stderr)
            self.assertTrue((paths.takeover_dir / "desktop.receipt").is_file())
            self.assertTrue(
                (
                    paths.home / "Library" / "LaunchAgents" / f"{PLIST_LABEL}.plist"
                ).is_file()
            )
            self.assertIsNone(self.launchctl_get(paths, "CODEX_CLI_PATH"))
            rollback = self.run_takeover(root, "rollback", reuse=paths)
            self.assertEqual(rollback.returncode, 0, rollback.stderr)

    def test_desktop_idempotent_preserves_original_prior_env(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))
            self.launchctl_set(paths, "CODEX_CLI_PATH", "/old/cli")

            self.assertEqual(
                self.run_takeover(root, "desktop", reuse=paths).returncode, 0
            )
            self.assertEqual(
                self.run_takeover(root, "desktop", reuse=paths).returncode, 0
            )
            receipt_text = (paths.takeover_dir / "desktop.receipt").read_text(
                encoding="utf-8"
            )
            self.assertIn("prior_env_value=/old/cli", receipt_text)

            result = self.run_takeover(root, "rollback", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(self.launchctl_get(paths, "CODEX_CLI_PATH"), "/old/cli")

    def test_desktop_refuses_foreign_plist(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))
            launch_agents = paths.home / "Library" / "LaunchAgents"
            launch_agents.mkdir(parents=True)
            plist = launch_agents / f"{PLIST_LABEL}.plist"
            foreign = (
                '<?xml version="1.0" encoding="UTF-8"?>\n'
                '<plist version="1.0"><dict>'
                "<key>Label</key><string>com.someone.else</string>"
                "</dict></plist>\n"
            )
            plist.write_text(foreign, encoding="utf-8")
            plist.chmod(0o644)

            result = self.run_takeover(root, "desktop", reuse=paths)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("foreign LaunchAgent plist", result.stderr)
            self.assertEqual(plist.read_text(encoding="utf-8"), foreign)
            # The read-only getenv probe is fine; setenv must never run.
            self.assertNotIn(" setenv ", self.launchctl_log(paths))

    def test_desktop_refuses_symlinked_plist(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))
            launch_agents = paths.home / "Library" / "LaunchAgents"
            launch_agents.mkdir(parents=True)
            target = root / "somewhere.plist"
            target.write_text("<plist/>\n", encoding="utf-8")
            (launch_agents / f"{PLIST_LABEL}.plist").symlink_to(target)

            result = self.run_takeover(root, "desktop", reuse=paths)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("foreign LaunchAgent plist", result.stderr)

    def test_desktop_refuses_competing_launch_agent_with_other_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))
            launch_agents = paths.home / "Library" / "LaunchAgents"
            launch_agents.mkdir(parents=True)
            competing = launch_agents / "com.example.codex-path.plist"
            competing.write_text(
                "<plist><array><string>setenv</string>"
                "<string>CODEX_CLI_PATH</string>"
                "<string>/another/codex</string></array></plist>\n",
                encoding="utf-8",
            )

            result = self.run_takeover(root, "desktop", reuse=paths)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Another LaunchAgent manages", result.stderr)
            self.assertFalse((paths.install_dir / "codex").exists())

    def test_desktop_accepts_competing_launch_agent_with_same_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))
            launch_agents = paths.home / "Library" / "LaunchAgents"
            launch_agents.mkdir(parents=True)
            competing = launch_agents / "com.cubelander.codex-cli-path.plist"
            competing.write_text(
                "<plist><array><string>setenv</string>"
                "<string>CODEX_CLI_PATH</string>"
                f"<string>{paths.install_dir / 'codex'}</string></array></plist>\n",
                encoding="utf-8",
            )

            result = self.run_takeover(root, "desktop", reuse=paths)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Compatible existing", result.stdout)

    def test_desktop_refused_on_linux(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Linux", "x86_64"))

            result = self.run_takeover(root, "desktop", reuse=paths)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("macOS-only", result.stderr)
            self.assertFalse((paths.install_dir / "codex").exists())
            self.assertEqual(self.launchctl_log(paths), "")

    def test_status_reports_clean_taken_over_and_drifted(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            paths = self.setup(root, platform=("Darwin", "arm64"))

            result = self.run_takeover(root, "status", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("codex: missing", result.stdout)
            self.assertIn("plist: missing", result.stdout)

            self.assertEqual(
                self.run_takeover(root, "desktop", reuse=paths).returncode, 0
            )
            result = self.run_takeover(root, "status", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("codex: ok", result.stdout)
            self.assertIn("codex-code-mode-host: ok", result.stdout)
            self.assertIn(
                f"CODEX_CLI_PATH: {paths.install_dir / 'codex'}", result.stdout
            )
            self.assertIn("plist: owned, points to", result.stdout)

            (paths.install_dir / "codex").unlink()
            (paths.install_dir / "codex").symlink_to("/elsewhere/codex")
            result = self.run_takeover(root, "status", reuse=paths)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("codex: DRIFTED", result.stdout)
            # Read-only status must not dump unrelated environment values.
            self.assertNotIn("SECRET", result.stdout)

    # -- helpers -----------------------------------------------------------

    def run_takeover(
        self,
        root: Path,
        *command: str,
        canonical: bool = True,
        platform: tuple[str, str] = ("Linux", "x86_64"),
        env_overrides: dict[str, str] | None = None,
        reuse: "TakeoverPaths | None" = None,
    ) -> "TakeoverResult":
        paths = reuse or self.setup(root, canonical=canonical, platform=platform)
        env = os.environ.copy()
        env.update(
            {
                "HOME": str(paths.home),
                "CODEX_HOME": str(paths.codex_home),
                "SHELL": "/bin/sh",
                "PATH": f"{paths.bin_dir}:/usr/bin:/bin",
                "LUMI_ROOT": str(paths.lumi_root),
                "LUMI_INSTALL_DIR": str(paths.install_dir),
                "XDG_STATE_HOME": str(paths.state_home),
                "XDG_DATA_HOME": str(paths.data_home),
                "LUMI_TEST_LAUNCHCTL_STATE": str(paths.launchctl_state),
                "LUMI_TEST_LAUNCHCTL_LOG": str(paths.launchctl_log_path),
                "SECRET": "sentinel-that-must-never-leak",
            }
        )
        env.pop("LUMI_STATE_DIR", None)
        if env_overrides:
            env.update(env_overrides)

        result = subprocess.run(
            ["/bin/sh", str(SCRIPT), *command],
            capture_output=True,
            check=False,
            env=env,
            text=True,
        )
        return TakeoverResult(result, paths)

    def setup(
        self,
        root: Path,
        *,
        canonical: bool = True,
        platform: tuple[str, str] = ("Linux", "x86_64"),
    ) -> "TakeoverPaths":
        paths = TakeoverPaths(root)
        paths.bin_dir.mkdir(parents=True, exist_ok=True)
        paths.home.mkdir(exist_ok=True)
        paths.codex_home.mkdir(exist_ok=True)

        os_name, arch = platform
        fake_uname = paths.bin_dir / "uname"
        fake_uname.write_text(
            "#!/bin/sh\n"
            'case "$1" in\n'
            f"  -s) printf '{os_name}\\n' ;;\n"
            f"  -m) printf '{arch}\\n' ;;\n"
            "esac\n",
            encoding="utf-8",
        )
        fake_uname.chmod(0o755)

        fake_launchctl = paths.bin_dir / "launchctl"
        fake_launchctl.write_text(FAKE_LAUNCHCTL, encoding="utf-8")
        fake_launchctl.chmod(0o755)

        if canonical:
            release_dir = paths.lumi_root / "releases" / "0.147.0-lumi.1"
            (release_dir / "bin").mkdir(parents=True, exist_ok=True)
            write_executable(
                release_dir / "bin" / "codex",
                "#!/bin/sh\nprintf 'codex-cli 0.147.0-lumi.1\\n'\n",
            )
            write_executable(
                release_dir / "bin" / "codex-code-mode-host",
                "#!/bin/sh\nexit 0\n",
            )
            current = paths.lumi_root / "current"
            if not current.is_symlink() and not current.exists():
                current.symlink_to(release_dir)
        return paths

    def launchctl_get(self, paths: "TakeoverPaths", key: str) -> str | None:
        if not paths.launchctl_state.is_file():
            return None
        for line in paths.launchctl_state.read_text(encoding="utf-8").splitlines():
            if line.startswith(f"{key}="):
                return line[len(key) + 1 :]
        return None

    def launchctl_set(self, paths: "TakeoverPaths", key: str, value: str) -> None:
        lines = []
        if paths.launchctl_state.is_file():
            lines = [
                line
                for line in paths.launchctl_state.read_text(
                    encoding="utf-8"
                ).splitlines()
                if not line.startswith(f"{key}=")
            ]
        lines.append(f"{key}={value}")
        paths.launchctl_state.write_text("\n".join(lines) + "\n", encoding="utf-8")

    def launchctl_log(self, paths: "TakeoverPaths") -> str:
        if not paths.launchctl_log_path.is_file():
            return ""
        return paths.launchctl_log_path.read_text(encoding="utf-8")

    def assert_mode(self, path: Path, mode: int) -> None:
        self.assertEqual(
            stat.S_IMODE(path.stat().st_mode),
            mode,
            f"{path} has mode {oct(stat.S_IMODE(path.stat().st_mode))}",
        )


class TakeoverPaths:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.bin_dir = root / "bin"
        self.home = root / "home"
        self.codex_home = root / "codex-home"
        self.lumi_root = root / "lumi-root"
        self.install_dir = root / "install-bin"
        self.state_home = root / "state"
        self.data_home = root / "data"
        self.state_root = self.state_home / "lumi-codex"
        self.takeover_dir = self.state_root / "takeover"
        self.launchctl_state = root / "launchctl.env"
        self.launchctl_log_path = root / "launchctl.log"


class TakeoverResult:
    def __init__(
        self, result: subprocess.CompletedProcess[str], paths: TakeoverPaths
    ) -> None:
        self._result = result
        self.paths = paths

    def __getattr__(self, name: str):
        return getattr(self._result, name)


def write_executable(path: Path, contents: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


def syntax_check(path: Path) -> None:
    result = subprocess.run(
        ["/bin/sh", "-n", str(path)],
        capture_output=True,
        check=False,
        text=True,
    )
    if result.returncode != 0:
        raise AssertionError(f"sh -n failed for {path.name}: {result.stderr}")


FAKE_LAUNCHCTL = textwrap.dedent(
    """\
    #!/bin/sh
    state="$LUMI_TEST_LAUNCHCTL_STATE"
    log="$LUMI_TEST_LAUNCHCTL_LOG"
    {
      printf 'launchctl'
      for arg in "$@"; do
        printf ' %s' "$arg"
      done
      printf '\\n'
    } >>"$log"
    cmd="$1"
    shift
    case "$cmd" in
      getenv)
        key="$1"
        [ -f "$state" ] || exit 0
        while IFS= read -r line || [ -n "$line" ]; do
          case "$line" in
            "$key="*)
              printf '%s\\n' "${line#"$key="}"
              exit 0
              ;;
          esac
        done <"$state"
        ;;
      setenv)
        key="$1"
        value="$2"
        : >"$state.tmp"
        if [ -f "$state" ]; then
          while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
              "$key="*) ;;
              *) printf '%s\\n' "$line" >>"$state.tmp" ;;
            esac
          done <"$state"
        fi
        printf '%s=%s\\n' "$key" "$value" >>"$state.tmp"
        mv "$state.tmp" "$state"
        ;;
      unsetenv)
        key="$1"
        : >"$state.tmp"
        if [ -f "$state" ]; then
          while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
              "$key="*) ;;
              *) printf '%s\\n' "$line" >>"$state.tmp" ;;
            esac
          done <"$state"
        fi
        mv "$state.tmp" "$state"
        ;;
      bootout)
        exit 0
        ;;
      bootstrap)
        [ "${LUMI_TEST_FAIL_BOOTSTRAP:-0}" != 1 ] || exit 70
        exit 0
        ;;
      *)
        exit 64
        ;;
    esac
    """
)


if __name__ == "__main__":
    unittest.main()
