---
name: observe-codex-sessions
description: Observe and diagnose Codex sessions, tasks, threads, and turns using the least invasive available state, event, log, or trace surface. Use when asked to inspect or monitor a running Codex task, determine whether it is active, blocked, waiting, stalled, or failed, follow task completion, inspect session events or debug logs, reconstruct a rollout, investigate app-server session behavior, or design a Codex session watcher, alarm, dashboard, or observability integration. Do not use for ordinary application debugging when the Codex runtime or session lifecycle is not the object being observed.
---

# Observe Codex Sessions

## Objective

Establish what a specific Codex session is doing without inventing a parallel
event model or exposing more sensitive data than the diagnosis requires.
Start with normalized runtime state. Escalate to events, persisted rollout,
structured logs, or an opt-in trace only when the current question requires it.

## Core workflow

1. **Resolve the target and owner.** Record the thread ID, host ID when present,
   working directory, and the process or app-server daemon that owns the live
   thread. Distinguish a local interactive session, a Desktop task, a caller-owned
   `codex exec` process, and a Codex Cloud task. Match by exact ID when supplied;
   otherwise combine title, host, working directory, and recency. If multiple
   plausible live targets remain, ask rather than observing the wrong session.
2. **Take one normalized snapshot.** Prefer Desktop `list_threads` and
   `read_thread`, or app-server `thread/read` on the owning daemon. Do not open
   raw logs first.
3. **Classify the state.** Use the observation tuple below. Never interpret the
   single word `active` as proof of progress.
4. **Wait at the event boundary.** Prefer Desktop `wait_threads`, app-server
   notifications, or the caller-owned `codex exec --json` stream. Choose one
   bounded wait ending at the next meaningful transition; avoid short polling.
5. **Escalate evidence only when needed.** Use filtered runtime logs for a stall
   or transport failure. Use rollout tracing only for a fresh reproduction that
   needs causal reconstruction.
6. **Report facts separately from inference.** State what the runtime reports,
   what conclusion follows, the last meaningful activity, and what observation
   would distinguish any remaining possibilities.

## Observation tuple

Collect the smallest useful receipt:

```text
thread_id and host_id
thread_status and active_flags
turn_id and turn_status
latest incomplete item kind and status
last meaningful event time
live background terminal count
latest warning or error
```

Classify it as:

- **working** — an active turn with recent expected events;
- **blocked** — waiting for approval or user input;
- **waiting** — the foreground turn ended but a background terminal remains;
- **failed** — the turn or thread reports a terminal error;
- **stalled** — nominally active without meaningful progress beyond a
  task-specific watchdog.

Treat `stalled` as an observer inference, never an intrinsic Codex status.

## Choose the surface

| Situation | Use | Avoid |
| --- | --- | --- |
| Ordinary Desktop supervision | `list_threads` -> `read_thread` -> `wait_threads` | Polling rollout files |
| Read-only app-server snapshot | `thread/read` on the owning daemon | `thread/resume` merely to inspect |
| Live app-server integration | `thread/status/changed`, `turn/*`, and `item/*` notifications | Starting a second daemon and assuming it owns live state |
| Automation that launches Codex | `codex exec --json` | Retrofitting attachment after launch |
| Post-hoc local history | Saved rollout JSONL | Treating `history.jsonl` as a full trace |
| Runtime or transport diagnosis | Filtered `logs_2.sqlite` rows through the existing `logs_client` | Dumping unfiltered log bodies |
| Complete causal reproduction | `CODEX_ROLLOUT_TRACE_ROOT` before launch, then `codex debug trace-reduce` | Expecting retroactive trace capture |
| Fleet telemetry | OTel logs, traces, and metrics | Using OTel as the first per-task debugger |

## Ordinary monitoring

When Desktop thread tools are available:

1. Call `list_threads` once to resolve the target and retain its `hostId`.
2. Call `read_thread` for runtime status, recent turns, and item summaries. Keep
   outputs excluded unless their content is needed.
3. If the task is still running, call `wait_threads` with a deadline sized for
   the next meaningful transition. Reuse its cursor on later waits.
4. Read the thread again only after completion, required attention, a timeout,
   or a material status change.

Do not call `wait_threads` on the calling thread; that surface is for observing
peer tasks. When Fletcher asks about the current task, answer naturally and
continue holding its work rather than treating the status exchange as a stop.

When using app-server directly, initialize the connection and use
`thread/read`. A separate app-server process can read persisted history but
usually reports a thread loaded by another daemon as `notLoaded`. Connect to the
existing managed daemon when live runtime truth matters. Keep non-loopback
transports authenticated and bounded.

## Diagnostic escalation

Escalate one layer at a time:

1. Inspect status, active flags, current turn, and incomplete items.
2. Check background terminals when `waitingOnBackgroundTerminal` is present or
   the task launched a persistent process.
3. Compare the last meaningful event time with a task-specific watchdog.
4. Query structured logs by exact thread ID and a useful minimum level. Inspect
   timestamp, level, target, module, and source location before reading bodies.
5. Read sanitized rollout event outlines only if the owning daemon is
   unavailable or persisted ordering matters.
6. Reproduce with rollout tracing only when state, events, and filtered logs do
   not explain the failure.

The repository already contains the log tailer at
`codex-rs/cli/src/bin/logs_client.rs`. Reuse or package it instead of writing
another SQLite polling client.

## Rollout trace reproduction

Rollout tracing in this checkout is a Lumi diagnostic implementation. It is
local and opt-in, not telemetry or a stable public contract.

1. Choose an explicit private trace root outside the repository and restrict its
   permissions.
2. Set `CODEX_ROLLOUT_TRACE_ROOT` before starting the root session.
3. Reproduce the behavior. Fresh child agents share the root bundle.
4. Run `codex debug trace-reduce <trace-bundle>` to create `state.json`.
5. Inspect reduced semantic objects and edges first. Dereference raw payloads
   only where the graph leaves a real ambiguity.

A trace cannot be enabled retroactively for a session already in progress.
Treat bundles as highly sensitive: they can contain prompts, responses, tool
inputs and outputs, terminal output, paths, and multi-agent messages. Never
place them in the repository or upload them by default.

## Safety and contract boundaries

- Preserve the owning process and subscription lifetime. Read-only observation
  must not resume, fork, interrupt, or otherwise mutate the target thread.
- Do not expose an unauthenticated non-loopback listener or discover unrelated
  hosts, sockets, or sessions.
- Prefer exact thread IDs, host IDs, socket paths, and verified rollout paths.
- Keep credentials and credential-bearing output out of commands, notes,
  traces, and handoffs.
- Treat rollout JSONL, `logs_2.sqlite`, and rollout trace schemas as internal
  diagnostics. Build durable integrations on app-server v2 or caller-owned
  `codex exec --json` events.
- Redact prompts and raw payloads by default. A monitor generally needs state,
  kind, status, and time—not content.
- Do not delete or rotate diagnostic artifacts without explicit authority and
  an exact verified target.

## Reporting format

Return a compact observation receipt:

```text
Target: <thread and host>
Runtime fact: <status, flags, turn, latest item>
Last activity: <timestamp and event kind>
Interpretation: <working, blocked, waiting, failed, or inferred stalled>
Next transition: <what event or deadline matters>
Evidence level: <state, events, logs, rollout, or trace>
Limitations: <only material uncertainty or missing ownership>
```

Do not paste raw prompts, reasoning, tool payloads, or terminal output unless the
user explicitly needs that evidence and it is safe to disclose.

## Detailed reference

Read [`references/field-guide.md`](references/field-guide.md) when exact
commands, protocol methods, source paths, event families, privacy details, or
the evidence/stability matrix are needed. Its diagnostic escalation playbook is
the source of truth for recurring operational use.

When repository behavior changes, update the reference's research snapshot and
source map together with this workflow. Keep detailed evidence in the reference
rather than expanding this skill body.
