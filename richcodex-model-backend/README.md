# RichCodex model backend

Internal, supervised model-plane component for RichCodex. It is packaged beside
`codex-app-server`; it is not an independently installed OpenCodex service or a
user-facing control plane.

The current slice implements only the bounded stdio lifecycle handshake. The
frozen OpenCodex kernel provenance is mirrored by
`codex-rs/app-server/richcodex-kernel.lock.json`; selected upstream modules will
enter here deliberately behind RichCodex-owned interfaces.
