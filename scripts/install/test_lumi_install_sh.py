#!/usr/bin/env python3

import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tarfile
import tempfile
import textwrap
import time
import unittest


INSTALL_SCRIPT = Path(__file__).with_name("lumi-install.sh")
TARGET = "x86_64-unknown-linux-musl"
AARCH64_TARGET = "aarch64-unknown-linux-musl"
VERSION = "0.147.0"
OTHER_VERSION = "0.146.0"
LUMI_VERSION = "0.147.0-lumi.1"

METADATA_URL = (
    f"https://api.github.com/repos/Lumi-weaves/codex/releases/tags/rust-v{VERSION}"
)
CHECKSUM_URL = (
    f"https://github.com/Lumi-weaves/codex/releases/download/rust-v{VERSION}/"
    "codex-package_SHA256SUMS"
)
ARCHIVE_URL = (
    f"https://github.com/Lumi-weaves/codex/releases/download/rust-v{VERSION}/"
    f"codex-package-{TARGET}.tar.gz"
)
MANAGER_URL = (
    f"https://github.com/Lumi-weaves/codex/releases/download/rust-v{VERSION}/"
    "lumi-install.sh"
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
      https://github.com/Lumi-weaves/codex/releases/download/*/lumi-install.sh)
        if [ -n "$LUMI_TEST_MANAGER_PATH" ]; then
          cp "$LUMI_TEST_MANAGER_PATH" "$output"
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


def canonical_package_metadata(version: str, target: str) -> str:
    return json.dumps(
        {
            "distribution": "lumi",
            "variant": "codex",
            "entrypoint": "bin/codex",
            "version": version,
            "target": target,
        }
    ) + "\n"


def package_files(package_dir: Path, version: str) -> None:
    (package_dir / "bin").mkdir(parents=True, exist_ok=True)
    (package_dir / "codex-path").mkdir(exist_ok=True)
    (package_dir / "codex-resources").mkdir(exist_ok=True)
    write_executable(
        package_dir / "bin" / "codex",
        f"#!/bin/sh\nprintf 'codex-cli {version}\\n'\n",
    )
    write_executable(
        package_dir / "bin" / "codex-code-mode-host", "#!/bin/sh\nexit 0\n"
    )
    write_executable(package_dir / "codex-path" / "rg", "#!/bin/sh\nexit 0\n")
    write_executable(
        package_dir / "codex-resources" / "bwrap", "#!/bin/sh\nexit 0\n"
    )


def create_package_release(
    root: Path,
    *,
    version: str = VERSION,
    target: str = TARGET,
    incomplete: bool = False,
    metadata_override: dict[str, str] | None = None,
    symlink_member: str | None = None,
    traversal_member: str | None = None,
    absolute_member: str | None = None,
) -> tuple[Path, Path, str, Path]:
    package_dir = root / f"package-{version}-{target.replace('/', '_')}"
    package_files(package_dir, version)
    metadata = dict(
        distribution="lumi",
        variant="codex",
        entrypoint="bin/codex",
        version=version,
        target=target,
    )
    if metadata_override:
        metadata.update(metadata_override)
    (package_dir / "codex-package.json").write_text(
        json.dumps(metadata) + "\n", encoding="utf-8"
    )
    if incomplete:
        (package_dir / "bin" / "codex").unlink()

    asset = f"codex-package-{target}.tar.gz"
    archive_path = root / f"archive-{version}-{target}.tar.gz"
    with tarfile.open(archive_path, "w:gz") as archive:
        for path in sorted(package_dir.iterdir()):
            archive.add(path, arcname=path.name)
        if symlink_member:
            info = tarfile.TarInfo(symlink_member)
            info.type = tarfile.SYMTYPE
            info.linkname = "/etc/passwd"
            archive.addfile(info)
        if traversal_member:
            data = b"evil"
            info = tarfile.TarInfo(traversal_member)
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
        if absolute_member:
            data = b"evil"
            info = tarfile.TarInfo(absolute_member)
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))

    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path = root / f"SHA256SUMS-{version}-{target}"
    checksum_path.write_text(f"{archive_digest}  {asset}\n", encoding="utf-8")
    checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
    manager_path = root / f"lumi-install-{version}-{target}.sh"
    manager_path.write_bytes(INSTALL_SCRIPT.read_bytes())
    manager_digest = hashlib.sha256(manager_path.read_bytes()).hexdigest()
    metadata_json = json.dumps(
        {
            "assets": [
                {"name": asset, "digest": f"sha256:{archive_digest}"},
                {
                    "name": "codex-package_SHA256SUMS",
                    "digest": f"sha256:{checksum_digest}",
                },
                {"name": "lumi-install.sh", "digest": f"sha256:{manager_digest}"},
            ],
            "tag_name": f"rust-v{version}",
        },
        indent=2,
    )
    return archive_path, checksum_path, metadata_json, manager_path


def metadata_without_manager_asset(metadata_json: str) -> str:
    metadata = json.loads(metadata_json)
    metadata["assets"] = [
        asset for asset in metadata["assets"] if asset["name"] != "lumi-install.sh"
    ]
    return json.dumps(metadata)


def release_metadata_with_decoys(
    archive_path: Path, checksum_path: Path, manager_path: Path
) -> str:
    fake_digest = f"sha256:{'0' * 64}"
    archive_digest = f"sha256:{hashlib.sha256(archive_path.read_bytes()).hexdigest()}"
    checksum_digest = f"sha256:{hashlib.sha256(checksum_path.read_bytes()).hexdigest()}"
    manager_digest = f"sha256:{hashlib.sha256(manager_path.read_bytes()).hexdigest()}"
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
                {
                    "metadata": {
                        "name": "lumi-install.sh",
                        "digest": fake_digest,
                    },
                    "name": "lumi-install.sh",
                    "digest": manager_digest,
                },
            ],
            "tag_name": f"rust-v{VERSION}",
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
    manager_path: Path | None = None,
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
            "LUMI_TEST_MANAGER_PATH": str(manager_path or ""),
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
    assets.append({"name": "lumi-install.sh", "digest": f"sha256:{'c' * 64}"})
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


def bashrc_has_block(root: Path) -> bool:
    bashrc = root / "home" / ".bashrc"
    if not bashrc.exists():
        return False
    return "# >>> Lumi Codex managed PATH >>>" in bashrc.read_text(encoding="utf-8")


def default_install_env(
    root: Path,
    *,
    release: str = VERSION,
    archive: Path,
    checksum: Path,
    metadata: str,
    manager: Path,
    extra: dict[str, str] | None = None,
) -> dict[str, str]:
    return make_env(
        root,
        release=release,
        metadata_json=metadata,
        checksum_path=checksum,
        archive_path=archive,
        manager_path=manager,
        extra=extra,
    )


def install_release(
    root: Path,
    archive: Path,
    checksum: Path,
    metadata: str,
    manager: Path,
    *,
    release: str = VERSION,
    args: list[str] | None = None,
    extra: dict[str, str] | None = None,
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    env = default_install_env(
        root, release=release, archive=archive, checksum=checksum,
        metadata=metadata, manager=manager, extra=extra,
    )
    result = run_manage(root, ["install", *(args or [])], env=env)
    return result, read_requests(root)


class LumiInstallShTest(unittest.TestCase):
    def test_happy_install_is_side_by_side_with_verified_manager(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)

            result, requests = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests, [METADATA_URL, CHECKSUM_URL, ARCHIVE_URL, MANAGER_URL]
            )
            for request in requests:
                self.assertIn("Lumi-weaves/codex", request)
            self.assertNotIn("openai/codex", requests)

            lr = lumi_root(root)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "shim" / "codex"), "../current/bin/codex"
            )
            self.assertEqual(
                os.readlink(lr / "shim" / "lumi-codex"),
                "../manager/lumi-install.sh",
            )

            # Visible launcher at ${LUMI_INSTALL_DIR:-$HOME/.local/bin}/lumi-codex
            launcher = root / "home" / ".local" / "bin" / "lumi-codex"
            self.assertTrue(launcher.is_symlink())
            self.assertEqual(os.readlink(launcher), str(lr / "manager" / "lumi-install.sh"))

            # Manager copy is the verified release asset, not $0.
            manager_copy = lr / "manager" / "lumi-install.sh"
            self.assertTrue(manager_copy.is_file())
            self.assertEqual(
                hashlib.sha256(manager_copy.read_bytes()).hexdigest(),
                hashlib.sha256(INSTALL_SCRIPT.read_bytes()).hexdigest(),
            )

            # Default install does NOT touch any profile and does not shadow codex.
            self.assertFalse(bashrc_has_block(root))
            keys = receipt_keys(root)
            self.assertEqual(keys["activated"], "no")
            self.assertEqual(keys["profile"], "-")
            self.assertEqual(keys["launcher"], str(launcher))
            self.assertEqual(keys["install_dir"], str(launcher.parent))

            # Strict receipt schema.
            current_receipt = (lr / "receipts" / "current.receipt").read_text(
                encoding="utf-8"
            )
            self.assertTrue(
                current_receipt.startswith("LUMI-CODEX-RECEIPT-V1\n"),
                current_receipt,
            )
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
                    "install_dir",
                    "releases_dir",
                    "receipts_dir",
                    "tmp_dir",
                },
            )
            self.assertEqual(keys["tag"], f"rust-v{VERSION}")
            self.assertEqual(keys["current"], f"{VERSION}-{TARGET}")
            self.assertEqual(keys["previous"], "-")
            self.assertEqual(len(keys["archive_sha256"]), 64)
            self.assertEqual(len(keys["bin_sha256"]), 64)

            release_receipt = (
                lr / "receipts" / f"{VERSION}-{TARGET}.receipt"
            ).read_text(encoding="utf-8")
            self.assertTrue(
                release_receipt.startswith("LUMI-CODEX-RECEIPT-V1\n"),
                release_receipt,
            )

            # Launcher execs the managed CLI.
            launched = subprocess.run(
                [str(launcher), "--version"],
                capture_output=True,
                check=False,
                env=make_env(root),
                text=True,
            )
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)
            self.assertIn("doctor: OK", doctor.stdout)

    def test_install_preserves_official_codex_byte_for_byte(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            official = official_codex_path(root)
            official.parent.mkdir(parents=True, exist_ok=True)
            official_bytes = os.urandom(4096)
            official.write_bytes(official_bytes)
            official.chmod(0o755)
            before = hashlib.sha256(official_bytes).hexdigest()

            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(hashlib.sha256(official.read_bytes()).hexdigest(), before)
            self.assertFalse((root / "official-marker").exists())

            uninstall = run_manage(root, ["uninstall"])
            self.assertEqual(uninstall.returncode, 0, uninstall.stderr)
            self.assertEqual(hashlib.sha256(official.read_bytes()).hexdigest(), before)
            self.assertFalse((root / "home" / ".local" / "bin" / "lumi-codex").exists())

    def test_activate_flag_and_activate_deactivate_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            bashrc = root / "home" / ".bashrc"

            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(bashrc_has_block(root))

            result, _ = install_release(
                root, archive, checksum, metadata, manager, args=["--activate"]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(bashrc_has_block(root))
            self.assertEqual(receipt_keys(root)["activated"], "yes")
            self.assertEqual(receipt_keys(root)["profile"], str(bashrc))

            # Official codex resolution restored after deactivate.
            env = make_env(
                root, metadata_json=metadata, checksum_path=checksum,
                archive_path=archive, manager_path=manager,
            )
            result = run_manage(root, ["deactivate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(bashrc_has_block(root))
            self.assertEqual(receipt_keys(root)["activated"], "no")

            result = run_manage(root, ["deactivate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("nothing to remove", result.stdout)

            result = run_manage(root, ["activate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(bashrc_has_block(root))
            self.assertEqual(receipt_keys(root)["activated"], "yes")

            result = run_manage(root, ["activate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                bashrc.read_text().count("# >>> Lumi Codex managed PATH >>>"), 1
            )

            # --no-activate is gone; the flag must be rejected.
            result, _ = install_release(
                root, archive, checksum, metadata, manager, args=["--no-activate"]
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Unknown install argument", result.stderr)

    def test_install_dir_absent_from_path_reports_exact_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            elsewhere = root / "elsewhere"
            result, _ = install_release(
                root,
                archive,
                checksum,
                metadata,
                manager,
                extra={"LUMI_INSTALL_DIR": str(elsewhere)},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((elsewhere / "lumi-codex").is_symlink())
            self.assertIn(f"{elsewhere} is not on PATH", result.stderr)
            self.assertIn(str(elsewhere / "lumi-codex"), result.stderr)

    def test_piped_bootstrap_install_works(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
                manager_path=manager,
            )
            piped = subprocess.run(
                ["/bin/sh"],
                input=INSTALL_SCRIPT.read_bytes(),
                capture_output=True,
                check=False,
                env=env,
            )
            self.assertEqual(piped.returncode, 0, piped.stderr.decode())
            self.assertEqual(
                os.readlink(lumi_root(root) / "current"),
                f"releases/{VERSION}-{TARGET}",
            )
            self.assertEqual(
                hashlib.sha256(
                    (lumi_root(root) / "manager" / "lumi-install.sh").read_bytes()
                ).hexdigest(),
                hashlib.sha256(INSTALL_SCRIPT.read_bytes()).hexdigest(),
            )

    def test_public_install_requires_manager_asset_and_dev_mode_is_opt_in(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            no_manager_metadata = metadata_without_manager_asset(metadata)

            result, _ = install_release(
                root,
                archive,
                checksum,
                no_manager_metadata,
                manager,
                extra={"LUMI_TEST_MANAGER_PATH": ""},
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("bootstrap asset lumi-install.sh", result.stderr)
            self.assertFalse((lumi_root(root) / "releases").exists())

            result, _ = install_release(
                root,
                archive,
                checksum,
                no_manager_metadata,
                manager,
                extra={
                    "LUMI_TEST_MANAGER_PATH": "",
                    "LUMI_DEV_MANAGER_SELF": str(INSTALL_SCRIPT),
                },
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Developer mode", result.stderr)
            self.assertEqual(
                hashlib.sha256(
                    (lumi_root(root) / "manager" / "lumi-install.sh").read_bytes()
                ).hexdigest(),
                hashlib.sha256(INSTALL_SCRIPT.read_bytes()).hexdigest(),
            )

    def test_lumi_semver_tag_version_installs(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(
                root, version=LUMI_VERSION
            )
            result, _ = install_release(
                root, archive, checksum, metadata, manager, release=LUMI_VERSION
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            release_dir = f"{LUMI_VERSION}-{TARGET}"
            self.assertEqual(os.readlink(lumi_root(root) / "current"), f"releases/{release_dir}")
            self.assertEqual(receipt_keys(root)["version"], LUMI_VERSION)

    def test_invalid_lumi_semver_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(
                root, archive, checksum, metadata, manager, release="0.147.0-lumi"
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Invalid Codex release version", result.stderr)

    def test_target_allowlist_rejects_unknown_and_traversal_before_requests(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            for bad_target in ("bogus-target", "../../etc/passwd", "x86_64;rm"):
                with self.subTest(target=bad_target):
                    result, requests = install_release(
                        root,
                        archive,
                        checksum,
                        metadata,
                        manager,
                        args=["--target", bad_target],
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("Unsupported target", result.stderr)
                    self.assertEqual(requests, [])

    def test_symlinked_root_and_control_char_root_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            real_root = root / "xdg" / "lumi-codex-real"
            real_root.mkdir(parents=True)
            (root / "xdg" / "lumi-codex").symlink_to("lumi-codex-real")

            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symlinked root", result.stderr)
            self.assertFalse((real_root / "releases").exists())

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            evil_root = "/tmp/evil\nlumi-codex"
            result, _ = install_release(
                root,
                archive,
                checksum,
                metadata,
                manager,
                extra={"LUMI_ROOT": evil_root},
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("control characters", result.stderr)

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            evil_install_dir = "/tmp/evil\nbin"
            result, _ = install_release(
                root,
                archive,
                checksum,
                metadata,
                manager,
                extra={"LUMI_INSTALL_DIR": evil_install_dir},
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("control characters", result.stderr)

    def test_lumi_target_env_and_aarch64_target_install(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(
                root, target=AARCH64_TARGET
            )
            result, _ = install_release(
                root,
                archive,
                checksum,
                metadata,
                manager,
                args=["--target", AARCH64_TARGET],
                extra={"LUMI_TARGET": "x86_64-unknown-linux-musl"},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            release_dir = f"{VERSION}-{AARCH64_TARGET}"
            self.assertEqual(os.readlink(lumi_root(root) / "current"), f"releases/{release_dir}")

            with tempfile.TemporaryDirectory() as temp_dir2:
                root2 = Path(temp_dir2)
                archive2, checksum2, metadata2, manager2 = create_package_release(
                    root2, target=AARCH64_TARGET
                )
                result2, _ = install_release(
                    root2,
                    archive2,
                    checksum2,
                    metadata2,
                    manager2,
                    release=VERSION,
                    extra={"LUMI_TARGET": AARCH64_TARGET},
                )
                self.assertEqual(result2.returncode, 0, result2.stderr)

    def test_archive_traversal_and_absolute_members_rejected(self) -> None:
        for member in ("../escape", "/etc/escape"):
            with self.subTest(member=member):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    if member.startswith("/"):
                        archive, checksum, metadata, manager = create_package_release(
                            root, absolute_member=member
                        )
                    else:
                        archive, checksum, metadata, manager = create_package_release(
                            root, traversal_member=member
                        )
                    result, _ = install_release(
                        root, archive, checksum, metadata, manager
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("unsafe member paths", result.stderr)
                    self.assertFalse((lumi_root(root) / "releases").exists())

    def test_archive_symlink_member_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(
                root, symlink_member="bin/codex"
            )
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unsafe entry types", result.stderr)
            self.assertFalse((lumi_root(root) / "releases").exists())

    def test_wrong_distribution_and_bad_metadata_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(
                root, metadata_override={"distribution": "other"}
            )
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("completeness", result.stderr)
            self.assertFalse((lumi_root(root) / "releases").exists())

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(
                root, metadata_override={"version": OTHER_VERSION}
            )
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("completeness", result.stderr)

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(
                root, metadata_override={"variant": "other"}
            )
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("completeness", result.stderr)

    def test_repeat_install_reuses_digest_verified_release(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)

            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            (root / "requests.log").unlink()

            result, requests = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(requests, [METADATA_URL])
            self.assertIn("digest-verified; reusing it", result.stdout)
            self.assertEqual(len(list((lumi_root(root) / "releases").iterdir())), 1)

    def test_tampered_binary_is_repaired_by_reinstall(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)

            codex = (
                lumi_root(root) / "releases" / f"{VERSION}-{TARGET}" / "bin" / "codex"
            )
            codex.write_text(
                f"#!/bin/sh\nprintf 'codex-cli {VERSION}\\n'\n# tampered\n",
                encoding="utf-8",
            )
            codex.chmod(0o755)
            (root / "requests.log").unlink()

            result, requests = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(requests, [METADATA_URL, CHECKSUM_URL, ARCHIVE_URL, MANAGER_URL])
            self.assertNotIn("tampered", codex.read_text(encoding="utf-8"))
            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)

    def test_upgrade_tracks_previous_and_rollback_swaps_both_directions(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata, first_manager = (
                create_package_release(root, version=VERSION)
            )
            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            first = install_release(
                root, first_archive, first_checksum, first_metadata, first_manager
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = install_release(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                second_manager,
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

            result = run_manage(root, ["rollback"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "previous"), f"releases/{OTHER_VERSION}-{TARGET}"
            )

            result = run_manage(root, ["rollback"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "previous"), f"releases/{VERSION}-{TARGET}"
            )

            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)

    def test_rollback_refuses_without_previous_or_with_missing_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)

            result = run_manage(root, ["rollback"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Nothing to roll back to", result.stderr)

            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            second = install_release(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                second_manager,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)
            (lr / "receipts" / f"{VERSION}-{TARGET}.receipt").unlink()
            result = run_manage(root, ["rollback"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing rollback", result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )

    def test_checksum_bad_and_metadata_failure_install_do_not_switch(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata, first_manager = (
                create_package_release(root, version=VERSION)
            )
            first = install_release(
                root, first_archive, first_checksum, first_metadata, first_manager
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            before = receipt_keys(root)

            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            env = make_env(
                root,
                release=OTHER_VERSION,
                metadata_json=second_metadata,
                checksum_path=second_checksum,
                archive_path=second_archive,
                manager_path=second_manager,
                archive_mode="corrupt",
            )
            result = run_manage(root, ["install"], env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("checksum", result.stderr)
            lr = lumi_root(root)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )
            self.assertFalse((lr / "releases" / f"{OTHER_VERSION}-{TARGET}").exists())
            self.assertEqual(receipt_keys(root), before)

            env = make_env(root, metadata_failure=True)
            result = run_manage(root, ["install"], env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("release metadata", result.stderr)

    def test_list_reports_current_previous_activation_and_launcher(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata, first_manager = (
                create_package_release(root, version=VERSION)
            )
            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            first = install_release(
                root, first_archive, first_checksum, first_metadata, first_manager
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = install_release(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                second_manager,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)

            result = run_manage(root, ["list"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"current: {OTHER_VERSION}-{TARGET}", result.stdout)
            self.assertIn(f"previous: {VERSION}-{TARGET}", result.stdout)
            self.assertIn(f"  - {VERSION}-{TARGET}", result.stdout)
            self.assertIn("activation: disabled", result.stdout)
            self.assertIn("launcher:", result.stdout)

    def test_doctor_clean_reports_official_codex_without_executing(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
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
            result = run_manage(root, ["doctor"], env=make_env(root))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("not installed", result.stdout)

    def test_doctor_detects_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)

            clean = run_manage(root, ["doctor"])
            self.assertEqual(clean.returncode, 0, clean.stdout)

            codex = lr / "releases" / f"{VERSION}-{TARGET}" / "bin" / "codex"
            codex.write_text("#!/bin/sh\nexit 7\n", encoding="utf-8")
            codex.chmod(0o755)
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("PROBLEMS FOUND", drifted.stdout)
            self.assertIn("bin/codex sha256", drifted.stdout)

    def test_doctor_detects_corrupt_receipt_unknown_symlink_and_bad_names(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
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
            shim_codex.unlink()
            shim_codex.symlink_to("../current/bin/codex")

            # Unsafe current value in the receipt is confined and reported.
            receipt.write_text(
                receipt.read_text(encoding="utf-8").replace(
                    f"current={VERSION}-{TARGET}", "current=../evil"
                ),
                encoding="utf-8",
            )
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("unsafe", drifted.stdout)

    def test_doctor_detects_missing_previous_receipt_and_profile_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata, first_manager = (
                create_package_release(root, version=VERSION)
            )
            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            first = install_release(
                root, first_archive, first_checksum, first_metadata, first_manager
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = install_release(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                second_manager,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)
            lr = lumi_root(root)

            (lr / "receipts" / f"{VERSION}-{TARGET}.receipt").unlink()
            drifted = run_manage(root, ["doctor"])
            self.assertNotEqual(drifted.returncode, 0)
            self.assertIn("previous release", drifted.stdout)

            first = install_release(
                root, first_archive, first_checksum, first_metadata, first_manager,
                args=["--activate"],
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
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
            archive, checksum, metadata, manager = create_package_release(root)
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
                manager_path=manager,
            )
            result = run_manage(root, ["install", "--activate"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(bashrc_has_block(root))

            uninstall = run_manage(root, ["uninstall"], env=env)
            self.assertEqual(uninstall.returncode, 0, uninstall.stderr)
            self.assertFalse(lumi_root(root).exists())
            self.assertFalse(bashrc_has_block(root))
            self.assertFalse((root / "home" / ".local" / "bin" / "lumi-codex").exists())
            self.assertEqual(
                hashlib.sha256(official.read_bytes()).hexdigest(), official_hash
            )
            self.assertEqual(
                (codex_home / "config.toml").read_text(encoding="utf-8"), "keep me\n"
            )
            self.assertTrue((root / "xdg").exists())

    def test_uninstall_refuses_unknown_symlink_missing_receipt_and_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)

            shim_codex = lr / "shim" / "codex"
            shim_codex.unlink()
            shim_codex.symlink_to("/usr/bin/codex")
            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("Refusing uninstall", uninstall.stderr)
            self.assertTrue((lr / "releases").exists())
            shim_codex.unlink()
            shim_codex.symlink_to("../current/bin/codex")

            (lr / "receipts" / "current.receipt").unlink()
            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("Refusing uninstall", uninstall.stderr)
            self.assertTrue((lr / "releases").exists())

    def test_uninstall_refuses_drifted_or_symlinked_profile_before_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(
                root, archive, checksum, metadata, manager, args=["--activate"]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)
            bashrc = root / "home" / ".bashrc"

            drifted = bashrc.read_text(encoding="utf-8").replace(
                f'export PATH="{lr}/shim:$PATH"',
                'export PATH="/tmp/somewhere-else:$PATH"',
            )
            bashrc.write_text(drifted, encoding="utf-8")
            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("Refusing uninstall", uninstall.stderr)
            self.assertIn("drifted", uninstall.stderr)
            self.assertTrue((lr / "releases").exists())

            bashrc.write_text(
                bashrc.read_text(encoding="utf-8").replace(
                    'export PATH="/tmp/somewhere-else:$PATH"',
                    f'export PATH="{lr}/shim:$PATH"',
                ),
                encoding="utf-8",
            )
            bashrc_real = root / "home" / ".bashrc.real"
            bashrc_real.write_text("real\n", encoding="utf-8")
            bashrc.unlink()
            bashrc.symlink_to(".bashrc.real")
            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("symlink", uninstall.stderr)
            self.assertTrue((lr / "releases").exists())

    def test_unknown_launcher_target_refused_on_uninstall(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            launcher = root / "home" / ".local" / "bin" / "lumi-codex"
            launcher.unlink()
            launcher.symlink_to("/usr/bin/codex")
            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("Refusing uninstall", uninstall.stderr)
            self.assertTrue((lumi_root(root) / "releases").exists())

    def test_launcher_execs_current_codex_and_intercepts_manage(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            env = make_env(
                root, metadata_json=metadata, checksum_path=checksum,
                archive_path=archive, manager_path=manager,
            )
            launcher = root / "home" / ".local" / "bin" / "lumi-codex"

            launched = subprocess.run(
                [str(launcher), "--version"], capture_output=True, check=False,
                env=env, text=True,
            )
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

            launched = subprocess.run(
                [str(launcher)], capture_output=True, check=False, env=env, text=True
            )
            self.assertEqual(launched.returncode, 0, launched.stderr)
            self.assertEqual(launched.stdout.strip(), f"codex-cli {VERSION}")

            managed = subprocess.run(
                [str(launcher), "manage", "list"], capture_output=True, check=False,
                env=env, text=True,
            )
            self.assertEqual(managed.returncode, 0, managed.stderr)
            self.assertIn(f"current: {VERSION}-{TARGET}", managed.stdout)

            bogus = subprocess.run(
                [str(launcher), "manage", "bogus"], capture_output=True, check=False,
                env=env, text=True,
            )
            self.assertNotEqual(bogus.returncode, 0)
            self.assertIn("Unknown Lumi Codex action", bogus.stderr)

            shim_launched = subprocess.run(
                [str(lumi_root(root) / "shim" / "lumi-codex"), "--version"],
                capture_output=True, check=False, env=env, text=True,
            )
            self.assertEqual(shim_launched.stdout.strip(), f"codex-cli {VERSION}")

    def test_metadata_decoys_do_not_confuse_asset_selection(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, _metadata, manager = create_package_release(root)
            env = make_env(
                root,
                metadata_json=release_metadata_with_decoys(archive, checksum, manager),
                checksum_path=checksum,
                archive_path=archive,
                manager_path=manager,
            )
            result = run_manage(root, ["install"], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                read_requests(root),
                [METADATA_URL, CHECKSUM_URL, ARCHIVE_URL, MANAGER_URL],
            )

    def test_concurrent_installs_serialize_to_consistent_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata, first_manager = (
                create_package_release(root, version=VERSION)
            )
            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            env_a = make_env(
                root,
                release=VERSION,
                metadata_json=first_metadata,
                checksum_path=first_checksum,
                archive_path=first_archive,
                manager_path=first_manager,
                archive_delay="3",
            )
            env_b = make_env(
                root,
                release=OTHER_VERSION,
                metadata_json=second_metadata,
                checksum_path=second_checksum,
                archive_path=second_archive,
                manager_path=second_manager,
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
            out_a, err_a = proc_a.communicate(timeout=90)
            out_b, err_b = proc_b.communicate(timeout=90)
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
            archive, checksum, metadata, manager = create_package_release(root)
            env = make_env(
                root,
                metadata_json=metadata,
                checksum_path=checksum,
                archive_path=archive,
                manager_path=manager,
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
            self.assertFalse((lr / "tmp" / "pending.journal").exists())

            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)

    def test_interruption_after_current_switch_reconciles_and_preserves_old_good(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata, first_manager = (
                create_package_release(root, version=VERSION)
            )
            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            first = install_release(
                root, first_archive, first_checksum, first_metadata, first_manager
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            lr = lumi_root(root)

            # Start an upgrade and SIGKILL the whole session right after the
            # current symlink has switched but before the previous switch.
            env = make_env(
                root,
                release=OTHER_VERSION,
                metadata_json=second_metadata,
                checksum_path=second_checksum,
                archive_path=second_archive,
                manager_path=second_manager,
            )
            env["LUMI_TEST_SLOW_AFTER_SWITCH"] = "30"
            proc = subprocess.Popen(
                ["/bin/sh", str(INSTALL_SCRIPT), "manage", "install"],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
            )
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                try:
                    if (
                        os.readlink(lr / "current")
                        == f"releases/{OTHER_VERSION}-{TARGET}"
                    ):
                        break
                except FileNotFoundError:
                    pass
                time.sleep(0.05)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            proc.communicate(timeout=10)

            journal = lr / "tmp" / "pending.journal"
            self.assertTrue(journal.exists())
            # The crash happened before the previous switch: previous is still
            # absent and the journal keeps the old-good release as its target.
            self.assertFalse(os.path.lexists(lr / "previous"))

            # Re-running the same install reconciles the journal deterministically.
            result, _ = install_release(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                second_manager,
                release=OTHER_VERSION,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Reconciled journal", result.stdout)
            self.assertFalse(journal.exists())
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{OTHER_VERSION}-{TARGET}"
            )
            self.assertEqual(
                os.readlink(lr / "previous"), f"releases/{VERSION}-{TARGET}"
            )
            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)

            # Old-good release is preserved as the rollback target.
            result = run_manage(root, ["rollback"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )

    def test_tampered_journal_fails_closed_and_doctor_reports(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata, manager = create_package_release(root)
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertEqual(result.returncode, 0, result.stderr)
            lr = lumi_root(root)

            journal = lr / "tmp" / "pending.journal"
            journal.parent.mkdir(parents=True, exist_ok=True)
            journal.write_text(
                "LUMI-CODEX-JOURNAL-V1\n"
                "schema=1\n"
                "op=install\n"
                "current_old=0.147.0-x86_64-unknown-linux-musl\n"
                "current_new=../../etc\n"
                "previous_old=-\n"
                "previous_new=-\n",
                encoding="utf-8",
            )
            result, _ = install_release(root, archive, checksum, metadata, manager)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("journal", result.stderr)
            self.assertEqual(
                os.readlink(lr / "current"), f"releases/{VERSION}-{TARGET}"
            )

            doctor = run_manage(root, ["doctor"])
            self.assertNotEqual(doctor.returncode, 0)
            self.assertIn("journal", doctor.stdout)

            uninstall = run_manage(root, ["uninstall"])
            self.assertNotEqual(uninstall.returncode, 0)
            self.assertIn("journal", uninstall.stderr)
            self.assertTrue((lr / "releases").exists())

    def test_valid_stale_journal_reconciled_by_doctor(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_archive, first_checksum, first_metadata, first_manager = (
                create_package_release(root, version=VERSION)
            )
            second_archive, second_checksum, second_metadata, second_manager = (
                create_package_release(root, version=OTHER_VERSION)
            )
            first = install_release(
                root, first_archive, first_checksum, first_metadata, first_manager
            )
            self.assertEqual(first[0].returncode, 0, first[0].stderr)
            second = install_release(
                root,
                second_archive,
                second_checksum,
                second_metadata,
                second_manager,
                release=OTHER_VERSION,
            )
            self.assertEqual(second[0].returncode, 0, second[0].stderr)
            lr = lumi_root(root)

            # Simulate a completed-but-not-finalized journal: switches already
            # applied, receipt still the old one.
            journal = lr / "tmp" / "pending.journal"
            journal.write_text(
                "LUMI-CODEX-JOURNAL-V1\n"
                "schema=1\n"
                "op=rollback\n"
                f"current_old={VERSION}-{TARGET}\n"
                f"current_new={OTHER_VERSION}-{TARGET}\n"
                f"previous_old={OTHER_VERSION}-{TARGET}\n"
                f"previous_new={VERSION}-{TARGET}\n",
                encoding="utf-8",
            )
            os.unlink(lr / "previous")
            os.symlink(f"releases/{VERSION}-{TARGET}", lr / "previous")

            doctor = run_manage(root, ["doctor"])
            self.assertEqual(doctor.returncode, 0, doctor.stdout + doctor.stderr)
            self.assertIn("Reconciled journal", doctor.stdout)
            self.assertFalse(journal.exists())
            keys = receipt_keys(root)
            self.assertEqual(keys["current"], f"{OTHER_VERSION}-{TARGET}")
            self.assertEqual(keys["previous"], f"{VERSION}-{TARGET}")


if __name__ == "__main__":
    unittest.main()
