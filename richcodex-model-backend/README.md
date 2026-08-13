# RichCodex model backend

Internal, supervised model-plane component for RichCodex. It is packaged beside
`codex-app-server`; it is not an independently installed OpenCodex service or a
user-facing control plane.

The backend exposes a bounded stdio lifecycle and a correlated private control
protocol. The frozen OpenCodex kernel provenance is mirrored by
`codex-rs/app-server/richcodex-kernel.lock.json`; selected upstream modules will
enter here deliberately behind RichCodex-owned interfaces.

`kernel-selection.json` records the exact OpenCodex symbols and invariants
adapted into the current composition. Slice 1A adds an explicit, user-selected
Codex-auth import into a RichCodex-owned provider store. It never discovers or
borrows the native Codex or OpenCodex homes, and it exposes only opaque account
handles and user labels over the private protocol.
