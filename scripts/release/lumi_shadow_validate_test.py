#!/usr/bin/env python3
"""Tests for lumi_shadow_validate_package.py native-run returncode handling.

Run with: python3 scripts/release/lumi_shadow_validate_test.py
"""

from __future__ import annotations

import io
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock


sys.path.insert(0, str(Path(__file__).resolve().parent))

import lumi_shadow_validate_package  # noqa: E402
from lumi_shadow_validate_package import validate_archive  # noqa: E402


def _run_fake(returncode: int, stdout: str = "") -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(
        args=[], returncode=returncode, stdout=stdout, stderr=""
    )


VER = "0.147.0-lumi.4"


def macho_arm64() -> bytes:
    return b"\xcf\xfa\xed\xfe" + (0x0100000C).to_bytes(4, "little") + b"\x00" * 64


def make_valid_darwin_archive(path: Path) -> None:
    meta = {
        "layoutVersion": 1,
        "distribution": "lumi",
        "version": VER,
        "target": "aarch64-apple-darwin",
        "variant": "codex",
        "entrypoint": "bin/codex",
        "resourcesDir": "codex-resources",
        "pathDir": "codex-path",
    }
    with tarfile.open(path, "w:gz") as tf:

        def add(name: str, data: bytes) -> None:
            ti = tarfile.TarInfo(name)
            ti.size = len(data)
            ti.mode = 0o755
            ti.type = tarfile.REGTYPE
            tf.addfile(ti, io.BytesIO(data))

        add("bin/codex", macho_arm64() + VER.encode())
        add("bin/codex-code-mode-host", macho_arm64() + VER.encode())
        add("codex-path/rg", b"rg")
        add("codex-resources/zsh/bin/zsh", b"zsh")
        add("codex-package.json", json.dumps(meta, indent=2).encode())


class NativeRunReturnCodeTest(unittest.TestCase):
    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.root = Path(self._temp.name)
        self.archive = self.root / "pkg.tar.gz"
        make_valid_darwin_archive(self.archive)

    def tearDown(self) -> None:
        self._temp.cleanup()

    def _run(self, returncode: int, stdout: str = "") -> None:
        fake = _run_fake(returncode, stdout)
        with mock.patch.object(
            lumi_shadow_validate_package, "subprocess"
        ) as fake_subprocess:
            fake_subprocess.run.return_value = fake
            validate_archive(
                self.archive,
                "aarch64-apple-darwin",
                VER,
                run_native=True,
            )

    def test_native_run_requires_returncode_zero(self) -> None:
        with self.assertRaises(SystemExit):
            self._run(returncode=1, stdout=f"codex {VER}\n")

    def test_native_run_zero_rc_and_matching_version_passes(self) -> None:
        self._run(returncode=0, stdout=f"codex {VER}\n")

    def test_native_run_zero_rc_wrong_version_fails(self) -> None:
        with self.assertRaises(SystemExit):
            self._run(returncode=0, stdout="codex 9.9.9-other.1\n")

    def test_native_run_empty_output_fails(self) -> None:
        with self.assertRaises(SystemExit):
            self._run(returncode=0, stdout="")

    def test_native_run_zero_rc_matching_version_uses_fake_stdout(self) -> None:
        self._run(returncode=0, stdout=f"codex {VER}\n")


if __name__ == "__main__":
    unittest.main(verbosity=2)
