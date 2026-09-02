# RichCodex model backend

Internal, supervised model-plane component for RichCodex. It is packaged beside
`codex-app-server`; it is not an independently installed OpenCodex service or a
user-facing control plane.

The backend exposes a bounded stdio lifecycle and a correlated private control
protocol. Normal Responses traffic uses a loopback-only HTTP data plane guarded
by a fresh process capability shared only with the supervising app-server. The
backend resolves stable model tags, refreshes provider credentials, applies
ordered account fallback, and streams the selected upstream response without
publishing credentials into Codex configuration. For an OpenAI request carrying
a stable prompt-cache lineage, the backend opportunistically retains an
upstream Responses WebSocket and sends only an exactly verified suffix with
`previous_response_id`. Connection loss, a route change, a non-prefix request,
or an unavailable WebSocket invalidates that optional handle; connection setup
falls back immediately and the next safe attempt always has the complete HTTP
request available. The connection is never conversation authority. The frozen OpenCodex kernel
provenance is mirrored by `codex-rs/app-server/richcodex-kernel.lock.json`;
selected upstream modules will enter here deliberately behind RichCodex-owned
interfaces.

Outbound auth, HTTPS/SSE, and WebSocket connection attempts share one
destination-aware network route resolver. On macOS it follows the current
System Configuration HTTP/HTTPS route with a bounded cache; a later physical
retry therefore observes proxy changes without restarting the backend. If
platform discovery is unavailable, explicit proxy environment variables are
the fallback. Proxy URLs remain transport-private and are never written to the
stdio protocol, model-plane store, or diagnostics.

`kernel-selection.json` records the exact OpenCodex symbols and invariants
adapted into the current composition. Explicitly selected Codex-auth imports
live in a RichCodex-owned provider store. The backend never discovers or
borrows the native Codex or OpenCodex homes, and it exposes only opaque account
handles, safe route state, and user labels over the private control protocol.
