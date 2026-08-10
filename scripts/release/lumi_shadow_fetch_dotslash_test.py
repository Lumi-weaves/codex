"""Tests for lumi_shadow_fetch_dotslash.py (curl-based DotSlash fetching).

Run with: python3 scripts/release/lumi_shadow_fetch_dotslash_test.py

The helper is exercised end-to-end as a subprocess against the real canonical
codex_package modules (from this checkout) with a fake curl that serves
fixture archives, can fail the first N attempts (retry behavior), and logs
the exact bounded-retry argument contract.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

HELPER = Path(__file__).resolve().parent / "lumi_shadow_fetch_dotslash.py"
TOOLS_ROOT = Path(__file__).resolve().parents[2]  # repository root
TARGET = "aarch64-unknown-linux-musl"
PLATFORM = "linux-aarch64"

RG_URL = "https://artifact.invalid/ripgrep-15.2.0-linux-aarch64.tar.gz"
ZSH_URL = "https://artifact.invalid/codex-zsh-linux-aarch64.tar.gz"
ZSH_MANIFEST_URL = "https://artifact.invalid/codex-zsh"


def make_tar_gz(
    path: Path,
    members: dict[str, bytes],
    extra_members: list[str] | None = None,
) -> None:
    with tarfile.open(path, "w:gz") as tf:
        for name, data in members.items():
            ti = tarfile.TarInfo(name)
            ti.size = len(data)
            ti.mode = 0o755
            ti.type = tarfile.REGTYPE
            tf.addfile(ti, io.BytesIO(data))
        for name in extra_members or []:
            ti = tarfile.TarInfo(name)
            ti.size = 0
            ti.mode = 0o644
            ti.type = tarfile.REGTYPE
            tf.addfile(ti, io.BytesIO(b""))


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def manifest_for(
    *,
    archive: Path,
    url: str,
    member: str,
    platform: str = PLATFORM,
    digest: str | None = None,
    size: int | None = None,
) -> dict:
    return {
        "name": "fixture",
        "platforms": {
            platform: {
                "size": archive.stat().st_size if size is None else size,
                "hash": "sha256",
                "digest": sha256_of(archive) if digest is None else digest,
                "format": "tar.gz",
                "path": member,
                "providers": [{"url": url}],
            }
        },
    }


FAKE_CURL = r"""#!/usr/bin/env python3
import json, os, sys

args = sys.argv[1:]
target = None
url = None
retries = 0
i = 0
value_flags = {"-o", "--retry", "--retry-delay", "--connect-timeout", "--max-time"}
while i < len(args):
    arg = args[i]
    if arg in value_flags:
        if arg == "-o":
            target = args[i + 1]
        elif arg == "--retry":
            retries = int(args[i + 1])
        i += 2
    elif arg.startswith("-"):
        i += 1
    else:
        url = arg
        i += 1

with open(os.environ["LUMI_SHADOW_CURL_LOG"], "a") as log:
    log.write(" ".join(args) + "\n")

counter = os.environ.get("LUMI_SHADOW_CURL_COUNTER")
fail_first = int(os.environ.get("LUMI_SHADOW_CURL_FAIL_FIRST", "0"))
mapping = json.load(open(os.environ["LUMI_SHADOW_CURL_MAP"]))
# Emulate curl's internal --retry/--retry-all-errors loop, bounded by the
# same finite retry count the helper passes.
for attempt in range(retries + 1):
    if counter:
        attempts = 0
        if os.path.exists(counter):
            attempts = int(open(counter).read())
        with open(counter, "w") as fh:
            fh.write(str(attempts + 1))
    if attempt < fail_first:
        continue
    with open(target, "wb") as out:
        out.write(open(mapping[url], "rb").read())
    sys.exit(0)
sys.exit(22)
"""


class FetchDotslashTest(unittest.TestCase):
    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.root = Path(self._temp.name)
        self.out = self.root / "out"

        self.rg_archive = self.root / "rg.tar.gz"
        self.rg_bytes = b"#!/bin/sh\necho fixture-rg\n"
        make_tar_gz(
            self.rg_archive,
            {"ripgrep-15.2.0-aarch64-unknown-linux-gnu/rg": self.rg_bytes},
            extra_members=["../evil", "/abs/evil"],
        )
        self.zsh_archive = self.root / "zsh.tar.gz"
        self.zsh_bytes = b"#!/bin/sh\necho fixture-zsh\n"
        make_tar_gz(self.zsh_archive, {"codex-zsh/bin/zsh": self.zsh_bytes})

        self.rg_manifest = self.root / "rg-manifest"
        self.rg_manifest.write_text(
            json.dumps(
                manifest_for(archive=self.rg_archive, url=RG_URL, member="ripgrep-15.2.0-aarch64-unknown-linux-gnu/rg")
            ),
            encoding="utf-8",
        )
        self.zsh_manifest = self.root / "zsh-manifest"
        self.zsh_manifest.write_text(
            json.dumps(
                manifest_for(archive=self.zsh_archive, url=ZSH_URL, member="codex-zsh/bin/zsh")
            ),
            encoding="utf-8",
        )

        self.fake_curl = self.root / "fake-curl"
        self.fake_curl.write_text(FAKE_CURL, encoding="utf-8")
        self.fake_curl.chmod(0o755)
        self.curl_log = self.root / "curl.log"
        self.counter = self.root / "counter"
        self.curl_map = self.root / "map.json"
        self.curl_map.write_text(
            json.dumps(
                {
                    RG_URL: str(self.rg_archive),
                    ZSH_URL: str(self.zsh_archive),
                    ZSH_MANIFEST_URL: str(self.zsh_manifest),
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self._temp.cleanup()

    def _run(
        self,
        *extra: str,
        fail_first: int = 0,
        extra_env: dict[str, str] | None = None,
        target: str = TARGET,
    ) -> subprocess.CompletedProcess[str]:
        env = dict(os.environ)
        env.update(
            {
                "LUMI_SHADOW_CURL": str(self.fake_curl),
                "LUMI_SHADOW_CURL_MAP": str(self.curl_map),
                "LUMI_SHADOW_CURL_LOG": str(self.curl_log),
                "LUMI_SHADOW_CURL_COUNTER": str(self.counter),
                "LUMI_SHADOW_CURL_FAIL_FIRST": str(fail_first),
            }
        )
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            [
                sys.executable,
                str(HELPER),
                "--target",
                target,
                "--tools-root",
                str(TOOLS_ROOT),
                "--output-dir",
                str(self.out),
                *extra,
            ],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    def _base_args(self) -> list[str]:
        return [
            "--rg-manifest",
            str(self.rg_manifest),
            "--zsh-manifest-path",
            str(self.zsh_manifest),
        ]

    def _assert_env_lines(self, result: subprocess.CompletedProcess[str]) -> None:
        lines = result.stdout.strip().splitlines()
        self.assertEqual(len(lines), 2, result.stdout)
        self.assertTrue(lines[0].startswith("LUMI_SHADOW_RG_BIN="), lines)
        self.assertTrue(lines[1].startswith("LUMI_SHADOW_ZSH_BIN="), lines)

    def test_success_fetches_verifies_extracts_and_prints_paths(self) -> None:
        result = self._run(*self._base_args())
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_env_lines(result)

        rg_path = Path(result.stdout.splitlines()[0].split("=", 1)[1])
        zsh_path = Path(result.stdout.splitlines()[1].split("=", 1)[1])
        self.assertEqual(rg_path.read_bytes(), self.rg_bytes)
        self.assertEqual(zsh_path.read_bytes(), self.zsh_bytes)
        for path in (rg_path, zsh_path):
            self.assertTrue(path.stat().st_mode & stat.S_IXUSR, path)

        # Bounded retry contract on every curl invocation.
        log = self.curl_log.read_text()
        self.assertIn("--retry 5", log)
        self.assertIn("--retry-all-errors", log)
        self.assertIn("--connect-timeout 20", log)
        self.assertIn("--max-time 300", log)
        self.assertNotIn(".tmp", os.listdir(self.out))

    def test_retries_then_succeeds(self) -> None:
        result = self._run(*self._base_args(), fail_first=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_env_lines(result)
        # One bounded curl invocation per artifact; each retried 3 times
        # internally (4 attempts per artifact) against the flaky proxy.
        self.assertEqual(int(self.counter.read_text()), 8)
        log_lines = self.curl_log.read_text().splitlines()
        self.assertEqual(len(log_lines), 2)
        for line in log_lines:
            self.assertIn("--retry 5", line)

    def test_persistent_failure_is_bounded_and_clean(self) -> None:
        result = self._run(*self._base_args(), fail_first=99)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("bounded retries", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(
            [name for name in os.listdir(self.out) if name.endswith(".tmp")],
            [],
        )

    def test_digest_mismatch_fails_and_drops_only_the_bad_archive(self) -> None:
        bad = self.root / "bad-rg-manifest"
        bad.write_text(
            json.dumps(
                manifest_for(
                    archive=self.rg_archive,
                    url=RG_URL,
                    member="ripgrep-15.2.0-aarch64-unknown-linux-gnu/rg",
                    digest="0" * 64,
                )
            ),
            encoding="utf-8",
        )
        result = self._run("--rg-manifest", str(bad), "--zsh-manifest-path", str(self.zsh_manifest))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sha256", result.stderr)
        self.assertFalse((self.out / "ripgrep-15.2.0-linux-aarch64.tar.gz").exists())

    def test_size_mismatch_fails(self) -> None:
        bad = self.root / "bad-size-manifest"
        bad.write_text(
            json.dumps(
                manifest_for(
                    archive=self.rg_archive,
                    url=RG_URL,
                    member="ripgrep-15.2.0-aarch64-unknown-linux-gnu/rg",
                    size=1,
                )
            ),
            encoding="utf-8",
        )
        result = self._run("--rg-manifest", str(bad), "--zsh-manifest-path", str(self.zsh_manifest))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("size", result.stderr)

    def test_zsh_missing_platform_emits_empty_path(self) -> None:
        mac_only = self.root / "mac-only-zsh-manifest"
        mac_only.write_text(
            json.dumps(
                manifest_for(
                    archive=self.zsh_archive,
                    url=ZSH_URL,
                    member="codex-zsh/bin/zsh",
                    platform="macos-aarch64",
                )
            ),
            encoding="utf-8",
        )
        result = self._run("--rg-manifest", str(self.rg_manifest), "--zsh-manifest-path", str(mac_only))
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(lines[1], "LUMI_SHADOW_ZSH_BIN=")
        self.assertTrue(lines[0].startswith("LUMI_SHADOW_RG_BIN="))

    def test_rg_missing_platform_fails(self) -> None:
        mac_only = self.root / "mac-only-rg-manifest"
        mac_only.write_text(
            json.dumps(
                manifest_for(
                    archive=self.rg_archive,
                    url=RG_URL,
                    member="ripgrep-15.2.0-aarch64-unknown-linux-gnu/rg",
                    platform="macos-aarch64",
                )
            ),
            encoding="utf-8",
        )
        result = self._run("--rg-manifest", str(mac_only), "--zsh-manifest-path", str(self.zsh_manifest))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("linux-aarch64", result.stderr)

    def test_extraction_is_single_member_only(self) -> None:
        result = self._run(*self._base_args())
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_env_lines(result)
        entries = sorted(os.listdir(self.out))
        self.assertEqual(
            entries,
            [
                "codex-zsh-linux-aarch64.tar.gz",
                "rg",
                "ripgrep-15.2.0-linux-aarch64.tar.gz",
                "zsh",
            ],
            entries,
        )
        self.assertFalse((self.out / "evil").exists())
        self.assertFalse(Path("/abs/evil").exists())

    def test_zsh_manifest_from_url(self) -> None:
        result = self._run(
            "--rg-manifest",
            str(self.rg_manifest),
            "--zsh-manifest-url",
            ZSH_MANIFEST_URL,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_env_lines(result)
        self.assertTrue((self.out / "codex-zsh-manifest").is_file())

    def test_bounded_retry_values_rejected_outside_range(self) -> None:
        result = self._run(
            *self._base_args(),
            extra_env={"LUMI_SHADOW_CURL_MAX_TIME": "999999"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("out of bounded range", result.stderr)

    def test_x86_64_unknown_linux_musl_target_fetches(self) -> None:
        """DotSlash resolution for the x86 shadow target (linux-x86_64)."""
        platform = "linux-x86_64"
        member = "ripgrep-15.2.0-x86_64-unknown-linux-gnu/rg"
        rg_url = "https://artifact.invalid/ripgrep-15.2.0-linux-x86_64.tar.gz"
        zsh_url = "https://artifact.invalid/codex-zsh-linux-x86_64.tar.gz"
        rg_archive = self.root / "rg-x86.tar.gz"
        rg_bytes = b"#!/bin/sh\necho fixture-rg-x86\n"
        make_tar_gz(rg_archive, {member: rg_bytes})
        zsh_archive = self.root / "zsh-x86.tar.gz"
        zsh_bytes = b"#!/bin/sh\necho fixture-zsh-x86\n"
        make_tar_gz(zsh_archive, {"codex-zsh/bin/zsh": zsh_bytes})
        rg_manifest = self.root / "rg-x86-manifest"
        rg_manifest.write_text(
            json.dumps(
                manifest_for(
                    archive=rg_archive,
                    url=rg_url,
                    member=member,
                    platform=platform,
                )
            ),
            encoding="utf-8",
        )
        zsh_manifest = self.root / "zsh-x86-manifest"
        zsh_manifest.write_text(
            json.dumps(
                manifest_for(
                    archive=zsh_archive,
                    url=zsh_url,
                    member="codex-zsh/bin/zsh",
                    platform=platform,
                )
            ),
            encoding="utf-8",
        )

        extra_map = {
            rg_url: str(rg_archive),
            zsh_url: str(zsh_archive),
        }
        mapping = json.loads(self.curl_map.read_text())
        mapping.update(extra_map)
        self.curl_map.write_text(json.dumps(mapping), encoding="utf-8")

        result = self._run(
            "--target",
            "x86_64-unknown-linux-musl",
            "--rg-manifest",
            str(rg_manifest),
            "--zsh-manifest-path",
            str(zsh_manifest),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self._assert_env_lines(result)
        rg_path = Path(result.stdout.splitlines()[0].split("=", 1)[1])
        zsh_path = Path(result.stdout.splitlines()[1].split("=", 1)[1])
        self.assertEqual(rg_path.read_bytes(), rg_bytes)
        self.assertEqual(zsh_path.read_bytes(), zsh_bytes)
        self.assertIn("--retry 5", self.curl_log.read_text())


if __name__ == "__main__":
    unittest.main(verbosity=2)
