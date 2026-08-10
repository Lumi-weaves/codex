#!/usr/bin/env python3
"""Fetch ripgrep and codex-zsh for a shadow target with curl, not urllib.

Owned exclusively by .github/workflows/lumi-release-shadow-worker.yml; the
canonical package builder and its DotSlash modules are not modified.

The canonical builder resolves rg/zsh through
scripts/codex_package/dotslash.py, whose urllib download proved unreliable
and proxy-ignoring on the real macOS host. This helper keeps the canonical
manifest parsing, exact size + SHA-256 verification, and safe single-member
extraction from scripts/codex_package/dotslash.py, but downloads with curl
under bounded retry/connect/max-time limits (the guest proxy intermittently
produced 502/TLS resets). It is shipped from the workflow commit
(shadow-tools), so it can operate while building an older gated source SHA
that may predate these helpers or the canonical modules.

Resolved executables are emitted as GITHUB_ENV-style lines on stdout:

    LUMI_SHADOW_RG_BIN=<path>
    LUMI_SHADOW_ZSH_BIN=<path>       (empty when the platform has no zsh)

All diagnostics go to stderr; stdout carries only the env lines so callers
may append it to GITHUB_ENV directly.
"""

from __future__ import annotations

import argparse
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path

# Bounded download contract: finite retries with finite timeouts. Env
# overrides exist for the mock tests only; the workflow never sets them.
CURL_RETRIES = int(os.environ.get("LUMI_SHADOW_CURL_RETRIES", "5"))
CURL_RETRY_DELAY = int(os.environ.get("LUMI_SHADOW_CURL_RETRY_DELAY", "2"))
CURL_CONNECT_TIMEOUT = int(
    os.environ.get("LUMI_SHADOW_CURL_CONNECT_TIMEOUT", "20")
)
CURL_MAX_TIME = int(os.environ.get("LUMI_SHADOW_CURL_MAX_TIME", "300"))
CURL_BIN = os.environ.get("LUMI_SHADOW_CURL", "curl")

ZSH_MANIFEST_DEFAULT_URL = (
    "https://github.com/openai/codex/releases/download/"
    "codex-zsh-v0.1.0/codex-zsh"
)


def _curl_args(url: str, output: Path) -> list[str]:
    for name, value, minimum, maximum in (
        ("retries", CURL_RETRIES, 0, 20),
        ("retry-delay", CURL_RETRY_DELAY, 0, 60),
        ("connect-timeout", CURL_CONNECT_TIMEOUT, 1, 120),
        ("max-time", CURL_MAX_TIME, 1, 3600),
    ):
        if not minimum <= value <= maximum:
            raise SystemExit(
                f"lumi_shadow_fetch_dotslash: {name} {value} out of bounded "
                f"range [{minimum}, {maximum}]"
            )
    return [
        CURL_BIN,
        "-fsSL",
        "--retry",
        str(CURL_RETRIES),
        "--retry-delay",
        str(CURL_RETRY_DELAY),
        "--retry-all-errors",
        "--connect-timeout",
        str(CURL_CONNECT_TIMEOUT),
        "--max-time",
        str(CURL_MAX_TIME),
        "-o",
        str(output),
        url,
    ]


def download_with_curl(url: str, dest: Path) -> None:
    """Download ``url`` to ``dest`` atomically with bounded curl behavior."""
    if shutil.which(CURL_BIN) is None:
        raise SystemExit(f"lumi_shadow_fetch_dotslash: curl not found: {CURL_BIN}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    temp_path = dest.with_suffix(f"{dest.suffix}.tmp")
    temp_path.unlink(missing_ok=True)
    try:
        try:
            subprocess.run(
                _curl_args(url, temp_path),
                check=True,
            )
        except subprocess.CalledProcessError as error:
            raise RuntimeError(
                f"download failed after bounded retries ({CURL_RETRIES}): "
                f"{url} (curl exit {error.returncode})"
            ) from error
        temp_path.replace(dest)
    finally:
        temp_path.unlink(missing_ok=True)


def fetch_artifact(
    spec,
    *,
    manifest_path: Path,
    artifact_label: str,
    dest_name: str,
    output_dir: Path,
    missing_ok: bool,
) -> Path | None:
    """Fetch one DotSlash artifact with curl; verify and extract canonically."""
    from codex_package.dotslash import (
        archive_filename,
        archive_is_valid,
        artifact_for_target,
        extract_archive_member,
        verify_archive,
    )

    artifact = artifact_for_target(
        spec,
        manifest_path,
        artifact_label=artifact_label,
        missing_ok=missing_ok,
    )
    if artifact is None:
        return None

    archive_path = output_dir / archive_filename(artifact.url)
    if not archive_is_valid(archive_path, artifact, artifact_label):
        download_with_curl(artifact.url, archive_path)
        try:
            verify_archive(archive_path, artifact, artifact_label)
        except RuntimeError:
            # Canonical behavior: drop only the exact invalid download.
            archive_path.unlink(missing_ok=True)
            raise

    dest = output_dir / dest_name
    extract_archive_member(archive_path, artifact, dest, artifact_label)
    mode = dest.stat().st_mode
    dest.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return dest


def run(args: argparse.Namespace) -> int:
    tools_root = Path(args.tools_root).resolve()
    scripts_dir = tools_root / "scripts"
    if not scripts_dir.is_dir():
        print(
            f"lumi_shadow_fetch_dotslash: tools root has no scripts/: {tools_root}",
            file=sys.stderr,
        )
        return 1
    # Canonical DotSlash modules come from the workflow commit (shadow-tools),
    # not from the possibly-older gated tree.
    sys.path.insert(0, str(scripts_dir))

    from codex_package.targets import TARGET_SPECS

    if args.target not in TARGET_SPECS:
        supported = ", ".join(sorted(TARGET_SPECS))
        print(
            f"lumi_shadow_fetch_dotslash: unsupported target {args.target!r}; "
            f"supported: {supported}",
            file=sys.stderr,
        )
        return 1
    spec = TARGET_SPECS[args.target]

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    rg_manifest = Path(args.rg_manifest)
    if not rg_manifest.is_file():
        print(
            f"lumi_shadow_fetch_dotslash: rg manifest not found: {rg_manifest}",
            file=sys.stderr,
        )
        return 1

    try:
        rg_bin = fetch_artifact(
            spec,
            manifest_path=rg_manifest,
            artifact_label="ripgrep",
            dest_name=spec.rg_name,
            output_dir=output_dir,
            missing_ok=False,
        )

        if args.zsh_manifest_path:
            zsh_manifest = Path(args.zsh_manifest_path)
        else:
            zsh_manifest = output_dir / "codex-zsh-manifest"
            download_with_curl(args.zsh_manifest_url, zsh_manifest)
        zsh_bin = fetch_artifact(
            spec,
            manifest_path=zsh_manifest,
            artifact_label="codex-zsh",
            dest_name="zsh",
            output_dir=output_dir,
            missing_ok=True,
        )
    except RuntimeError as error:
        print(f"lumi_shadow_fetch_dotslash: {error}", file=sys.stderr)
        return 1

    lines = [
        f"LUMI_SHADOW_RG_BIN={rg_bin}",
        f"LUMI_SHADOW_ZSH_BIN={zsh_bin if zsh_bin is not None else ''}",
    ]
    for line in lines:
        print(line)
    if args.output_file:
        Path(args.output_file).write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Fetch the shadow target's ripgrep and codex-zsh from their "
            "canonical DotSlash manifests using bounded curl downloads."
        )
    )
    parser.add_argument("--target", required=True)
    parser.add_argument("--tools-root", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument(
        "--rg-manifest",
        help="DotSlash manifest for ripgrep (default: <tools-root>/scripts/codex_package/rg)",
    )
    parser.add_argument(
        "--zsh-manifest-url",
        default=ZSH_MANIFEST_DEFAULT_URL,
        help="Canonical codex-zsh DotSlash manifest URL",
    )
    parser.add_argument(
        "--zsh-manifest-path",
        help="Local codex-zsh DotSlash manifest instead of a URL fetch",
    )
    parser.add_argument(
        "--output-file",
        help="Also write the env lines to this file",
    )
    args = parser.parse_args()
    if args.rg_manifest is None:
        args.rg_manifest = str(
            Path(args.tools_root) / "scripts" / "codex_package" / "rg"
        )
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
