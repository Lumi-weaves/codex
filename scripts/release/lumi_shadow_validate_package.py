#!/usr/bin/env python3
"""Static (and optionally native) validation of Lumi package archives.

Used by the manual installer and shadow workflows. The canonical lumi-release
workflow validates its release assets with an inline Python validator that
these workflows must not modify; this helper mirrors the same checks for their
three supported targets:

  * safe tar members: no absolute paths, no `..` components, no duplicate
    names, regular files and directories only;
  * canonical eight-field codex-package.json metadata;
  * required regular executables (including the bwrap resource for Linux);
  * correct binary architecture (arm64 Mach-O / aarch64 or x86_64 ELF) for
    the entrypoint, code-mode host, and (Linux) bwrap;
  * Linux binaries are fully static: no PT_INTERP (dynamic interpreter);
  * the packaged bwrap SHA-256 (64-byte lowercase ASCII hex) is embedded in
    bin/codex for Linux targets;
  * the expected version embedded in the entrypoint;
  * optionally, native execution of `bin/codex --version`.

Supported targets: aarch64-apple-darwin, aarch64-unknown-linux-musl, and
x86_64-unknown-linux-musl.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile


MACH_O_MAGIC_64 = b"\xcf\xfa\xed\xfe"
ELF_MAGIC = b"\x7fELF"
CPU_TYPE_ARM64 = 0x0100000C
EM_AARCH64 = 183
EM_X86_64 = 62
PT_INTERP = 3

SUPPORTED_TARGETS = (
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
)


def check_archive_members(archive: pathlib.Path) -> None:
    seen: set[str] = set()
    with tarfile.open(archive, "r:gz") as tf:
        for member in tf.getmembers():
            name = member.name
            if (
                name == ""
                or name.startswith("/")
                or any(part == ".." for part in name.split("/"))
            ):
                raise SystemExit(f"Unsafe member name in {archive.name}: {name!r}")
            if not member.isfile() and not member.isdir():
                raise SystemExit(f"Unsafe member type in {archive.name}: {name!r}")
            if name in seen:
                raise SystemExit(f"Duplicate member in {archive.name}: {name!r}")
            seen.add(name)


def require_regular_executable(root: pathlib.Path, relative: str) -> None:
    path = root / relative
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"Missing or invalid package file: {relative}")
    if not path.stat().st_mode & stat.S_IXUSR:
        raise SystemExit(f"Package file is not executable: {relative}")


def check_arch(path: pathlib.Path, target: str) -> None:
    data = path.read_bytes()
    if target.endswith("apple-darwin"):
        if data[:4] != MACH_O_MAGIC_64:
            raise SystemExit(f"{path} is not a 64-bit Mach-O binary")
        cputype = struct.unpack("<I", data[4:8])[0]
        if cputype != CPU_TYPE_ARM64:
            raise SystemExit(f"{path} has wrong Mach-O cputype {cputype:#x}")
    else:
        if data[:4] != ELF_MAGIC or data[4] != 2:
            raise SystemExit(f"{path} is not a 64-bit ELF binary")
        machine = struct.unpack("<H", data[18:20])[0]
        expected_machine = EM_AARCH64 if target.startswith("aarch64") else EM_X86_64
        if machine != expected_machine:
            raise SystemExit(
                f"{path} has wrong ELF machine {machine} "
                f"(expected {expected_machine} for {target})"
            )


def check_no_pt_interp(path: pathlib.Path) -> None:
    """Fail if the ELF binary carries a PT_INTERP dynamic interpreter."""
    data = path.read_bytes()
    if len(data) < 64 or data[:4] != ELF_MAGIC or data[4] != 2:
        raise SystemExit(f"{path} is not a 64-bit ELF binary")
    e_phoff = struct.unpack_from("<Q", data, 0x20)[0]
    e_phentsize = struct.unpack_from("<H", data, 0x36)[0]
    e_phnum = struct.unpack_from("<H", data, 0x38)[0]
    if e_phentsize == 0:
        raise SystemExit(f"{path} has an invalid program header table")
    for i in range(e_phnum):
        offset = e_phoff + i * e_phentsize
        if offset + 8 > len(data):
            raise SystemExit(f"{path} has a truncated program header table")
        p_type = struct.unpack_from("<I", data, offset)[0]
        if p_type == PT_INTERP:
            raise SystemExit(
                f"{path} has a PT_INTERP (dynamic interpreter); "
                "expected a fully static binary"
            )


def validate_archive(
    archive: pathlib.Path,
    target: str,
    expected_version: str,
    *,
    run_native: bool,
) -> None:
    if target not in SUPPORTED_TARGETS:
        raise SystemExit(f"Unsupported target for shadow validation: {target}")

    check_archive_members(archive)

    with tempfile.TemporaryDirectory(prefix="lumi-shadow-validate-") as temp_dir:
        extract_dir = pathlib.Path(temp_dir) / target
        extract_dir.mkdir(parents=True)
        with tarfile.open(archive, "r:gz") as tf:
            tf.extractall(extract_dir, filter="data")

        pkgjson = extract_dir / "codex-package.json"
        if not pkgjson.is_file() or pkgjson.is_symlink():
            raise SystemExit(
                f"Missing package metadata in {target}: codex-package.json"
            )
        metadata = json.loads(pkgjson.read_text(encoding="utf-8"))
        expected = {
            "layoutVersion": 1,
            "distribution": "lumi",
            "version": expected_version,
            "target": target,
            "variant": "codex",
            "entrypoint": "bin/codex",
            "resourcesDir": "codex-resources",
            "pathDir": "codex-path",
        }
        if metadata != expected:
            raise SystemExit(
                "Invalid package metadata: expected the canonical eight-field "
                f"schema {expected!r}, got {metadata!r}"
            )

        require_regular_executable(extract_dir, "bin/codex")
        require_regular_executable(extract_dir, "bin/codex-code-mode-host")
        require_regular_executable(extract_dir, "codex-path/rg")
        require_regular_executable(extract_dir, "codex-resources/zsh/bin/zsh")

        entrypoint = extract_dir / "bin/codex"
        code_mode_host = extract_dir / "bin/codex-code-mode-host"
        check_arch(entrypoint, target)
        check_arch(code_mode_host, target)

        if target.endswith("unknown-linux-musl"):
            bwrap = extract_dir / "codex-resources/bwrap"
            require_regular_executable(extract_dir, "codex-resources/bwrap")
            check_arch(bwrap, target)
            check_no_pt_interp(entrypoint)
            check_no_pt_interp(code_mode_host)
            check_no_pt_interp(bwrap)
            # Codex embeds the digest of the exact packaged bwrap bytes at
            # build time and verifies the bundled resource against it; the
            # real accepted x86 artifact contains the 64-byte lowercase
            # hex digest exactly once.
            bwrap_digest = hashlib.sha256(bwrap.read_bytes()).hexdigest()
            if bwrap_digest.encode("ascii") not in entrypoint.read_bytes():
                raise SystemExit(
                    f"bin/codex in {target} does not embed the packaged "
                    f"bwrap sha256 {bwrap_digest}"
                )

        if expected_version.encode("utf-8") not in entrypoint.read_bytes():
            raise SystemExit(
                f"bin/codex in {target} does not embed version {expected_version}"
            )

        if run_native:
            try:
                result = subprocess.run(
                    [str(entrypoint), "--version"],
                    capture_output=True,
                    text=True,
                    check=False,
                )
            except OSError as error:
                raise SystemExit(
                    f"Could not execute bin/codex natively: {error}"
                ) from error
            if result.returncode != 0:
                raise SystemExit(
                    f"Native bin/codex --version exited {result.returncode}: "
                    f"{result.stderr.strip()}"
                )
            stdout = result.stdout or ""
            parsed = stdout.strip().rsplit(" ", 1)[-1] if stdout.strip() else ""
            if parsed != expected_version:
                raise SystemExit(
                    f"Native bin/codex version {parsed!r} != expected "
                    f"{expected_version!r}"
                )

        print(f"✅  Validated canonical package for {target} ({expected_version})")


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Validate a Lumi Codex package archive against the "
            "canonical Lumi package checks."
        )
    )
    parser.add_argument("--archive", type=pathlib.Path, required=True)
    parser.add_argument("--target", required=True, choices=SUPPORTED_TARGETS)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument(
        "--run",
        action="store_true",
        help="additionally execute bin/codex --version natively",
    )
    args = parser.parse_args()

    if not args.archive.is_file():
        print(f"Archive not found: {args.archive}", file=sys.stderr)
        return 1

    try:
        validate_archive(
            args.archive,
            args.target,
            args.expected_version,
            run_native=args.run,
        )
    except SystemExit as error:
        if error.code:
            print(f"❌  {error}", file=sys.stderr)
            return 1
        raise
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
