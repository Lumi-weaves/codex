---
name: build-codex-prototype
description: Export, verify, and hand off a locally runnable Codex prototype from a checked-in package profile. Use when someone wants a fresh local candidate binary, needs to test worktree changes in Codex Desktop or the CLI, asks how to reproduce a prototype build, or adds a new local prototype profile.
---

# Build Codex Prototype

Use the repository's profile exporter instead of reconstructing Cargo, V8, or
package-layout commands by hand.

## Export

From the repository root, export the default local profile:

```sh
just export-prototype
```

To select another checked-in profile from
`scripts/codex_package/profiles/<name>.json`:

```sh
just export-prototype <name>
```

The exporter builds in a staging directory, preserves the previous runnable
candidate if the build fails, and publishes the successful package at
`codex-rs/target/prototypes/<profile-name>`. Treat the absolute `Run ...` path
printed by the command as the prototype entrypoint.

Do not separately build or copy `codex-code-mode-host`. The canonical package
builder places it beside the entrypoint and resolves the checked, cached V8
artifacts required by local source builds.

## Verify and Hand Off

1. Confirm the export command exited successfully.
2. Read `codex-package.json` in the exported directory. Report the profile,
   target, source revision, and whether the source tree was dirty.
3. Resolve the entrypoint from the metadata's `entrypoint` field rather than
   assuming a variant or platform suffix. Confirm it and the adjacent
   `bin/codex-code-mode-host[.exe]` are executable. On Linux, also confirm
   `codex-resources/bwrap` is executable.
4. Run the resolved entrypoint with `--version` as the cheap smoke check unless
   the user asked for a more specific scenario.
5. Give the user the absolute entrypoint path and the exact experiment prompt
   or launch command relevant to the feature under test.

Do not install over an official Codex binary or mutate shell profiles for a
prototype handoff. The exported package is intentionally isolated.

## Add or Change a Profile

Keep profiles declarative and small. Use schema version 1 with these fields:

```json
{
  "schemaVersion": 1,
  "name": "example-local",
  "variant": "codex",
  "target": "native",
  "cargoProfile": "dev-small"
}
```

Prefer `native` for a prototype intended to run on the build host. On Linux it
selects GNU rather than the musl release target. Use an explicit supported Rust
target only when the artifact is meant for another known host. Keep build
semantics in `scripts/codex_package/`; do not add executable build logic to the
skill.

When changing the profile contract or exporter, run:

```sh
python3 -m unittest discover -s scripts/codex_package -p 'test_*.py'
```
