#!/usr/bin/env python3

import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import textwrap
import unittest


INSTALL_SCRIPT = Path(__file__).with_name("install.sh")
POWERSHELL_SCRIPT = Path(__file__).with_name("install.ps1")
VERSION = "0.142.5"
MISMATCH_VERSION = "0.145.0"
LUMI_VERSION = "0.147.0-lumi.4"
TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)


class InstallShTest(unittest.TestCase):
    def test_installer_scripts_never_reference_openai_download_paths(self) -> None:
        for script in (INSTALL_SCRIPT, POWERSHELL_SCRIPT):
            with self.subTest(script=script.name):
                contents = script.read_text(encoding="utf-8")
                self.assertNotIn("releases.openai.com", contents)
                self.assertNotIn("openai/codex", contents)
                self.assertIn("Lumi-weaves/codex", contents)

    def test_install_sh_is_valid_shell(self) -> None:
        syntax_check(INSTALL_SCRIPT)

    def test_powershell_launcher_rejects_cmd_expansion_characters(self) -> None:
        contents = POWERSHELL_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('$Path.Contains("%")', contents)
        self.assertIn('$Path.Contains("!")', contents)

    def test_latest_failure_explains_that_prereleases_are_excluded(self) -> None:
        result, requests = run_installer("latest", metadata_failure=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            ["https://api.github.com/repos/Lumi-weaves/codex/releases/latest"],
        )
        self.assertIn("GitHub excludes prereleases", result.stderr)
        self.assertIn("--release x.y.z-lumi.N", result.stderr)

    def test_piped_bootstrap_install_works(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                piped=True,
                script_args=("--release", VERSION),
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"Lumi Codex CLI {VERSION} installed successfully.", result.stdout)
            launcher = root / "install-bin" / "lumi-codex"
            self.assertTrue(launcher.is_file())
            env = os.environ.copy()
            env["PATH"] = "/usr/bin:/bin"
            launched = subprocess.run(
                [str(launcher), "--version"],
                capture_output=True,
                check=False,
                env=env,
                text=True,
            )
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

    def test_metadata_fetch_failure_is_not_reported_as_missing_assets(self) -> None:
        result, requests = run_installer(VERSION, metadata_failure=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                f"rust-v{VERSION}"
            ],
        )
        self.assertIn(
            f"Could not fetch GitHub release metadata for Lumi Codex {VERSION}",
            result.stderr,
        )
        self.assertNotIn("Could not find Codex package", result.stderr)

    def test_exact_release_uses_github_metadata_once(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                    f"rust-v{VERSION}",
                    "https://github.com/Lumi-weaves/codex/releases/download/"
                    f"rust-v{VERSION}/codex-package_SHA256SUMS",
                    "https://github.com/Lumi-weaves/codex/releases/download/"
                    f"rust-v{VERSION}/codex-package-x86_64-unknown-linux-musl.tar.gz",
                ],
            )
            self.assertIn(f"Resolved version: {VERSION}", result.stdout)
            self.assertIn(f"Lumi Codex CLI {VERSION} installed successfully.", result.stdout)

    def test_lumi_semver_tags_are_accepted(self) -> None:
        for release in (f"rust-v{LUMI_VERSION}", LUMI_VERSION):
            with self.subTest(release=release):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    archive_path, checksum_path, metadata_json = create_package_release(
                        root, version=LUMI_VERSION
                    )

                    result, requests = run_installer_in(
                        root,
                        release,
                        metadata_json=metadata_json,
                        archive_path=archive_path,
                        checksum_path=checksum_path,
                    )

                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(
                        requests[0],
                        "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                        f"rust-v{LUMI_VERSION}",
                    )
                    self.assertIn(f"Resolved version: {LUMI_VERSION}", result.stdout)
                    release_dir = (
                        root / "lumi-root" / "releases" / f"{LUMI_VERSION}-x86_64-unknown-linux-musl"
                    )
                    self.assertTrue((release_dir / "bin" / "codex").is_file())

    def test_invalid_versions_are_rejected_before_any_request(self) -> None:
        for version in (
            "0.147.0-lumi",
            "0.147.0-lumi.",
            "0.147.0-lumi.x",
            "0.147",
            "0.147.0-lumi.1/../../etc/passwd",
            "..",
        ):
            with self.subTest(version=version):
                result, requests = run_installer(version)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(requests, [])
                self.assertIn("Invalid Codex release version", result.stderr)

    def test_latest_release_reuses_version_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://api.github.com/repos/Lumi-weaves/codex/releases/latest",
                    "https://github.com/Lumi-weaves/codex/releases/download/"
                    f"rust-v{VERSION}/codex-package_SHA256SUMS",
                    "https://github.com/Lumi-weaves/codex/releases/download/"
                    f"rust-v{VERSION}/codex-package-x86_64-unknown-linux-musl.tar.gz",
                ],
            )
            self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_compact_metadata_is_independent_of_field_order(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=recompact_metadata(metadata_json),
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(requests), 3)
            self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_json_like_strings_and_nested_fields_do_not_define_assets(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, _metadata = create_package_release(root)

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=release_metadata_with_decoys(archive_path, checksum_path),
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(requests), 3)
            self.assertIn(
                f"rust-v{VERSION}/codex-package-x86_64-unknown-linux-musl.tar.gz",
                requests[2],
            )

    def test_package_only_metadata_never_falls_back_to_legacy_npm(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, _metadata = create_package_release(root)
            decoy_metadata = release_metadata_with_decoys(archive_path, checksum_path)
            metadata = json.loads(decoy_metadata)
            # Keep only nested decoys and the checksum; no top-level package.
            metadata["assets"] = [
                asset
                for asset in metadata["assets"]
                if asset.get("name") == "codex-package_SHA256SUMS"
            ]

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=json.dumps(metadata),
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("/codex-npm-", requests[1] if len(requests) > 1 else "")
            self.assertIn("Could not find Codex package", result.stderr)

    def test_four_unix_targets_are_detected_and_installed(self) -> None:
        platforms = (
            ("Linux", "x86_64", "x86_64-unknown-linux-musl", "Linux (x64)"),
            ("Linux", "aarch64", "aarch64-unknown-linux-musl", "Linux (ARM64)"),
            ("Darwin", "x86_64", "x86_64-apple-darwin", "macOS (Intel)"),
            ("Darwin", "arm64", "aarch64-apple-darwin", "macOS (Apple Silicon)"),
        )
        for os_name, arch, target, label in platforms:
            with self.subTest(target=target):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    archive_path, checksum_path, metadata_json = create_package_release(
                        root, target=target
                    )

                    result, requests = run_installer_in(
                        root,
                        VERSION,
                        metadata_json=metadata_json,
                        archive_path=archive_path,
                        checksum_path=checksum_path,
                        platform=(os_name, arch),
                    )

                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn(
                        "https://github.com/Lumi-weaves/codex/releases/download/"
                        f"rust-v{VERSION}/codex-package-{target}.tar.gz",
                        requests,
                    )
                    self.assertIn(f"Detected platform: {label}", result.stdout)
                    release_dir = root / "lumi-root" / "releases" / f"{VERSION}-{target}"
                    self.assertTrue((release_dir / "bin" / "codex").is_file())
                    self.assertTrue((release_dir / "bin" / "codex-code-mode-host").is_file())
                    self.assertTrue((release_dir / "codex").is_symlink())
                    if target.endswith("linux-musl"):
                        self.assertTrue((release_dir / "codex-resources" / "bwrap").is_file())

    def test_explicit_target_override_is_validated(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(
                root, target="aarch64-apple-darwin"
            )

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                target_override="aarch64-apple-darwin",
                platform=("Linux", "x86_64"),
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(
                f"rust-v{VERSION}/codex-package-aarch64-apple-darwin.tar.gz",
                requests[2],
            )

    def test_unknown_or_unsafe_target_rejected_before_any_request(self) -> None:
        for target in ("../../evil", "x86_64-pc-windows-msvc", "codex-package-$(id).tar.gz"):
            with self.subTest(target=target):
                result, requests = run_installer(VERSION, target_override=target)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(requests, [])
                self.assertIn("Unsupported target", result.stderr)

    def test_side_by_side_install_preserves_official_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            home = root / "home"
            bin_dir = home / ".local" / "bin"
            bin_dir.mkdir(parents=True)
            official_codex = bin_dir / "codex"
            official_codex.write_text("#!/bin/sh\necho official codex\n", encoding="utf-8")
            official_codex.chmod(0o755)
            official_codex_bytes = official_codex.read_bytes()

            for profile in (".bashrc", ".zshrc", ".profile", ".bash_profile"):
                (home / profile).write_text(f"# sentinel {profile}\n", encoding="utf-8")
            codex_home = root / "official-codex-home"
            codex_home.mkdir()
            config = codex_home / "config.toml"
            config.write_text("model = \"official\"\n", encoding="utf-8")
            config_bytes = config.read_bytes()

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                home=home,
                codex_home=codex_home,
                use_default_paths=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            # Official codex binary and CODEX_HOME state are byte-identical.
            self.assertEqual(official_codex.read_bytes(), official_codex_bytes)
            self.assertEqual(config.read_bytes(), config_bytes)
            # No profile was modified and no PATH markers were inserted.
            for profile in (".bashrc", ".zshrc", ".profile", ".bash_profile"):
                self.assertEqual(
                    (home / profile).read_text(encoding="utf-8"),
                    f"# sentinel {profile}\n",
                )
            # Only the lumi-codex launcher appears in the shared bin dir.
            self.assertEqual(
                sorted(p.name for p in bin_dir.iterdir()),
                ["codex", "lumi-codex"],
            )
            launcher = bin_dir / "lumi-codex"
            self.assertTrue(launcher.is_file())
            self.assertTrue(os.access(launcher, os.X_OK))
            # The default Lumi root is used and stays out of CODEX_HOME.
            self.assertTrue((home / ".local" / "share" / "lumi-codex" / "current").is_symlink())
            self.assertFalse((codex_home / "packages").exists())

    def test_launcher_execs_the_real_current_codex(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            launcher = root / "install-bin" / "lumi-codex"
            lumi_root = root / "lumi-root"

            launcher_text = launcher.read_text(encoding="utf-8")
            self.assertIn(
                f"exec '{lumi_root}/current/bin/codex' \"$@\"",
                launcher_text,
            )

            env = os.environ.copy()
            env["PATH"] = "/usr/bin:/bin"
            launched = subprocess.run(
                [str(launcher), "--version"],
                capture_output=True,
                check=False,
                env=env,
                text=True,
            )
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

            launched = subprocess.run(
                [str(launcher)],
                capture_output=True,
                check=False,
                env=env,
                text=True,
            )
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

            # current/bin/codex is a real file and the launcher execs it, so
            # code-mode host and resources stay adjacent on macOS.
            self.assertTrue((lumi_root / "current" / "bin" / "codex").is_file())
            self.assertTrue(
                (lumi_root / "current" / "bin" / "codex-code-mode-host").is_file()
            )

    def test_macos_install_keeps_code_mode_host_beside_real_binary(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(
                root, target="aarch64-apple-darwin"
            )

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                platform=("Darwin", "arm64"),
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            install_bin = root / "install-bin"
            current = root / "lumi-root" / "current"
            self.assertTrue((current / "bin" / "codex-code-mode-host").is_file())
            self.assertTrue(os.access(current / "bin" / "codex-code-mode-host", os.X_OK))
            # No official-named visible commands are created.
            self.assertEqual(
                sorted(p.name for p in install_bin.iterdir()),
                ["lumi-codex"],
            )

    def test_transient_bad_checksum_digest_falls_back_to_github_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            bad_metadata = json.loads(metadata_json)
            for release_asset in bad_metadata["assets"]:
                if release_asset["name"] == "codex-package_SHA256SUMS":
                    release_asset["digest"] = "sha256:" + "0" * 64

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=json.dumps(bad_metadata),
                metadata_json2=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                    f"rust-v{VERSION}",
                    "https://github.com/Lumi-weaves/codex/releases/download/"
                    f"rust-v{VERSION}/codex-package_SHA256SUMS",
                    "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                    f"rust-v{VERSION}",
                    "https://github.com/Lumi-weaves/codex/releases/download/"
                    f"rust-v{VERSION}/codex-package-x86_64-unknown-linux-musl.tar.gz",
                ],
            )
            self.assertIn(
                "re-verifying against GitHub release metadata", result.stderr
            )
            self.assertIn(f"Lumi Codex CLI {VERSION} installed successfully.", result.stdout)

    def test_wrong_manifest_archive_digest_falls_back_to_github_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            # The checksum manifest claims a wrong digest for the archive; the
            # GitHub release-metadata digest for the archive is correct.
            checksum_path.write_text(
                f"{'0' * 64}  codex-package-x86_64-unknown-linux-musl.tar.gz\n",
                encoding="utf-8",
            )
            checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
            metadata = json.loads(metadata_json)
            for release_asset in metadata["assets"]:
                if release_asset["name"] == "codex-package_SHA256SUMS":
                    release_asset["digest"] = f"sha256:{checksum_digest}"

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=json.dumps(metadata),
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(requests), 4)
            self.assertEqual(requests[3], requests[0])
            self.assertIn(
                "re-verifying against GitHub release metadata", result.stderr
            )
            self.assertIn(f"Lumi Codex CLI {VERSION} installed successfully.", result.stdout)

    def test_corrupt_downloads_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                mode="corrupt_downloads",
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                requests,
                [
                    "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                    f"rust-v{VERSION}",
                    "https://github.com/Lumi-weaves/codex/releases/download/"
                    f"rust-v{VERSION}/codex-package_SHA256SUMS",
                    "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                    f"rust-v{VERSION}",
                ],
            )
            self.assertIn("checksum did not match expected digest", result.stderr)
            self.assertNotIn("installed successfully", result.stdout)
            self.assertFalse((root / "lumi-root" / "current").exists())

    def test_corrupt_metadata_digests_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            metadata = json.loads(metadata_json)
            for release_asset in metadata["assets"]:
                release_asset["digest"] = "sha256:" + "0" * 64

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=json.dumps(metadata),
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(len(requests), 3)
            self.assertIn("checksum did not match expected digest", result.stderr)
            self.assertFalse((root / "lumi-root" / "current").exists())

    def test_missing_checksum_manifest_entry_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            checksum_path.write_text(
                f"{'a' * 64}  codex-package-some-other-target.tar.gz\n",
                encoding="utf-8",
            )
            checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
            metadata = json.loads(metadata_json)
            for release_asset in metadata["assets"]:
                if release_asset["name"] == "codex-package_SHA256SUMS":
                    release_asset["digest"] = f"sha256:{checksum_digest}"

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=json.dumps(metadata),
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "Could not find SHA-256 digest for codex-package-"
                "x86_64-unknown-linux-musl.tar.gz in codex-package_SHA256SUMS",
                result.stderr,
            )

    def test_archive_traversal_and_absolute_members_rejected(self) -> None:
        for members in (
            ["../evil"],
            ["/etc/passwd"],
            ["bin/../../evil"],
        ):
            with self.subTest(members=members):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    archive_path, checksum_path, metadata_json = (
                        create_archive_release(root, members)
                    )

                    result, _requests = run_installer_in(
                        root,
                        VERSION,
                        metadata_json=metadata_json,
                        archive_path=archive_path,
                        checksum_path=checksum_path,
                    )

                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(
                        "unsafe member paths", result.stderr
                    )
                    self.assertFalse(
                        any(
                            (root / "lumi-root" / "releases").glob(
                                f"{VERSION}-*"
                            )
                        )
                    )

    def test_archive_symlink_and_hardlink_members_rejected(self) -> None:
        for entry_type in ("symlink", "hardlink"):
            with self.subTest(entry_type=entry_type):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    archive_path, checksum_path, metadata_json = (
                        create_archive_release(root, [], special=entry_type)
                    )

                    result, _requests = run_installer_in(
                        root,
                        VERSION,
                        metadata_json=metadata_json,
                        archive_path=archive_path,
                        checksum_path=checksum_path,
                    )

                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(
                        "unsafe entry types", result.stderr
                    )
                    self.assertFalse(
                        any(
                            (root / "lumi-root" / "releases").glob(
                                f"{VERSION}-*"
                            )
                        )
                    )

    def test_binary_version_mismatch_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(
                root, version=MISMATCH_VERSION
            )

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                f"did not report expected version {VERSION}",
                result.stderr,
            )
            self.assertNotIn("installed successfully", result.stdout)

    def test_incomplete_existing_release_is_reinstalled(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            release_dir = (
                root / "lumi-root" / "releases" / f"{VERSION}-x86_64-unknown-linux-musl"
            )
            release_dir.mkdir(parents=True)
            (release_dir / "stale-marker").write_text("incomplete", encoding="utf-8")

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("incomplete existing release", result.stderr)
            self.assertTrue((release_dir / "bin" / "codex").is_file())

    def test_repeat_install_reuses_verified_release(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            first_result, _ = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )
            self.assertEqual(first_result.returncode, 0, first_result.stderr)

            (root / "requests.log").unlink()
            second_result, second_requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(second_result.returncode, 0, second_result.stderr)
            self.assertEqual(
                second_requests,
                [
                    "https://api.github.com/repos/Lumi-weaves/codex/releases/tags/"
                    f"rust-v{VERSION}"
                ],
            )
            self.assertNotIn("Downloading Lumi Codex CLI", second_result.stdout)

    def test_launcher_conflict_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            install_bin = root / "install-bin"
            install_bin.mkdir()
            launcher = install_bin / "lumi-codex"
            launcher.write_text("#!/bin/sh\necho foreign\n", encoding="utf-8")
            launcher.chmod(0o755)

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing to overwrite unexpected file", result.stderr)
            self.assertEqual(
                launcher.read_text(encoding="utf-8"),
                "#!/bin/sh\necho foreign\n",
            )

    def test_symlinked_launcher_path_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            install_bin = root / "install-bin"
            install_bin.mkdir()
            launcher = install_bin / "lumi-codex"
            launcher.symlink_to("/usr/bin/true")

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing to replace non-regular file", result.stderr)
            self.assertTrue(launcher.is_symlink())

    def test_current_file_conflict_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            lumi_root = root / "lumi-root"
            lumi_root.mkdir(parents=True)
            (lumi_root / "current").write_text("not a symlink\n", encoding="utf-8")

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing to replace non-symlink at", result.stderr)
            self.assertEqual(
                (lumi_root / "current").read_text(encoding="utf-8"),
                "not a symlink\n",
            )

    def test_foreign_current_symlink_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            lumi_root = root / "lumi-root"
            lumi_root.mkdir(parents=True)
            (lumi_root / "current").symlink_to("/elsewhere/release")

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing to retarget foreign symlink", result.stderr)
            self.assertEqual(os.readlink(lumi_root / "current"), "/elsewhere/release")

    def test_symlinked_root_rejected_before_any_request(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            real_root = root / "real-root"
            real_root.mkdir()
            symlink_root = root / "symlinked-root"
            symlink_root.symlink_to(real_root)

            result, requests = run_installer_in(
                root,
                VERSION,
                lumi_root=symlink_root,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(requests, [])
            self.assertIn("Refusing to operate on symlinked root", result.stderr)

    def test_symlinked_install_dir_rejected_before_any_request(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            real_bin = root / "real-bin"
            real_bin.mkdir()
            symlink_bin = root / "symlinked-bin"
            symlink_bin.symlink_to(real_bin)

            result, requests = run_installer_in(
                root,
                VERSION,
                install_dir=symlink_bin,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(requests, [])
            self.assertIn("symlinked directory", result.stderr)

    def test_install_dir_not_on_path_reports_exact_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"Add {root / 'install-bin'} to your PATH", result.stdout)

    def test_install_dir_on_path_reports_lumi_codex_command(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            install_bin = root / "install-bin"
            install_bin.mkdir()

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                extra_path=install_bin,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Current terminal: lumi-codex", result.stdout)

    def test_metadata_version_is_validated_for_latest(self) -> None:
        for tag_name, message in (
            ("not-a-tag", "Failed to resolve the latest Lumi Codex release version"),
            ("rust-vinvalid", "Invalid Codex release version: invalid"),
        ):
            with self.subTest(tag_name=tag_name):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    metadata = {
                        "assets": [
                            {
                                "name": "codex-package-x86_64-unknown-linux-musl.tar.gz",
                                "digest": "sha256:" + "a" * 64,
                            }
                        ],
                        "tag_name": tag_name,
                    }

                    result, requests = run_installer_in(
                        root,
                        "latest",
                        metadata_json=json.dumps(metadata),
                    )

                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(
                        requests,
                        ["https://api.github.com/repos/Lumi-weaves/codex/releases/latest"],
                    )
                    self.assertIn(message, result.stderr)


def run_installer(
    release: str,
    *,
    metadata_failure: bool = False,
    metadata_json: str | None = None,
    metadata_json2: str | None = None,
    target_override: str | None = None,
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    with tempfile.TemporaryDirectory() as temp_dir:
        return run_installer_in(
            Path(temp_dir),
            release,
            metadata_failure=metadata_failure,
            metadata_json=metadata_json,
            metadata_json2=metadata_json2,
            target_override=target_override,
        )


def run_installer_in(
    root: Path,
    release: str,
    *,
    metadata_failure: bool = False,
    metadata_json: str | None = None,
    metadata_json2: str | None = None,
    archive_path: Path | None = None,
    checksum_path: Path | None = None,
    platform: tuple[str, str] = ("Linux", "x86_64"),
    target_override: str | None = None,
    mode: str = "",
    home: Path | None = None,
    codex_home: Path | None = None,
    lumi_root: Path | None = None,
    install_dir: Path | None = None,
    use_default_paths: bool = False,
    extra_path: Path | None = None,
    piped: bool = False,
    script_args: tuple[str, ...] = (),
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    bin_dir = root / "bin"
    bin_dir.mkdir(exist_ok=True)
    request_log = root / "requests.log"
    fake_curl = bin_dir / "curl"
    fake_curl.write_text(
        textwrap.dedent(
            """\
            #!/bin/sh
            url=""
            output=""
            previous=""
            for arg in "$@"; do
              case "$arg" in
                https://*) url="$arg" ;;
              esac
              if [ "$previous" = "-o" ]; then
                output="$arg"
              fi
              previous="$arg"
            done
            printf '%s\\n' "$url" >>"$LUMI_TEST_REQUEST_LOG"

            case "$url" in
              https://api.github.com/repos/Lumi-weaves/codex/*)
                if [ "$LUMI_TEST_METADATA_FAILURE" = "1" ]; then
                  echo "curl: (22) The requested URL returned error: 403" >&2
                  exit 22
                fi
                if [ -n "$LUMI_TEST_METADATA_JSON2" ] &&
                  [ "$(grep -c 'api.github.com' "$LUMI_TEST_REQUEST_LOG" 2>/dev/null || echo 0)" -ge 2 ]; then
                  printf '%s\\n' "$LUMI_TEST_METADATA_JSON2"
                else
                  printf '%s\\n' "$LUMI_TEST_METADATA_JSON"
                fi
                ;;
              https://github.com/Lumi-weaves/codex/releases/download/*/codex-package_SHA256SUMS)
                if [ "$LUMI_TEST_MODE" = "corrupt_downloads" ]; then
                  printf '<html>proxy error</html>\\n' >"$output"
                  exit 0
                fi
                if [ -n "$LUMI_TEST_CHECKSUM_PATH" ]; then
                  cp "$LUMI_TEST_CHECKSUM_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              https://github.com/Lumi-weaves/codex/releases/download/*/codex-package-*.tar.gz)
                if [ "$LUMI_TEST_MODE" = "corrupt_downloads" ]; then
                  printf '<html>proxy error</html>\\n' >"$output"
                  exit 0
                fi
                if [ -n "$LUMI_TEST_ARCHIVE_PATH" ]; then
                  cp "$LUMI_TEST_ARCHIVE_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              *)
                exit 22
                ;;
            esac
            """
        ),
        encoding="utf-8",
    )
    fake_curl.chmod(0o755)

    os_name, arch = platform
    fake_uname = bin_dir / "uname"
    fake_uname.write_text(
        "#!/bin/sh\n"
        'case "$1" in\n'
        f"  -s) printf '{os_name}\\n' ;;\n"
        f"  -m) printf '{arch}\\n' ;;\n"
        "esac\n",
        encoding="utf-8",
    )
    fake_uname.chmod(0o755)
    if os_name == "Darwin" and arch == "x86_64":
        fake_sysctl = bin_dir / "sysctl"
        fake_sysctl.write_text("#!/bin/sh\nprintf '0\\n'\n", encoding="utf-8")
        fake_sysctl.chmod(0o755)

    if home is None:
        home = root / "home"
    home.mkdir(exist_ok=True)
    if codex_home is None:
        codex_home = root / "codex-home"
    codex_home.mkdir(exist_ok=True)

    env = os.environ.copy()
    env.update(
        {
            "LUMI_RELEASE": release,
            "LUMI_TEST_ARCHIVE_PATH": str(archive_path or ""),
            "LUMI_TEST_CHECKSUM_PATH": str(checksum_path or ""),
            "LUMI_TEST_METADATA_FAILURE": "1" if metadata_failure else "0",
            "LUMI_TEST_METADATA_JSON": (
                metadata_json if metadata_json is not None else release_metadata()
            ),
            "LUMI_TEST_METADATA_JSON2": metadata_json2 or "",
            "LUMI_TEST_MODE": mode,
            "LUMI_TEST_REQUEST_LOG": str(request_log),
            "HOME": str(home),
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "SHELL": "/bin/sh",
            "CODEX_HOME": str(codex_home),
        }
    )
    if target_override is not None:
        env["LUMI_TARGET"] = target_override
    else:
        env.pop("LUMI_TARGET", None)
    if use_default_paths:
        env.pop("LUMI_INSTALL_DIR", None)
        env.pop("LUMI_ROOT", None)
        env.pop("XDG_DATA_HOME", None)
    else:
        env["LUMI_INSTALL_DIR"] = str(install_dir or root / "install-bin")
        env["LUMI_ROOT"] = str(lumi_root or root / "lumi-root")
        env.pop("XDG_DATA_HOME", None)
    if extra_path is not None:
        env["PATH"] = f"{bin_dir}:{extra_path}:/usr/bin:/bin"

    if piped:
        result = subprocess.run(
            ["/bin/sh", "-s", "--", *script_args],
            input=INSTALL_SCRIPT.read_text(encoding="utf-8"),
            capture_output=True,
            check=False,
            env=env,
            text=True,
        )
    else:
        result = subprocess.run(
            ["/bin/sh", str(INSTALL_SCRIPT)],
            capture_output=True,
            check=False,
            env=env,
            text=True,
        )
    requests = (
        request_log.read_text(encoding="utf-8").splitlines()
        if request_log.exists()
        else []
    )
    return result, requests


def create_package_release(
    root: Path,
    *,
    version: str = VERSION,
    metadata_version: str = VERSION,
    target: str = "x86_64-unknown-linux-musl",
) -> tuple[Path, Path, str]:
    package_dir = root / "package"
    (package_dir / "bin").mkdir(parents=True)
    (package_dir / "codex-path").mkdir()
    (package_dir / "codex-package.json").write_text("{}\n", encoding="utf-8")
    write_executable(
        package_dir / "bin" / "codex",
        f"#!/bin/sh\nprintf 'codex-cli {version}\\n'\n",
    )
    write_executable(
        package_dir / "bin" / "codex-code-mode-host",
        "#!/bin/sh\nexit 0\n",
    )
    write_executable(package_dir / "codex-path" / "rg", "#!/bin/sh\nexit 0\n")
    if target.endswith("linux-musl"):
        write_executable(
            package_dir / "codex-resources" / "bwrap", "#!/bin/sh\nexit 0\n"
        )

    asset = f"codex-package-{target}.tar.gz"
    archive_path = root / asset
    with tarfile.open(archive_path, "w:gz") as archive:
        for path in sorted(package_dir.rglob("*")):
            archive.add(path, arcname=path.relative_to(package_dir).as_posix())

    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path = root / "codex-package_SHA256SUMS"
    checksum_path.write_text(f"{archive_digest}  {asset}\n", encoding="utf-8")
    checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
    metadata_json = json.dumps(
        {
            "assets": [
                {
                    "name": f"codex-package-{other_target}.tar.gz",
                    "digest": f"sha256:{'a' * 64}",
                }
                for other_target in TARGETS
            ]
            + [
                {
                    "name": "codex-package_SHA256SUMS",
                    "digest": f"sha256:{checksum_digest}",
                }
            ],
            "tag_name": f"rust-v{metadata_version}",
        },
        indent=2,
    )
    # Replace the selected target's placeholder digest with the real one.
    metadata = json.loads(metadata_json)
    for release_asset in metadata["assets"]:
        if release_asset["name"] == asset:
            release_asset["digest"] = f"sha256:{archive_digest}"
    return archive_path, checksum_path, json.dumps(metadata, indent=2)


def create_archive_release(
    root: Path,
    unsafe_names: list[str],
    *,
    special: str = "",
) -> tuple[Path, Path, str]:
    """Build a release whose archive contains unsafe members."""
    asset = "codex-package-x86_64-unknown-linux-musl.tar.gz"
    archive_path = root / asset
    with tarfile.open(archive_path, "w:gz") as archive:
        for name in unsafe_names:
            info = tarfile.TarInfo(name)
            info.size = 4
            archive.addfile(info, io.BytesIO(b"evil"))
        if special == "symlink":
            info = tarfile.TarInfo("bin/codex")
            info.type = tarfile.SYMTYPE
            info.linkname = "/etc/passwd"
            archive.addfile(info)
        elif special == "hardlink":
            info = tarfile.TarInfo("bin/codex")
            info.type = tarfile.LNKTYPE
            info.linkname = "codex-path/rg"
            archive.addfile(info)

    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path = root / "codex-package_SHA256SUMS"
    checksum_path.write_text(f"{archive_digest}  {asset}\n", encoding="utf-8")
    checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
    metadata_json = json.dumps(
        {
            "assets": [
                {"name": asset, "digest": f"sha256:{archive_digest}"},
                {
                    "name": "codex-package_SHA256SUMS",
                    "digest": f"sha256:{checksum_digest}",
                },
            ],
            "tag_name": f"rust-v{VERSION}",
        },
        indent=2,
    )
    return archive_path, checksum_path, metadata_json


def write_executable(path: Path, contents: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


def release_metadata(*, compact: bool = False, reorder: bool = False) -> str:
    assets = [
        asset_metadata(
            f"codex-package-{target}.tar.gz",
            f"sha256:{'a' * 64}",
            reorder=reorder,
        )
        for target in TARGETS
    ]
    assets.append(
        asset_metadata(
            "codex-package_SHA256SUMS",
            f"sha256:{'b' * 64}",
            reorder=reorder,
        )
    )
    separators = (",", ":") if compact else None
    return json.dumps(
        {"assets": assets, "body": "braces: { } [ ]", "tag_name": f"rust-v{VERSION}"},
        indent=None if compact else 2,
        separators=separators,
    )


def asset_metadata(name: str, digest: str, *, reorder: bool) -> dict[str, str]:
    if reorder:
        return {"digest": digest, "name": name}
    return {"name": name, "digest": digest}


def recompact_metadata(metadata_json: str) -> str:
    """Re-serialize release metadata compactly with digest-first asset keys."""
    metadata = json.loads(metadata_json)
    metadata["assets"] = [
        {"digest": asset["digest"], "name": asset["name"]}
        for asset in metadata["assets"]
    ]
    return json.dumps(metadata, separators=(",", ":"))


def release_metadata_with_decoys(
    archive_path: Path, checksum_path: Path
) -> str:
    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
    fake_digest = f"sha256:{'0' * 64}"
    assets = [
        {
            "metadata": {
                "name": f"codex-package-{target}.tar.gz",
                "digest": fake_digest,
            },
            "digest": fake_digest,
            "name": f"codex-package-{target}.tar.gz",
        }
        for target in TARGETS
    ]
    for release_asset in assets:
        if release_asset["name"] == archive_path.name:
            release_asset["digest"] = f"sha256:{archive_digest}"
    assets.append(
        {
            "digest": f"sha256:{checksum_digest}",
            "name": "codex-package_SHA256SUMS",
        }
    )
    return json.dumps(
        {
            "body": (
                f'fake: {{"name":"codex-package_SHA256SUMS","digest":"{fake_digest}"}}'
            ),
            "assets": assets,
            "tag_name": f"rust-v{VERSION}",
        },
        separators=(",", ":"),
    )


def syntax_check(path: Path) -> None:
    result = subprocess.run(
        ["/bin/sh", "-n", str(path)],
        capture_output=True,
        check=False,
        text=True,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"sh -n failed for {path.name}: {result.stderr}"
        )


if __name__ == "__main__":
    unittest.main()
