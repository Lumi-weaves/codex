#!/usr/bin/env python3
"""Export a locally runnable Codex package from a checked-in profile."""

import argparse
from pathlib import Path
import sys


sys.path.insert(0, str(Path(__file__).resolve().parent))

from codex_package.prototype import DEFAULT_OUTPUT_ROOT
from codex_package.prototype import export_profile
from codex_package.prototype import load_profile
from codex_package.prototype import prototype_entrypoint
from codex_package.prototype import resolve_profile_path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Export a locally runnable Codex prototype package.",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument(
        "--profile",
        help=(
            "Checked-in profile name or JSON path. A bare name resolves under "
            "scripts/codex_package/profiles."
        ),
    )
    parser.add_argument(
        "--output-root",
        type=Path,
        default=DEFAULT_OUTPUT_ROOT,
        help="Directory that owns the stable per-profile prototype directories.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    profile_path = resolve_profile_path(args.profile)
    profile = load_profile(profile_path)
    package_dir = export_profile(profile, args.output_root)
    entrypoint = prototype_entrypoint(profile, package_dir)
    print(f"Exported prototype profile {profile.name!r} to {package_dir}")
    print(f"Run {entrypoint}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
