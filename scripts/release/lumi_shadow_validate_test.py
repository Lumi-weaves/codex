#!/usr/bin/env python3
"""Tests for lumi_shadow_validate_package.py: native-run returncode
handling, x86_64/aarch64 Linux arch + no-PT_INTERP checks, and the embedded
packaged-bwrap SHA-256 digest check.

Run with: python3 scripts/release/lumi_shadow_validate_test.py
"""

from __future__ import annotations

import io
import hashlib
import json
import struct
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


def elf64(machine: int, ph_type: int = 1) -> bytes:
    """Minimal 64-bit little-endian ELF with one program header."""
    header = bytearray(64)
    header[0:4] = b"\x7fELF"
    header[4] = 2  # ELFCLASS64
    header[5] = 1  # ELFDATA2LSB
    struct.pack_into("<H", header, 0x12, machine)  # e_machine
    struct.pack_into("<Q", header, 0x20, 64)  # e_phoff
    struct.pack_into("<H", header, 0x36, 56)  # e_phentsize
    struct.pack_into("<H", header, 0x38, 1)  # e_phnum
    program_header = bytearray(56)
    struct.pack_into("<I", program_header, 0, ph_type)  # p_type
    return bytes(header) + bytes(program_header)


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


def make_linux_archive(
    path: Path,
    target: str,
    machine: int,
    ph_type: int = 1,
    digest_embed: str | None = None,
) -> None:
    meta = {
        "layoutVersion": 1,
        "distribution": "lumi",
        "version": VER,
        "target": target,
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

        bwrap_data = elf64(machine, ph_type) + VER.encode() + b"bwrap-resource"
        bwrap_digest = hashlib.sha256(bwrap_data).hexdigest()
        if digest_embed is None:
            digest_embed = bwrap_digest

        add("bin/codex", elf64(machine, ph_type) + VER.encode() + digest_embed.encode())
        add("bin/codex-code-mode-host", elf64(machine, ph_type) + VER.encode())
        add("codex-path/rg", b"rg")
        add("codex-resources/zsh/bin/zsh", b"zsh")
        add("codex-resources/bwrap", bwrap_data)
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


class X86LinuxValidationTest(unittest.TestCase):
    """x86_64-unknown-linux-musl: ELF EM_X86_64, no PT_INTERP, native run."""

    X86_TARGET = "x86_64-unknown-linux-musl"

    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.root = Path(self._temp.name)

    def tearDown(self) -> None:
        self._temp.cleanup()

    def _archive(self, machine: int, ph_type: int = 1) -> Path:
        archive = self.root / "pkg.tar.gz"
        make_linux_archive(archive, self.X86_TARGET, machine, ph_type)
        return archive

    def _run_native(self, archive: Path, returncode: int = 0, stdout: str = "") -> None:
        fake = _run_fake(returncode, stdout or f"codex {VER}\n")
        with mock.patch.object(
            lumi_shadow_validate_package, "subprocess"
        ) as fake_subprocess:
            fake_subprocess.run.return_value = fake
            validate_archive(
                archive,
                self.X86_TARGET,
                VER,
                run_native=True,
            )

    def test_x86_archive_passes_arch_static_and_native_run(self) -> None:
        self._run_native(self._archive(62))  # EM_X86_64

    def test_x86_archive_passes_without_native_run(self) -> None:
        validate_archive(self._archive(62), self.X86_TARGET, VER, run_native=False)

    def test_x86_wrong_machine_fails(self) -> None:
        with self.assertRaises(SystemExit):
            validate_archive(
                self._archive(183),  # EM_AARCH64 in an x86_64 package
                self.X86_TARGET,
                VER,
                run_native=False,
            )

    def test_x86_pt_interp_fails(self) -> None:
        with self.assertRaises(SystemExit):
            validate_archive(
                self._archive(62, ph_type=3),  # PT_INTERP
                self.X86_TARGET,
                VER,
                run_native=False,
            )

    def test_x86_native_run_rc_zero_matching_version_passes(self) -> None:
        self._run_native(self._archive(62), returncode=0, stdout=f"codex {VER}\n")

    def test_x86_native_run_wrong_version_fails(self) -> None:
        with self.assertRaises(SystemExit):
            self._run_native(
                self._archive(62),
                returncode=0,
                stdout="codex 9.9.9-other.1\n",
            )


class LinuxBwrapDigestTest(unittest.TestCase):
    """Linux packages must embed the packaged bwrap SHA-256 in bin/codex."""

    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.root = Path(self._temp.name)

    def tearDown(self) -> None:
        self._temp.cleanup()

    def _archive(
        self,
        target: str,
        machine: int,
        digest_embed: str | None = None,
    ) -> Path:
        archive = self.root / "pkg.tar.gz"
        make_linux_archive(
            archive,
            target,
            machine,
            digest_embed=digest_embed,
        )
        return archive

    def test_x86_matching_digest_passes(self) -> None:
        validate_archive(
            self._archive("x86_64-unknown-linux-musl", 62),
            "x86_64-unknown-linux-musl",
            VER,
            run_native=False,
        )

    def test_arm_matching_digest_passes(self) -> None:
        validate_archive(
            self._archive("aarch64-unknown-linux-musl", 183),
            "aarch64-unknown-linux-musl",
            VER,
            run_native=False,
        )

    def test_x86_digest_mismatch_fails(self) -> None:
        with self.assertRaises(SystemExit):
            validate_archive(
                self._archive(
                    "x86_64-unknown-linux-musl",
                    62,
                    digest_embed="0" * 64,
                ),
                "x86_64-unknown-linux-musl",
                VER,
                run_native=False,
            )

    def test_arm_digest_mismatch_fails(self) -> None:
        with self.assertRaises(SystemExit):
            validate_archive(
                self._archive(
                    "aarch64-unknown-linux-musl",
                    183,
                    digest_embed="f" * 64,
                ),
                "aarch64-unknown-linux-musl",
                VER,
                run_native=False,
            )

    def test_x86_digest_missing_fails(self) -> None:
        with self.assertRaises(SystemExit):
            validate_archive(
                self._archive(
                    "x86_64-unknown-linux-musl",
                    62,
                    digest_embed="",
                ),
                "x86_64-unknown-linux-musl",
                VER,
                run_native=False,
            )

    def test_fixture_embeds_digest_exactly_once(self) -> None:
        """Documented evidence: the real accepted x86 artifact contains the
        digest exactly once; the fixture mirrors that shape."""
        archive = self._archive("x86_64-unknown-linux-musl", 62)
        with tarfile.open(archive, "r:gz") as tf:
            codex = tf.extractfile("bin/codex").read()
            bwrap = tf.extractfile("codex-resources/bwrap").read()
        digest = hashlib.sha256(bwrap).hexdigest().encode("ascii")
        self.assertEqual(codex.count(digest), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
