#!/usr/bin/env python3
"""Resolve a manual source input to the exact commit for the shadow workflow.

Owned exclusively by .github/workflows/lumi-release-shadow-worker.yml; the
canonical lumi-release tag gate is not modified.

Allowed inputs:
  * an exact Lumi canary tag `rust-vX.Y.Z-lumi.N`; or
  * an exact 40-hex commit SHA (upper or lower case).

The resolved commit must exist and be an ancestor of the origin reference
(origin/main by default). For tags, the tag version must equal the workspace
version in codex-rs/Cargo.toml at that commit. PR refs, branch names, prefixed
refs, short SHAs, and any other input are rejected as ambiguous.

The workflow definition itself is gated too: when --main-head is given, the
workflow commit must equal the current origin reference, so workflow_dispatch
is accepted only from current main (the workflow YAML separately requires
github.ref == refs/heads/main).

The resolved commit is printed to stdout as lowercase 40-hex text; diagnostics
go to stderr and the exit code is nonzero on rejection.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


TAG_PATTERN = re.compile(r"^rust-v[0-9]+\.[0-9]+\.[0-9]+-lumi\.[0-9]+$")
SHA_PATTERN = re.compile(r"^[0-9a-fA-F]{40}$")
VERSION_PATTERN = re.compile(r'^version\s*=\s*"([^"]+)"', re.MULTILINE)


class GateError(Exception):
    """A source input or workflow ref was rejected by the gate."""


def run_git(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
        check=False,
    )


def _resolve_ref(repo: Path, source: str) -> tuple[str, str]:
    """Return (mode, 40-hex sha) for source, raising GateError on rejection."""
    if not source:
        raise GateError("source input is empty")

    if TAG_PATTERN.match(source):
        # refs/tags names are exact and unambiguous; ^{commit} peels annotated
        # tags to the commit they point at.
        mode = "tag"
        verify_ref = f"refs/tags/{source}^{{commit}}"
    elif SHA_PATTERN.match(source):
        mode = "sha"
        verify_ref = f"{source.lower()}^{{commit}}"
    else:
        raise GateError(
            "source must be an exact Lumi canary tag (rust-vX.Y.Z-lumi.N) or "
            "an exact 40-hex commit SHA; got an ambiguous ref or invalid input"
        )

    resolved = run_git(repo, "rev-parse", "--verify", "--quiet", verify_ref)
    if resolved.returncode != 0:
        raise GateError(
            f"source {source!r} ({mode}) does not resolve to an existing commit"
        )
    sha = resolved.stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise GateError(f"resolved value is not a 40-hex commit SHA: {sha!r}")
    return mode, sha


def _check_ancestor(repo: Path, sha: str, origin_ref: str) -> None:
    origin_check = run_git(repo, "rev-parse", "--verify", "--quiet", origin_ref)
    if origin_check.returncode != 0:
        raise GateError(f"origin reference {origin_ref!r} is not available")
    ancestor = run_git(repo, "merge-base", "--is-ancestor", sha, origin_ref)
    if ancestor.returncode != 0:
        raise GateError(f"resolved commit {sha} is not an ancestor of {origin_ref}")


def _check_tag_version(repo: Path, source: str, sha: str) -> None:
    tag_version = source[len("rust-v") :]
    cargo = run_git(repo, "show", f"{sha}:codex-rs/Cargo.toml")
    if cargo.returncode != 0:
        raise GateError(f"codex-rs/Cargo.toml not found at {sha}")
    match = VERSION_PATTERN.search(cargo.stdout)
    if match is None or match.group(1) != tag_version:
        found = match.group(1) if match else "<missing>"
        raise GateError(
            f"tag version {tag_version!r} does not match codex-rs/Cargo.toml "
            f"version {found!r} at {sha}"
        )


def resolve_commit(repo: Path, source: str, origin_ref: str) -> str:
    """Return the exact 40-hex commit for ``source`` or raise GateError."""
    mode, sha = _resolve_ref(repo, source)
    _check_ancestor(repo, sha, origin_ref)
    if mode == "tag":
        _check_tag_version(repo, source, sha)
    return sha


def require_main_head(repo: Path, head_sha: str, origin_ref: str) -> str:
    """Return the origin_ref commit, requiring the workflow head to equal it."""
    origin_check = run_git(repo, "rev-parse", "--verify", "--quiet", origin_ref)
    if origin_check.returncode != 0:
        raise GateError(f"origin reference {origin_ref!r} is not available")
    main_sha = origin_check.stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{40}", main_sha):
        raise GateError(f"origin reference resolves to: {main_sha!r}")
    if head_sha.strip().lower() != main_sha:
        raise GateError(
            f"workflow commit {head_sha.strip()} is not current {origin_ref} ({main_sha})"
        )
    return main_sha


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Resolve a shadow-workflow source input to an exact commit SHA "
            "that is an ancestor of the origin reference."
        )
    )
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--origin-ref", default="origin/main")
    parser.add_argument(
        "--main-head",
        help="workflow HEAD; must equal the origin reference commit",
    )
    args = parser.parse_args()

    try:
        if args.main_head:
            require_main_head(args.repo, args.main_head, args.origin_ref)
        sha = resolve_commit(args.repo, args.source, args.origin_ref)
    except GateError as error:
        print(f"gate rejected source {args.source!r}: {error}", file=sys.stderr)
        return 1

    print(sha)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
