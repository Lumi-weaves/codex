#!/usr/bin/env python3

"""Classify a commit range for Lumi's path-gated CI contract."""

import argparse
import subprocess
from dataclasses import dataclass
from fnmatch import fnmatchcase
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RUST_PATTERNS = (
    "codex-rs/**",
    "bazel/**",
    "patches/**",
    "third_party/**",
    "tools/argument-comment-lint/**",
    "MODULE.bazel",
    "MODULE.bazel.lock",
    "defs.bzl",
    "justfile",
    "rbe.bzl",
    "rust-toolchain.toml",
    "workspace_root_test_launcher.*",
)
SDK_PATTERNS = (
    "sdk/**",
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
)
WEB_PATTERNS = (
    "lumi-web/**",
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
)
DISTRIBUTION_PATTERNS = (
    "scripts/codex_package/**",
    "scripts/install/**",
    "scripts/release/**",
    "scripts/build_codex_package.py",
    ".github/scripts/build-codex-package-archive.sh",
    ".github/workflows/lumi-release.yml",
    ".github/workflows/lumi-release-shadow-worker.yml",
)


@dataclass(frozen=True)
class Areas:
    rust: bool = False
    sdk: bool = False
    web: bool = False
    distribution: bool = False


def matches(path: str, patterns: tuple[str, ...]) -> bool:
    return any(fnmatchcase(path, pattern) for pattern in patterns)


def classify(paths: set[str], *, force: bool = False) -> Areas:
    if force or any(path.startswith(".github/") for path in paths):
        return Areas(rust=True, sdk=True, web=True, distribution=True)

    return Areas(
        rust=any(matches(path, RUST_PATTERNS) for path in paths),
        sdk=any(matches(path, SDK_PATTERNS) for path in paths),
        web=any(matches(path, WEB_PATTERNS) for path in paths),
        distribution=any(matches(path, DISTRIBUTION_PATTERNS) for path in paths),
    )


def changed_files(base: str, head: str, *, root: Path = ROOT) -> set[str]:
    output = subprocess.check_output(
        ["git", "diff", "--name-only", "--no-renames", f"{base}...{head}"],
        cwd=root,
        text=True,
    )
    return set(output.splitlines())


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()

    paths = changed_files(args.base, args.head)
    areas = classify(paths, force=args.force)
    print(f"rust={str(areas.rust).lower()}")
    print(f"sdk={str(areas.sdk).lower()}")
    print(f"web={str(areas.web).lower()}")
    print(f"distribution={str(areas.distribution).lower()}")


if __name__ == "__main__":
    main()
