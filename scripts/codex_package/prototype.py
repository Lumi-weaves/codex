"""Profile-driven exports of locally runnable Codex prototypes."""

import json
import re
import shutil
import subprocess
import sys
import tempfile
import uuid
import warnings
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from .targets import PACKAGE_VARIANTS
from .targets import REPO_ROOT
from .targets import TARGET_SPECS
from .targets import native_target


PROFILE_SCHEMA_VERSION = 1
PROFILE_NAME_PATTERN = re.compile(r"^[a-z0-9][a-z0-9-]*$")
PROFILE_KEYS = {
    "schemaVersion",
    "name",
    "variant",
    "target",
    "cargoProfile",
}
DEFAULT_PROFILE = (
    REPO_ROOT / "scripts" / "codex_package" / "profiles" / "lumi-local.json"
)
DEFAULT_OUTPUT_ROOT = REPO_ROOT / "codex-rs" / "target" / "prototypes"
PACKAGE_BUILDER = REPO_ROOT / "scripts" / "build_codex_package.py"


@dataclass(frozen=True)
class PrototypeProfile:
    name: str
    variant: str
    target: str
    cargo_profile: str

    def resolved_target(self) -> str:
        return native_target() if self.target == "native" else self.target


def load_profile(path: Path) -> PrototypeProfile:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise RuntimeError(f"Prototype profile does not exist: {path}") from error
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"Prototype profile is not valid JSON: {path}: {error}"
        ) from error

    if not isinstance(payload, dict):
        raise RuntimeError(f"Prototype profile must contain a JSON object: {path}")

    unknown_keys = sorted(set(payload) - PROFILE_KEYS)
    if unknown_keys:
        raise RuntimeError(
            f"Prototype profile contains unknown fields: {', '.join(unknown_keys)}"
        )

    schema_version = payload.get("schemaVersion")
    if schema_version != PROFILE_SCHEMA_VERSION:
        raise RuntimeError(
            "Unsupported prototype profile schemaVersion: "
            f"expected {PROFILE_SCHEMA_VERSION}, got {schema_version!r}"
        )

    name = required_string(payload, "name")
    if PROFILE_NAME_PATTERN.fullmatch(name) is None:
        raise RuntimeError(
            "Prototype profile name must contain only lowercase letters, digits, and hyphens"
        )

    variant = required_string(payload, "variant")
    if variant not in PACKAGE_VARIANTS:
        raise RuntimeError(f"Unsupported prototype package variant: {variant}")

    target = required_string(payload, "target")
    if target != "native" and target not in TARGET_SPECS:
        raise RuntimeError(f"Unsupported prototype target: {target}")

    cargo_profile = required_string(payload, "cargoProfile")
    return PrototypeProfile(
        name=name,
        variant=variant,
        target=target,
        cargo_profile=cargo_profile,
    )


def required_string(payload: dict[str, object], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str) or not value:
        raise RuntimeError(
            f"Prototype profile field {key!r} must be a non-empty string"
        )
    return value


def resolve_profile_path(value: str | None) -> Path:
    if value is None:
        return DEFAULT_PROFILE

    candidate = Path(value)
    if candidate.name == value and candidate.suffix == "":
        return DEFAULT_PROFILE.parent / f"{value}.json"
    return candidate.resolve()


def builder_command(profile: PrototypeProfile, package_dir: Path) -> list[str]:
    return [
        sys.executable,
        str(PACKAGE_BUILDER),
        "--target",
        profile.resolved_target(),
        "--variant",
        profile.variant,
        "--cargo-profile",
        profile.cargo_profile,
        "--package-dir",
        str(package_dir),
    ]


def prototype_entrypoint(profile: PrototypeProfile, package_dir: Path) -> Path:
    spec = TARGET_SPECS[profile.resolved_target()]
    variant = PACKAGE_VARIANTS[profile.variant]
    return package_dir / "bin" / variant.entrypoint_name(spec)


def export_profile(
    profile: PrototypeProfile,
    output_root: Path,
    *,
    run: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> Path:
    output_root = output_root.resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    destination = output_root / profile.name
    staging = Path(
        tempfile.mkdtemp(prefix=f".{profile.name}.staging-", dir=output_root)
    )

    try:
        run(builder_command(profile, staging), cwd=REPO_ROOT, check=True)
        add_prototype_metadata(staging, profile)
        replace_directory(staging, destination)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise

    return destination


def add_prototype_metadata(package_dir: Path, profile: PrototypeProfile) -> None:
    metadata_path = package_dir / "codex-package.json"
    try:
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise RuntimeError(
            f"Package builder did not create metadata: {metadata_path}"
        ) from error

    metadata["prototype"] = {
        "profile": profile.name,
        **source_provenance(),
    }
    metadata_path.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")


def source_provenance() -> dict[str, object]:
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    dirty = bool(
        subprocess.run(
            ["git", "status", "--short"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    )
    return {"sourceRevision": revision, "sourceDirty": dirty}


def replace_directory(staging: Path, destination: Path) -> None:
    backup = destination.with_name(f".{destination.name}.previous-{uuid.uuid4().hex}")
    had_destination = destination.exists() or destination.is_symlink()
    if had_destination:
        if destination.is_symlink() or not destination.is_dir():
            raise RuntimeError(
                f"Refusing to replace non-directory prototype path: {destination}"
            )
        destination.rename(backup)

    try:
        staging.rename(destination)
    except BaseException:
        if had_destination:
            backup.rename(destination)
        raise
    else:
        if had_destination:
            try:
                shutil.rmtree(backup)
            except OSError as error:
                warnings.warn(
                    f"Published {destination}, but could not remove prior prototype "
                    f"at {backup}: {error}",
                    stacklevel=2,
                )
