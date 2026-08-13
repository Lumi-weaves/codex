# RichCodex model backend

Internal, supervised model-plane component for RichCodex. It is packaged beside
`codex-app-server`; it is not an independently installed OpenCodex service or a
user-facing control plane.

The backend exposes a bounded stdio lifecycle and a correlated private control
protocol. Normal Responses traffic uses a loopback-only HTTP data plane guarded
by a fresh process capability shared only with the supervising app-server. The
backend resolves stable model tags, refreshes provider credentials, applies
ordered account fallback, and streams the selected upstream response without
publishing credentials into Codex configuration. The frozen OpenCodex kernel provenance is mirrored by
`codex-rs/app-server/richcodex-kernel.lock.json`; selected upstream modules will
enter here deliberately behind RichCodex-owned interfaces.

`kernel-selection.json` records the exact OpenCodex symbols and invariants
adapted into the current composition. Explicitly selected Codex-auth imports
live in a RichCodex-owned provider store. The backend never discovers or
borrows the native Codex or OpenCodex homes, and it exposes only opaque account
handles, safe route state, and user labels over the private control protocol.
