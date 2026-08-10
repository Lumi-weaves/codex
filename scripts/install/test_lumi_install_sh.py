#!/usr/bin/env python3

import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tarfile
import tempfile
import textwrap
import time
import unittest


INSTALL_SCRIPT = Path(__file__).with_name("lumi-install.sh")
TARGET = "x86_64-unknown-linux-musl"
VERSION = "0.147.0"
OTHER_VERSION = "0.146.0"

METADATA_URL = (
    f"https://api.github.com/repos/Lumi-weaves/codex/releases/tags/rust-v{VERSION}"
)
LATEST_URL = "https://api.github.com/repos/Lumi-weaves/codex/releases/latest"
CHECKSUM_URL = (
    f"https://github.com/Lumi-weaves/codex/releases/download/rust-v{VERSION}/"
    "codex-package_SHA256SUMS"
)
ARCHIVE_URL = (
    f"https://github.com/Lumi-weaves/codex/releases/download/rust-v{VERSION}/"
    f"codex-package-{TARGET}.tar.gz"
)

FAKE_CURL_SH = textwrap.dedent(
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
        printf '%s\\n' "$LUMI_TEST_METADATA_JSON"
        ;;
      https://github.com/Lumi-weaves/codex/releases/download/*/codex-package_SHA256SUMS)
        if [ -n "$LUMI_TEST_CHECKSUM_PATH" ]; then
          cp "$LUMI_TEST_CHECKSUM_PATH" "$output"
        else
          exit 22
        fi
        ;;
      https://github.com/Lumi-weaves/codex/releases/download/*/codex-package-*.tar.gz)
        if [ -n "$LUMI_TEST_ARCHIVE_DELAY" ]; then
          sleep "$LUMI_TEST_ARCHIVE_DELAY"
        fi
        if [ "$LUMI_TEST_ARCHIVE_MODE" = "corrupt" ]; then
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
)


def lumi_root(root: Path) -> Path:
    return root / "xdg" / "lumi-codex"


def write_executable(path: Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


def official_codex_path(root: Path) -> Path:
    return root / "home" / ".local" / "bin" / "codex"


def create_official_codex(root: Path) -> Path:
    path = official_codex_path(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        return path
    write_executable(
        path,
        '#!/bin/sh\n'
        'printf "official codex executed\\n" >"$LUMI_TEST_OFFICIAL_MARKER"\n'
        "exit 0\n",
    )
    return path


def create_package_release(
    root: Path,
    *,
    version: str = VERSION,
    target: str = TARGET,
    incomplete: bool = False,
    missing_bwrap: bool = False,
) -> tuple[Path, Path, str]:
    package_dir = root / f"package-{version}"
    (package_dir / "bin").mkdir(parents=True)
    (package_dir / "codex-path").mkdir()
    (package_dir / "codex-resources").mkdir()
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
    if not missing_bwrap:
        write_executable(
            package_dir / "codex-resources" / "bwrap", "#!/bin/sh\nexit 0\n"
        )
    if incomplete:
        (package_dir / "bin" / "codex").unlink()

    asset = f"codex-package-{target}.tar.gz"
    archive_path = root / f"archive-{version}.tar.gz"
    with tarfile.open(archive_path, "w:gz") as archive:
        for path in package_dir.iterdir():
            archive.add(path, arcname=path.name)

    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path = root / f"SHA256SUMS-{version}"
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
            "tag_name": f"rust-v{version}",
        },
        indent=2,
    )
    return archive_path, checksum_path, metadata_json


def release_metadata_with_decoys(
    archive_path: Path, checksum_path: Path, version: str = VERSION
) -> str:
    fake_digest = f"sha256:{'0' * 64}"
    archive_digest = f"sha256:{hashlib.sha256(archive_path.read_bytes()).hexdigest()}"
    checksum_digest = f"sha256:{hashlib.sha256(checksum_path.read_bytes()).hexdigest()}"
    return json.dumps(
        {
            "body": (
                f'fake: {{"name":"codex-package_SHA256SUMS","digest":"{fake_digest}"}}'
            ),
            "assets": [
                {
                    "metadata": {
                        "name": f"codex-package-{TARGET}.tar.gz",
                        "digest": fake_digest,
                    },
                    "name": f"codex-package-{TARGET}.tar.gz",
                    "digest": archive_digest,
                },
                {
                    "metadata": {
                        "name": "codex-package_SHA256SUMS",
                        "digest": fake_digest,
                    },
                    "name": "codex-package_SHA256SUMS",
                    "digest": checksum_digest,
                },
            ],
            "tag_name": f"rust-v{version}",
        },
        separators=(",", ":"),
    )


def write_fake_curl(root: Path) -> Path:
    bin_dir = root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    fake_curl = bin_dir / "curl"
    if not fake_curl.exists():
        fake_curl.write_text(FAKE_CURL_SH, encoding="utf-8")
        fake_curl.chmod(0o755)
    return bin_dir


def make_env(
    root: Path,
    *,
    release: str = VERSION,
    metadata_json: str | None = None,
    checksum_path: Path | None = None,
    archive_path: Path | None = None,
    metadata_failure: bool = False,
    archive_delay: str | None = None,
    archive_mode: str = "",
    extra: dict[str, str] | None = None,
) -> dict[str, str]:
    home = root / "home"
    home.mkdir(parents=True, exist_ok=True)
    (root / "xdg").mkdir(parents=True, exist_ok=True)
    bin_dir = write_fake_curl(root)
    create_official_codex(root)
    official_bin = official_codex_path(root).parent

    env = os.environ.copy()
    env.update(
        {
            "HOME": str(home),
            "XDG_DATA_HOME": str(root / "xdg"),
            "PATH": f"{bin_dir}:{official_bin}:/usr/bin:/bin",
            "SHELL": "/bin/bash",
            "LUMI_RELEASE": release,
            "LUMI_TEST_REQUEST_LOG": str(root / "requests.log"),
            "LUMI_TEST_METADATA_JSON": (
                metadata_json if metadata_json is not None else release_metadata()
            ),
            "LUMI_TEST_CHECKSUM_PATH": str(checksum_path or ""),
            "LUMI_TEST_ARCHIVE_PATH": str(archive_path or ""),
            "LUMI_TEST_METADATA_FAILURE": "1" if metadata_failure else "0",
            "LUMI_TEST_ARCHIVE_DELAY": archive_delay or "",
            "LUMI_TEST_ARCHIVE_MODE": archive_mode,
            "LUMI_TEST_OFFICIAL_MARKER": str(root / "official-marker"),
            "CODEX_HOME": str(root / "codex-home"),
        }
    )
    if extra:
        env.update(extra)
    return env


def read_requests(root: Path) -> list[str]:
    request_log = root / "requests.log"
    if not request_log.exists():
        return []
    return request_log.read_text(encoding="utf-8").splitlines()


def run_manage(
    root: Path,
    args: list[str],
    *,
    script: Path = INSTALL_SCRIPT,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    if env is None:
        env = make_env(root)
    return subprocess.run(
        ["/bin/sh", str(script), "manage", *args],
        capture_output=True,
        check=False,
        env=env,
        text=True,
    )


def run_shim(
    root: Path,
    args: list[str],
    *,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    if env is None:
        env = make_env(root)
    return subprocess.run(
        [str(lumi_root(root) / "shim" / "lumi-codex"), *args],
        capture_output=True,
        check=False,
        env=env,
        text=True,
    )


def release_metadata() -> str:
    assets = [
        {
            "name": f"codex-package-{target}.tar.gz",
            "digest": f"sha256:{'a' * 64}",
        }
        for target in (
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-musl",
        )
    ]
    assets.append(
        {"name": "codex-package_SHA256SUMS", "digest": f"sha256:{'b' * 64}"}
    )
    return json.dumps(
        {"assets": assets, "body": "braces: { } [ ]", "tag_name": f"rust-v{VERSION}"},
        indent=2,
    )


def receipt_keys(root: Path) -> dict[str, str]:
    text = (lumi_root(root) / "receipts" / "current.receipt").read_text(
        encoding="utf-8"
    )
    result: dict[str, str] = {}
    for line in text.splitlines()[1:]:
        if "=" in line:
            key, value = line.split("=", 1)
            result[key] = value
    return result


class LumiInstallShTest(unittest.TestCase):
    def test_happy_install_creates_managed_root_and_activates(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)

            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)

            lr = lumi_root(root)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertTrue(
                (lr / "releases" / f"{VERSION}-{TARGET}" / "bin" / "codex").exists()
            )
            self.assertEqual(
                os.readlink(lr / "shim" / "codex"), "../current/bin/codex"
            )
            self.assertEqual(
                os.readlink(lr / "shim" / "lumi-codex"),
                "../manager/lumi-install.sh",
            )
            self.assertTrue((lr / "manager" / "lumi-install.sh").is_file())
            self.assertTrue(os.access(lr / "manager" / "lumi-install.sh", os.X_OK))

            current_receipt = (lr / "receipts" / "current.receipt").read_text(
                encoding="utf-8"
            )
            self.assertTrue(
                current_receipt.startswith("LUMI-CODEX-RECEIPT-V1\n"),
                current_receipt,
            )
            keys = receipt_keys(root)
            self.assertEqual(
                set(keys),
                {
                    "schema",
                    "root",
                    "tag",
                    "version",
                    "target",
                    "archive",
                    "archive_sha256",
                    "bin_sha256",
                    "release_dir",
                    "current",
                    "previous",
                    "activated",
                    "profile",
                    "manager",
                    "launcher",
                    "shim",
                    "shim_dir",
                    "releases_dir",
                    "receipts_dir",
                    "tmp_dir",
                },
            )
            self.assertEqual(keys["tag"], f"rust-v{VERSION}")
            self.assertEqual(keys["version"], VERSION)
            self.assertEqual(keys["target"], TARGET)
            self.assertEqual(keys["current"], f"{VERSION}-{TARGET}")
            self.assertEqual(keys["previous"], "-")
            self.assertEqual(keys["activated"], "yes")
            self.assertEqual(keys["profile"], str(root / "home" / ".bashrc"))
            self.assertEqual(len(keys["archive_sha256"]), 64)
            self.assertEqual(len(keys["bin_sha256"]), 64)

            release_receipt = (
                lr / "receipts" / f"{VERSION}-{TARGET}.receipt"
            ).read_text(encoding="utf-8")
            self.assertTrue(
                release_receipt.startswith("LUMI-CODEX-RECEIPT-V1\n"),
                release_receipt,
            )

            bashrc = (root / "home" / ".bashrc").read_text(encoding="utf-8")
            self.assertIn("# >>> Lumi Codex managed PATH >>>", bashrc)
            self.assertIn("# <<< Lumi Codex managed PATH <<<", bashrc)
            self.assertIn(f'export PATH="{lr}/shim:$PATH"', bashrc)

    def test_happy_install_requests_fork_release_urls_only(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)

            result, requests = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [METADATA_URL, CHECKSUM_URL, ARCHIVE_URL],
            )
            for request in requests:
                self.assertIn("Lumi-weaves/codex", request)
            self.assertNotIn("openai/codex", requests)

    def test_install_preserves_official_codex_byte_for_byte(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            official = official_codex_path(root)
            official.parent.mkdir(parents=True, exist_ok=True)
            official_bytes = os.urandom(4096)
            official.write_bytes(official_bytes)
            official.chmod(0o755)
            before = hashlib.sha256(official_bytes).hexdigest()

            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(hashlib.sha256(official.read_bytes()).hexdigest(), before)
            self.assertFalse((root / "official-marker").exists())

            uninstall = run_manage(root, ["uninstall"])
            self.assertEqual(uninstall.returncode, 0, uninstall.stderr)
            self.assertEqual(hashlib.sha256(official.read_bytes()).hexdigest(), before)

    def test_install_no_activate_then_activate_deactivate_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
            )
            bashrc = root / "home" / ".bashrc"

            result = run_manage(root, ["install", "--no-activate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(bashrc.exists())
            self.assertEqual(receipt_keys(root)["activated"], "no")
            self.assertEqual(receipt_keys(root)["profile"], "-")

            result = run_manage(root, ["activate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("# >>> Lumi Codex managed PATH >>>", bashrc.read_text())
            self.assertEqual(receipt_keys(root)["activated"], "yes")
            self.assertEqual(receipt_keys(root)["profile"], str(bashrc))

            result = run_manage(root, ["activate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(bashrc.read_text().count("# >>> Lumi Codex managed PATH >>>"), 1)

            result = run_manage(root, ["deactivate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("Lumi Codex managed PATH", bashrc.read_text())
            self.assertEqual(receipt_keys(root)["activated"], "no")
            self.assertEqual(receipt_keys(root)["profile"], "-")

            result = run_manage(root, ["deactivate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("nothing to remove", result.stdout)

            result = run_manage(root, ["activate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("# >>> Lumi Codex managed PATH >>>", bashrc.read_text())

    def test_activate_deactivate_fail_closed_on_drifted_block(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
            )
            result = run_manage(root, ["install"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            bashrc = root / "home" / ".bashrc"
            original = bashrc.read_text()
            drifted = original.replace(
                f'export PATH="{lumi_root(root)}/shim:$PATH"',
                'export PATH="/tmp/somewhere-else:$PATH"',
            )
            bashrc.write_text(drifted, encoding="utf-8")

            result = run_manage(root, ["activate"], env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(bashrc.read_text(), drifted)

            result = run_manage(root, ["deactivate"], env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(bashrc.read_text(), drifted)

            result = run_manage(root, ["doctor"], env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("PROBLEMS FOUND", result.stdout)

    def test_repeat_install_reuses_complete_release(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)

            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            (root / "requests.log").unlink()

            result, requests = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(requests, [METADATA_URL])
            self.assertIn("already installed and complete; reusing it", result.stdout)
            releases = list((lumi_root(root) / "releases").iterdir())
            self.assertEqual(len(releases), 1)

    def test_upgrade_switches_current_and_tracks_previous(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )

            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)

            lr = lumi_root(root)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "previous"), f"releases/{VERSION}-{TARGET}"
            )
            keys = receipt_keys(root)
            self.assertEqual(keys["current"], f"{OTHER_VERSION}-{TARGET}")
            self.assertEqual(keys["previous"], f"{VERSION}-{TARGET}")

    def test_checksum_bad_install_fails_without_switching(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            before = receipt_keys(root)

            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )
            result, _ = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
                archive_mode="corrupt",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("checksum", result.stderr)

            lr = lumi_root(root)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertFalse(
                (lr / "releases" / f"{OTHER_VERSION}-{TARGET}").exists()
            )
            self.assertEqual(receipt_keys(root), before)

    def test_incomplete_package_install_fails_without_switching(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)

            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION, incomplete=True
            )
            result, _ = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("completeness", result.stderr)

            lr = lumi_root(root)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertFalse(
                (lr / "releases" / f"{OTHER_VERSION}-{TARGET}").exists()
            )
            self.assertEqual(receipt_keys(root)["current"], f"{VERSION}-{TARGET}")

    def test_metadata_fetch_failure_fails_cleanly(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
                metadata_failure=True,
            )
            result, requests = run_installer_with_env(root, env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("release metadata", result.stderr)
            self.assertEqual(requests, [METADATA_URL])
            lr = lumi_root(root)
            self.assertFalse((lr / "releases").exists())
            self.assertFalse((lr / "receipts").exists())
            self.assertFalse((lr / "current").exists())

            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_list_reports_current_previous_and_activation(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )
            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)

            result = run_manage(root, ["list"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"current: {OTHER_VERSION}-{TARGET}", result.stdout)
            self.assertIn(f"previous: {VERSION}-{TARGET}", result.stdout)
            self.assertIn(f"  - {VERSION}-{TARGET}", result.stdout)
            self.assertIn(f"  - {OTHER_VERSION}-{TARGET}", result.stdout)
            self.assertIn("activation: enabled", result.stdout)

    def test_rollback_swaps_both_directions(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )
            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)

            lr = lumi_root(root)
            result = run_manage(root, ["rollback"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Nothing to roll back to", result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )

            second = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)

            result = run_manage(root, ["rollback"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "previous"), f"releases/{OTHER_VERSION}-{TARGET}"
            )
            keys = receipt_keys(root)
            self.assertEqual(keys["current"], f"{VERSION}-{TARGET}")
            self.assertEqual(keys["previous"], f"{OTHER_VERSION}-{TARGET}")

            result = run_manage(root, ["rollback"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "previous"), f"releases/{VERSION}-{TARGET}"
            )

            result = run_manage(root, ["doctor"])
            self.assertEqual(result.returncode, 0, result.stdout)

    def test_rollback_refuses_when_previous_receipt_missing(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )
            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)
            lr = lumi_root(root)
            (lr / "receipts" / f"{VERSION}-{TARGET}.receipt").unlink()

            result = run_manage(root, ["rollback"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing rollback", result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "previous"), f"releases/{VERSION}-{TARGET}"
            )

    def test_activate_fails_closed_on_unknown_shim_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)
            shim_codex = lr / "shim" / "codex"
            shim_codex.unlink()
            shim_codex.symlink_to("/usr/bin/codex")

            result = run_manage(root, ["deactivate"])
            self.assertEqual(result.returncode, 0, result.stderr)
            result = run_manage(root, ["activate"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unknown symlink", result.stderr)

    def test_doctor_clean_reports_official_codex_without_executing(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)

            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)
            self.assertIn("doctor: OK", doctor.stdout)
            self.assertIn(str(official_codex_path(root)), doctor.stdout)
            self.assertIn("official codex (outside shim)", doctor.stdout)
            self.assertFalse((root / "official-marker").exists())

    def test_doctor_not_installed_reports_and_exits_zero(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            env = make_env(root)
            result = run_manage(root, ["doctor"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("not installed", result.stdout)

    def test_doctor_detects_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            first = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            lr = lumi_root(root)

            # baseline clean
            clean = run_manage(root, ["doctor"])
            self.assertEqual(clean.returncode, 0, clean.stdout)

            # tampered bin/codex content (hash mismatch)
            codex = lr / "releases" / f"{VERSION}-{TARGET}" / "bin" / "codex"
            codex.write_text("#!/bin/sh\nexit 7\n", encoding="utf-8")
            codex.chmod(0o755)
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("PROBLEMS FOUND", drifted.stdout)
            self.assertIn("bin/codex sha256", drifted.stdout)

    def test_doctor_detects_corrupt_receipt_and_unknown_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            first = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            lr = lumi_root(root)

            receipt = lr / "receipts" / "current.receipt"
            receipt.write_text(
                receipt.read_text(encoding="utf-8") + "bogus_extra=1\n",
                encoding="utf-8",
            )
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("current.receipt", drifted.stdout)
            receipt.write_text(
                "\n".join(
                    line
                    for line in receipt.read_text(encoding="utf-8").splitlines()
                    if not line.startswith("bogus_extra=")
                )
                + "\n",
                encoding="utf-8",
            )

            shim_codex = lr / "shim" / "codex"
            shim_codex.unlink()
            shim_codex.symlink_to("/usr/bin/codex")
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("unknown symlink", drifted.stdout)

    def test_doctor_detects_missing_previous_receipt_and_profile_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )
            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)
            lr = lumi_root(root)

            (lr / "receipts" / f"{VERSION}-{TARGET}.receipt").unlink()
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("previous release", drifted.stdout)

            # restore, then remove the activated profile block
            run_installer_with(root, first_archive, first_checksum, first_metadata)
            bashrc = root / "home" / ".bashrc"
            lines = [
                line
                for line in bashrc.read_text(encoding="utf-8").splitlines()
                if "Lumi Codex managed PATH" not in line
                and "lumi-codex/shim" not in line
            ]
            bashrc.write_text("\n".join(lines) + "\n", encoding="utf-8")
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("activated", drifted.stdout)

    def test_uninstall_removes_owned_paths_only(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            official = official_codex_path(root)
            official.parent.mkdir(parents=True, exist_ok=True)
            official_bytes = os.urandom(2048)
            official.write_bytes(official_bytes)
            official.chmod(0o755)
            official_hash = hashlib.sha256(official_bytes).hexdigest()
            codex_home = root / "codex-home"
            codex_home.mkdir(parents=True, exist_ok=True)
            (codex_home / "config.toml").write_text("keep me\n", encoding="utf-8")

            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
            )
            result = run_manage(root, ["install"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(bashrc_has_block(root))

            uninstall = run_manage(root, ["uninstall"], env=env)
            self.assertEqual(uninstall.returncode, 0, uninstall.stderr)
            self.assertFalse(lumi_root(root).exists())
            self.assertFalse(bashrc_has_block(root))
            self.assertEqual(
                hashlib.sha256(official.read_bytes()).hexdigest(), official_hash
            )
            self.assertEqual(
                (codex_home / "config.toml").read_text(encoding="utf-8"), "keep me\n"
            )
            self.assertTrue((root / "xdg").exists())

    def test_uninstall_refuses_unknown_shim_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)

            shim_codex = lr / "shim" / "codex"
            shim_codex.unlink()
            shim_codex.symlink_to("/usr/bin/codex")
            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("Refusing uninstall", uninstall.stderr)
            self.assertTrue((lr / "releases").exists())
            self.assertTrue((lr / "receipts" / "current.receipt").exists())

    def test_uninstall_refuses_without_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)
            (lr / "receipts" / "current.receipt").unlink()

            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("Refusing uninstall", uninstall.stderr)
            self.assertTrue((lr / "releases").exists())

    def test_unknown_target_refused_without_state_change(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
            )
            result = run_manage(
                root, ["install", "--target", "aarch64-unknown-linux-musl"], env=env
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("no package asset", result.stderr)
            lr = lumi_root(root)
            self.assertFalse((lr / "releases").exists())
            self.assertFalse((lr / "receipts").exists())
            self.assertFalse((lr / "current").exists())

            result = run_manage(root, ["install", "--target", "bogus-target"], env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("no package asset", result.stderr)

    def test_launcher_execs_current_codex_and_intercepts_manage(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)

            launched = run_shim(root, ["--version"])
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

            launched = run_shim(root, [])
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

            managed = run_shim(root, ["manage", "list"])
            self.assertEqual(managed.returncode, 0, managed.stderr)
            self.assertIn(f"current: {VERSION}-{TARGET}", managed.stdout)

            bogus = run_shim(root, ["manage", "bogus"])
            self.assertNotEqual(bogus.returncode, 0)
            self.assertIn("Unknown Lumi Codex action", bogus.stderr)

    def test_metadata_decoys_do_not_confuse_asset_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, _metadata = create_package_release(root)
            env = make_env(
                root,
                metadata_json=release_metadata_with_decoys(archive, checksum),
                checksum_path=checksum,
                archive_path=archive,
            )
            result, requests = run_installer_with_env(root, env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(requests, [METADATA_URL, CHECKSUM_URL, ARCHIVE_URL])

    def test_concurrent_installs_serialize_to_consistent_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )
            env_a = make_env(
                root,
                release=VERSION,
                metadata_json=first_metadata,
                checksum_path=first_checksum,
                archive_path=first_archive,
                archive_delay="3",
            )
            env_b = make_env(
                root,
                release=OTHER_VERSION,
                metadata_json=second_metadata,
                checksum_path=second_checksum,
                archive_path=second_archive,
                archive_delay="3",
            )

            proc_a = subprocess.Popen(
                ["/bin/sh", str(INSTALL_SCRIPT), "manage", "install"],
                env=env_a,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            proc_b = subprocess.Popen(
                ["/bin/sh", str(INSTALL_SCRIPT), "manage", "install"],
                env=env_b,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            out_a, err_a = proc_a.communicate(timeout=60)
            out_b, err_b = proc_b.communicate(timeout=60)
            self.assertEqual(proc_a.returncode, 0, out_a + err_a)
            self.assertEqual(proc_b.returncode, 0, out_b + err_b)

            lr = lumi_root(root)
            releases = sorted(p.name for p in (lr / "releases").iterdir())
            self.assertEqual(
                releases,
                sorted([f"{VERSION}-{TARGET}", f"{OTHER_VERSION}-{TARGET}"]),
            )
            keys = receipt_keys(root)
            self.assertIn(keys["current"], releases)
            self.assertIn(keys["previous"], releases)
            self.assertNotEqual(keys["current"], keys["previous"])

            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)

    def test_interruption_during_download_leaves_no_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
                archive_delay="30",
            )
            proc = subprocess.Popen(
                ["/bin/sh", str(INSTALL_SCRIPT), "manage", "install"],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
            )
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if ARCHIVE_URL in read_requests(root):
                    break
                time.sleep(0.05)
            self.assertIn(ARCHIVE_URL, read_requests(root))
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            proc.communicate(timeout=10)

            lr = lumi_root(root)
            self.assertFalse((lr / "releases").exists())
            self.assertFalse((lr / "receipts").exists())
            self.assertFalse((lr / "current").exists())
            self.assertFalse(bashrc_has_block(root))

            result, _ = run_installer_with(root, archive, checksum, metadata)
            self.assertEqual(result.returncode, 0, result.stderr)
            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)

    def test_interruption_during_upgrade_keeps_previous_current(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata = create_package_release(
                root, version=VERSION
            )
            second_archive, second_checksum, second_metadata = create_package_release(
                root, version=OTHER_VERSION
            )
            first = run_installer_with(
                root, first_archive, first_checksum, first_metadata
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            bashrc_before = (
                root / "home" / ".bashrc"
            ).read_text(encoding="utf-8")
            keys_before = receipt_keys(root)

            env = make_env(
                root,
                release=OTHER_VERSION,
                metadata_json=second_metadata,
                checksum_path=second_checksum,
                archive_path=second_archive,
                archive_delay="30",
            )
            proc = subprocess.Popen(
                ["/bin/sh", str(INSTALL_SCRIPT), "manage", "install"],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
            )
            deadline = time.monotonic() + 15
            other_archive_url = (
                f"https://github.com/Lumi-weaves/codex/releases/download/"
                f"rust-v{OTHER_VERSION}/codex-package-{TARGET}.tar.gz"
            )
            while time.monotonic() < deadline:
                if other_archive_url in read_requests(root):
                    break
                time.sleep(0.05)
            self.assertIn(other_archive_url, read_requests(root))
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            proc.communicate(timeout=10)

            lr = lumi_root(root)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertEqual(receipt_keys(root), keys_before)
            self.assertEqual(
                (root / "home" / ".bashrc").read_text(encoding="utf-8"),
                bashrc_before,
            )

            second = run_installer_with(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )


def bashrc_has_block(root: Path) -> bool:
    bashrc = root / "home" / ".bashrc"
    if not bashrc.exists():
        return False
    return "# >>> Lumi Codex managed PATH >>>" in bashrc.read_text(encoding="utf-8")


def run_installer_with(
    root: Path,
    archive: Path,
    checksum: Path,
    metadata: str,
    *,
    release: str = VERSION,
    archive_mode: str = "",
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    env = make_env(
        root,
        release=release,
        metadata_json=metadata,
        checksum_path=checksum,
        archive_path=archive,
        archive_mode=archive_mode,
    )
    return run_installer_with_env(root, env)


def run_installer_with_env(
    root: Path, env: dict[str, str]
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    result = run_manage(root, ["install"], env=env)
    return result, read_requests(root)


if __name__ == "__main__":
    unittest.main()
