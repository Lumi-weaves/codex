# Codex session observability field guide

## Status

This note is an engineering field guide for observing, diagnosing, and
reconstructing Codex sessions. It records repository evidence, current host
capabilities, and a preferred escalation path; it is not a promise that every
surface below is a stable public API.

Evidence labels used here:

- **Upstream/public fact**: documented by OpenAI or exposed by the app-server
  protocol.
- **Repository fact**: supported by source in this checkout, but not necessarily
  a stable external contract.
- **Lumi implementation**: present in the Lumi build or fork and subject to
  change until it is adopted upstream.
- **Host integration fact**: observed in Codex Desktop's agent tool surface;
  useful to us, but not a general shell or REST API.

Research snapshot:

- Date: 2026-08-12
- Checkout: `5d185cd755d0fa4189ea19ed67fb62276c678e1f`
- Installed build observed: `codex-cli 0.147.0-lumi.4`

## Executive conclusion

Do not begin by building another logging system. Codex already provides four
distinct observability layers:

1. **Thread and turn state** for ordinary supervision.
2. **Live protocol events** for clients that own an app-server connection or a
   `codex exec --json` process.
3. **Persistent rollout and structured runtime logs** for post-hoc or stall
   diagnosis.
4. **Opt-in rollout trace bundles** for causal reconstruction of model, tool,
   terminal, code-mode, compaction, and multi-agent behavior.

The default monitoring path should stay at layer 1. Escalate only when the
question requires more evidence.

## The observation model

"Active" is not the same as "making progress." A useful observer should keep
this tuple rather than one boolean:

```text
thread status
active flags
latest turn status
latest incomplete item kind and status
last meaningful event time
live background terminal count
latest warning or error
```

This separates four materially different conditions:

```text
working        active turn, recent events
blocked        waiting for approval or user input
waiting        turn finished, but a background terminal is still live
stalled        nominally active, but no meaningful event arrives inside policy
```

The first three are runtime facts. "Stalled" is a monitoring policy and must be
derived from elapsed time plus expected activity; Codex cannot report it as an
intrinsic state.

## Capability map

| Need | Preferred surface | Contract level | Live | Persisted |
| --- | --- | --- | --- | --- |
| List tasks and see coarse status | Desktop `list_threads` | Host integration | Yes | Summary |
| Inspect one task's current turn and recent items | Desktop `read_thread` | Host integration | Yes | Yes |
| Wait for completion or required attention | Desktop `wait_threads` | Host integration | Yes, event-driven | No |
| Read thread runtime status | app-server `thread/read` | Protocol | Yes on the owning daemon | Yes |
| Receive status changes | app-server `thread/status/changed` | Protocol | Yes | No |
| Stream turn, item, plan, diff, and tool progress | app-server notifications | Protocol | Yes | Partly |
| Stream a non-interactive run | `codex exec --json` | CLI | Yes | Caller decides |
| Read the saved rollout | `$CODEX_HOME/sessions/.../rollout-*.jsonl` | Internal persistence | Append-only while running | Yes |
| Tail DEBUG/TRACE rows by thread | `logs_2.sqlite` and `logs_client` | Repository/internal | Yes, polling | Yes |
| Reconstruct a causal rollout graph | `CODEX_ROLLOUT_TRACE_ROOT` and `trace-reduce` | Lumi implementation | Raw append while running | Yes |
| Export fleet telemetry | OpenTelemetry | Public configuration | Yes, batched | Exporter decides |

## 1. Ordinary supervision: Desktop host tools

**Host integration fact.** The current Codex Desktop agent surface exposes:

- `list_threads(limit)` — returns Codex and ChatGPT task summaries, backing
  kind, host, project context, and coarse status.
- `read_thread(threadId, hostId, ...)` — returns current thread status, recent
  turns, item summaries, errors, timestamps, and optionally truncated outputs.
- `wait_threads(targets, timeoutMs)` — waits until one target completes or
  needs attention. Commentary does not wake the wait; a timeout returns compact
  progress for every target.

Use this layer for an alarm, supervisor, dashboard, or companion agent. It is
already normalized across hosts and avoids exposing raw prompts and tool
payloads.

Important limitations:

- A task cannot call `wait_threads` on itself.
- These are Desktop host tools, not commands an arbitrary local process can
  invoke.
- A task reported as `active` still needs `read_thread` or app-server status
  flags to distinguish progress from blocking.

Recommended monitoring loop:

```text
list_threads
    -> select target and retain hostId
    -> read_thread for current status and recent items
    -> wait_threads with a bounded, meaningful deadline
    -> read_thread only after completion, attention, or suspected stall
```

Avoid frequent polling. `wait_threads` exists specifically to absorb uncertain
waits without producing a stream of unchanged snapshots.

## 2. Runtime truth: app-server

**Upstream/public fact.** App-server is the primary protocol for rich Codex
clients. Relevant methods include:

- `thread/list`
- `thread/read`
- `thread/loaded/list`
- `thread/turns/list` (experimental)
- `thread/items/list` (experimental)
- `thread/backgroundTerminals/list` (experimental)
- `thread/unsubscribe`

Relevant notifications include:

- `thread/started`
- `thread/status/changed`
- `thread/closed`
- `turn/started`
- `turn/completed`
- `turn/plan/updated`
- `turn/diff/updated`
- `item/started`
- `item/completed`
- item-specific deltas and approval requests
- `error` and `warning`

The protocol runtime status is:

```text
notLoaded
idle
systemError
active
```

In this checkout, an active thread can additionally carry:

```text
waitingOnApproval
waitingOnUserInput
waitingOnBackgroundTerminal
```

Source of truth:

- `codex-rs/app-server/README.md`
- `codex-rs/app-server/src/thread_status.rs`
- `codex-rs/app-server-protocol/src/protocol/v2/thread.rs`

Minimal read request:

```json
{ "method": "thread/read", "id": 22, "params": { "threadId": "thr_123", "includeTurns": true } }
```

Status transition notification:

```json
{
  "method": "thread/status/changed",
  "params": {
    "threadId": "thr_123",
    "status": {
      "type": "active",
      "activeFlags": ["waitingOnApproval"]
    }
  }
}
```

### Owning-daemon boundary

Runtime state belongs to the app-server process that has the thread loaded. A
new, independent app-server process can read persisted history but will usually
report the other process's live thread as `notLoaded`.

An external observer that needs live status should connect to the existing
managed daemon, for example through its Unix socket or an explicitly secured
transport. Do not expose a non-loopback unauthenticated WebSocket listener;
WebSocket transport is currently experimental.

`thread/read` is the safe read-only snapshot. Resuming a thread changes its
lifetime and subscribes the connection, so do not use `thread/resume` merely to
inspect it.

## 3. Runs we launch: `codex exec --json`

**Upstream/public fact.** For automation that owns process launch, this is the
simplest complete live stream:

```bash
codex exec --json "inspect and fix the failing test"
```

`stdout` becomes JSONL with one event per line. The top-level event family
includes:

```text
thread.started
turn.started
item.*
turn.completed
turn.failed
error
```

Use this for CI, scripts, and one-shot workers. It cannot retroactively attach
to an interactive session that another process already launched.

Source of truth:

- `codex-rs/exec/src/lib.rs`
- `codex-rs/exec/src/exec_events.rs`
- `codex-rs/exec/src/cli.rs`
- <https://learn.chatgpt.com/docs/non-interactive-mode>

## 4. Saved rollout JSONL

**Repository fact.** Non-ephemeral sessions persist append-only rollout files
under:

```text
$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread-id>.jsonl
```

Archived sessions move under:

```text
$CODEX_HOME/archived_sessions/
```

The rollout is richer than `$CODEX_HOME/history.jsonl`:

- `history.jsonl` is prompt history with fields such as `session_id`, `text`,
  and `ts`.
- `rollout-*.jsonl` contains session metadata, turn context, world state,
  protocol events, response items, tool calls and outputs, token counts, and
  other runtime evidence.

### Locate a rollout

Prefer app-server `thread/read` or state APIs when available. For local
diagnostics, a bounded filename lookup is sufficient:

```bash
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
THREAD_ID="<thread-id>"
find "$CODEX_HOME/sessions" -type f -name "*${THREAD_ID}.jsonl" -print -quit
```

### Tail only a sanitized outline

Do not stream raw records into an ordinary terminal or shared log collector.
Project only lifecycle metadata:

```bash
tail -F "$ROLLOUT" | jq -c '
  {
    timestamp,
    record: .type,
    event: (.payload.type // null),
    item: (.payload.item.type // null),
    tool: (
      if .payload.type == "custom_tool_call"
      then .payload.name
      else null
      end
    )
  }
'
```

Treat the rollout schema as persistence and diagnostic evidence, not a durable
external event contract. Prefer app-server protocol objects for product code.

## 5. Structured runtime logs: `logs_2.sqlite`

**Repository fact.** Codex writes structured tracing rows to the dedicated logs
database:

```text
$CODEX_HOME/logs_2.sqlite
```

The `logs` table includes:

```text
timestamp and nanoseconds
level
target and module path
source file and line
thread id
process UUID
rendered log body
estimated bytes
```

This is the right layer for questions such as:

- Did the SSE or WebSocket stream stop?
- Is a turn handler retrying?
- Which runtime module last emitted activity for this thread?
- Did a tool or terminal operation fail below the item protocol?

The existing tail client is:

```text
codex-rs/cli/src/bin/logs_client.rs
```

It supports thread, level, module, file, text, time-range, backfill, and polling
filters. From the Rust workspace:

```bash
cd codex-rs
cargo run -q -p codex-cli --bin logs_client -- \
  --thread-id "<thread-id>" \
  --level debug \
  --backfill 100
```

Stop it with `Ctrl-C`. The first build can be expensive; package or expose the
existing binary rather than writing another SQLite tailer.

Do not make the database schema a product contract. It is versioned internal
state and its rendered bodies may contain prompts, tool inputs and outputs,
terminal output, paths, and other sensitive values.

Related source:

- `codex-rs/state/src/log_db.rs`
- `codex-rs/state/src/runtime/logs.rs`
- `codex-rs/state/src/model/log.rs`
- `codex-rs/state/src/sqlite.rs`

## 6. Full causal reconstruction: rollout trace

**Lumi implementation.** This checkout contains an opt-in diagnostic system
designed specifically to answer "what happened throughout this session?"

Enable it before the root session starts:

```bash
CODEX_ROLLOUT_TRACE_ROOT="/explicit/private/trace-root" codex
```

The environment variable is read when the root `ThreadTraceContext` starts.
It cannot backfill a session that is already running. Fresh spawned child
threads inherit the root trace writer, so one bundle covers the multi-agent
rollout tree.

Each root session creates a bundle containing:

```text
manifest.json
trace.jsonl
payloads/*.json
```

`trace.jsonl` is an append-only raw event spine ordered by writer-assigned
sequence number. Payload files hold large evidence separately. Events cover:

- rollout, thread, and turn start/end;
- inference start/completion/failure/cancellation;
- tool dispatch and runtime start/end/result;
- MCP correlation;
- code-mode cell start/initial response/end;
- compaction request and installed checkpoints;
- child-to-parent agent-result delivery;
- wrapped protocol events and extension points.

Reduce the raw bundle into a semantic graph:

```bash
codex debug trace-reduce "/explicit/private/trace-bundle"
```

This writes `state.json` by default. The reduced graph distinguishes:

- model-visible conversation;
- inference calls;
- tool calls;
- code cells;
- terminal operations;
- compactions;
- agent threads; and
- information-flow edges such as spawn, task delivery, result, and close.

Exact evidence remains reachable through raw payload references.

Source of truth:

- `codex-rs/rollout-trace/README.md`
- `codex-rs/rollout-trace/src/raw_event.rs`
- `codex-rs/rollout-trace/src/thread.rs`
- `codex-rs/rollout-trace/src/reducer/mod.rs`
- `codex-rs/cli/src/main.rs` (`debug trace-reduce`)

### Trace safety

Rollout tracing is local, opt-in diagnostics, not telemetry. Bundles can contain
prompts, responses, raw tool inputs and outputs, terminal output, paths, and
multi-agent messages.

Always:

- use an explicit private directory outside the repository;
- restrict directory permissions;
- avoid automatic upload or long retention;
- redact before sharing; and
- delete only an exact verified bundle when cleanup is authorized.

Trace recording is best-effort by design: failure to write diagnostics must not
fail the Codex session.

## 7. OpenTelemetry

**Upstream/public fact.** Codex can export structured OTel logs, traces, and
metrics. Representative events include:

```text
codex.conversation_starts
codex.api_request
codex.sse_event
codex.websocket_request
codex.websocket_event
codex.user_prompt
codex.tool_decision
codex.tool_result
```

Use OTel for fleet health, aggregate latency, transport failures, and tool
success rates. It is not the first choice for inspecting one Desktop task.

Configuration is disabled by default and belongs in user-level config, not a
project-local `.codex/config.toml`:

```toml
[otel]
environment = "dev"
exporter = "none"
trace_exporter = "none"
log_user_prompt = false
```

Sources:

- <https://learn.chatgpt.com/docs/config-file/config-advanced#observability-and-telemetry>
- <https://learn.chatgpt.com/docs/config-file/config-reference>
- `codex-rs/otel/`

## Diagnostic escalation playbook

### A. "Is it still running?"

1. Use Desktop `list_threads` or app-server `thread/read`.
2. Check `thread.status` and `activeFlags`.
3. Check the latest turn status.

Do not open raw logs yet.

### B. "What is it waiting for?"

1. Check `waitingOnApproval`, `waitingOnUserInput`, and
   `waitingOnBackgroundTerminal`.
2. Read the most recent incomplete item.
3. If background terminals matter, call
   `thread/backgroundTerminals/list` on the owning app-server.

### C. "Has it stalled?"

1. Record the last meaningful item or status-transition timestamp.
2. Compare elapsed time with a task-specific watchdog, not a global short
   timeout.
3. Re-read the state once at the deadline.
4. If still active with no expected progress, inspect filtered runtime logs.

### D. "Why did it fail or hang?"

1. Read the turn error and final item statuses.
2. Tail `logs_2.sqlite` filtered by thread ID and a useful minimum level.
3. Inspect transport, turn, tool, and terminal targets before reading raw
   payload bodies.
4. Reproduce with rollout tracing only when ordinary evidence is insufficient.

### E. "We need a complete causal account"

1. Start a fresh reproduction with `CODEX_ROLLOUT_TRACE_ROOT` set.
2. Preserve the exact bundle privately.
3. Run `codex debug trace-reduce`.
4. Inspect semantic objects first and dereference raw payloads only where the
   graph leaves a real ambiguity.

## Design guidance for future monitors

A small Codex supervisor should consume existing state rather than inventing a
parallel event model:

```text
Desktop/app-server state and events
              |
              v
normalized observation receipt
              |
              v
watchdog and attention policy
              |
              v
notification, wakeup, or diagnostic escalation
```

The normalized receipt should contain only:

```text
thread_id
host_id
thread_status
active_flags
turn_id and turn_status
latest_item_kind and status
last_meaningful_event_at
background_terminal_count
attention_reason
```

It should not copy raw prompts, tool arguments, model reasoning, or terminal
output into ordinary monitoring state.

The monitor owns policy such as "this looks stalled after 20 minutes." Codex
continues to own thread, turn, item, approval, terminal, and error truth.

## Stability and privacy summary

| Surface | Safe to build product behavior on? | Sensitive contents |
| --- | --- | --- |
| Desktop `list/read/wait_threads` | Host-local integration only | Summaries and optional outputs |
| app-server v2 status and item APIs | Yes, respecting experimental gates | Conversation and tool items |
| `codex exec --json` | Yes for caller-owned processes | Full streamed work |
| rollout JSONL | No; diagnostic/persistence format | High |
| `logs_2.sqlite` | No; internal schema | High |
| rollout trace bundle | No; opt-in diagnostic schema | Very high |
| OTel | Yes as configured telemetry | Configurable; prompts redacted by default |

## External references

- App-server protocol: <https://learn.chatgpt.com/docs/app-server>
- Non-interactive JSONL mode:
  <https://learn.chatgpt.com/docs/non-interactive-mode>
- Advanced observability configuration:
  <https://learn.chatgpt.com/docs/config-file/config-advanced#observability-and-telemetry>
- Configuration reference:
  <https://learn.chatgpt.com/docs/config-file/config-reference>
- CLI developer commands:
  <https://learn.chatgpt.com/docs/developer-commands>

## Open questions

- Should the Desktop thread tools become a documented local automation API, or
  remain host-only agent capabilities?
- Should `logs_client` be shipped as a supported `codex debug logs` command?
- Which rollout-trace schema elements should become stable enough for a viewer?
- Should app-server expose a read-only subscribe operation that observes an
  already-loaded thread without resume semantics?
- What retention and redaction policy should apply to `logs_2.sqlite` and local
  trace bundles?
- What is the smallest stable observation receipt an alarm or organization
  plane can consume without inheriting raw event schemas?
