#!/usr/bin/env python3
"""Export a content-free causal trace from local Codex rollouts.

The exporter reads only local state. It deliberately ignores prompts, model
text, tool inputs and tool outputs; its JSON contains aliases, item kinds,
causal references and timestamps only.

This is a historical rollout adapter for dogfood evidence, not the live trace
instrumentation contract. Rollout line timestamps are recorder-observation
times; generation boundaries and completion consumption are inferred.
"""

from __future__ import annotations

import json
import sqlite3
import sys
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def iso(value: str | int | float) -> str:
    if isinstance(value, str):
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    else:
        parsed = datetime.fromtimestamp(float(value), timezone.utc)
    return parsed.astimezone(timezone.utc).isoformat(timespec="milliseconds").replace(
        "+00:00", "Z"
    )


def millis(value: str) -> int:
    return int(datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000)


@dataclass
class SafeRecord:
    timestamp: str
    kind: str
    call_id: str | None = None
    name: str | None = None
    turn_id: str | None = None
    child_thread_id: str | None = None
    activity_kind: str | None = None
    author: str | None = None
    recipient: str | None = None
    phase: str | None = None


@dataclass
class ThreadData:
    thread_id: str
    alias: str
    label: str
    model: str
    parent_id: str | None
    agent_path: str | None
    records: list[SafeRecord]
    generations: list[dict[str, Any]] = field(default_factory=list)
    operations: list[dict[str, Any]] = field(default_factory=list)
    completion_events: list[dict[str, Any]] = field(default_factory=list)
    spawn_calls: list[tuple[str, str, str]] = field(default_factory=list)
    task_start_generation_ids: list[str] = field(default_factory=list)
    task_complete_at: str | None = None


def read_records(path: Path) -> list[SafeRecord]:
    records: list[SafeRecord] = []
    with path.open(encoding="utf-8") as rollout:
        for line in rollout:
            raw = json.loads(line)
            timestamp = raw.get("timestamp")
            payload = raw.get("payload")
            if not isinstance(timestamp, str) or not isinstance(payload, dict):
                continue
            kind = payload.get("type")
            if kind not in {
                "task_started",
                "task_complete",
                "reasoning",
                "agent_reasoning",
                "message",
                "agent_message",
                "custom_tool_call",
                "custom_tool_call_output",
                "function_call",
                "function_call_output",
                "sub_agent_activity",
            }:
                continue
            # Content, arguments, tool input and tool output are never copied.
            records.append(
                SafeRecord(
                    timestamp=iso(timestamp),
                    kind=kind,
                    call_id=payload.get("call_id") or payload.get("event_id"),
                    name=payload.get("name"),
                    turn_id=payload.get("turn_id"),
                    child_thread_id=payload.get("agent_thread_id"),
                    activity_kind=payload.get("kind"),
                    author=payload.get("author"),
                    recipient=payload.get("recipient"),
                    phase=payload.get("phase"),
                )
            )
    return records


def load_threads(db_path: Path, root_id: str) -> tuple[list[ThreadData], dict[str, str]]:
    connection = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    children: dict[str, list[str]] = {}
    queue = [root_id]
    ordered_ids: list[str] = []
    while queue:
        thread_id = queue.pop(0)
        if thread_id in ordered_ids:
            continue
        ordered_ids.append(thread_id)
        child_ids = [
            row[0]
            for row in connection.execute(
                "select child_thread_id from thread_spawn_edges "
                "where parent_thread_id = ? order by child_thread_id",
                (thread_id,),
            )
        ]
        children[thread_id] = child_ids
        queue.extend(child_ids)

    rows = {
        row["id"]: row
        for row in connection.execute(
            "select id, rollout_path, model, reasoning_effort, agent_nickname, "
            "agent_role, agent_path from threads where id in (%s)"
            % ",".join("?" for _ in ordered_ids),
            ordered_ids,
        )
    }
    if len(rows) != len(ordered_ids):
        raise SystemExit("one or more thread rows are missing")

    parent_by_child = {
        child: parent for parent, child_ids in children.items() for child in child_ids
    }
    aliases = {root_id: "main"}
    child_number = 0
    for thread_id in ordered_ids[1:]:
        child_number += 1
        nickname = rows[thread_id]["agent_nickname"]
        aliases[thread_id] = (
            str(nickname).lower().replace(" ", "-") if nickname else f"agent-{child_number}"
        )

    threads: list[ThreadData] = []
    for thread_id in ordered_ids:
        row = rows[thread_id]
        nickname = row["agent_nickname"]
        role = row["agent_role"]
        if thread_id == root_id:
            label = "Main agent"
        elif nickname and role:
            label = f"{nickname} · {role}"
        else:
            label = str(nickname or f"Subagent {len(threads)}")
        model = str(row["model"] or "unknown")
        effort = row["reasoning_effort"]
        if effort:
            model = f"{model} · {effort}"
        threads.append(
            ThreadData(
                thread_id=thread_id,
                alias=aliases[thread_id],
                label=label,
                model=model,
                parent_id=parent_by_child.get(thread_id),
                agent_path=row["agent_path"],
                records=read_records(Path(row["rollout_path"])),
            )
        )
    return threads, aliases


def is_model_item(record: SafeRecord) -> bool:
    if record.kind in {"reasoning", "agent_reasoning", "custom_tool_call", "function_call"}:
        return True
    if record.kind == "message":
        return True
    return record.kind == "agent_message" and record.author is None


def parse_thread(thread: ThreadData) -> None:
    records = thread.records
    operation_number = 0
    generation_number = 0
    index = 0
    pending_event_ids: list[str] = []

    while index < len(records):
        started = records[index]
        if started.kind != "task_started":
            index += 1
            continue
        turn_id = started.turn_id
        complete_index = next(
            (
                position
                for position in range(index + 1, len(records))
                if records[position].kind == "task_complete"
                and records[position].turn_id == turn_id
            ),
            None,
        )
        if complete_index is None:
            break
        complete = records[complete_index]
        thread.task_complete_at = complete.timestamp
        first_generation_number = generation_number + 1
        boundary = started.timestamp
        first_token: str | None = None
        cursor = index + 1

        while cursor < complete_index:
            record = records[cursor]
            if is_model_item(record) and first_token is None:
                first_token = record.timestamp

            if record.kind not in {"custom_tool_call", "function_call"}:
                if record.kind == "agent_message" and record.phase == "final_answer":
                    generation_number += 1
                    thread.generations.append(
                        generation(
                            thread,
                            generation_number,
                            boundary,
                            first_token or record.timestamp,
                            record.timestamp,
                            "final",
                            pending_event_ids,
                        )
                    )
                    pending_event_ids = []
                    boundary = record.timestamp
                    first_token = None
                cursor += 1
                continue

            output_kind = (
                "custom_tool_call_output"
                if record.kind == "custom_tool_call"
                else "function_call_output"
            )
            output_index = next(
                (
                    position
                    for position in range(cursor + 1, complete_index)
                    if records[position].kind == output_kind
                    and records[position].call_id == record.call_id
                ),
                None,
            )
            if output_index is None:
                cursor += 1
                continue
            output = records[output_index]
            generation_number += 1
            generation_id = f"{thread.alias}-g{generation_number}"
            thread.generations.append(
                generation(
                    thread,
                    generation_number,
                    boundary,
                    first_token or record.timestamp,
                    record.timestamp,
                    "tool-call",
                    pending_event_ids,
                )
            )
            pending_event_ids = []

            operation_number += 1
            operation_id = f"{thread.alias}-op-{operation_number:02d}"
            event_id = f"{operation_id}-complete"
            name = record.name or "tool"
            kind = (
                "terminal"
                if name == "exec"
                else "wait"
                if name in {"wait", "wait_agent"}
                else "agent-control"
                if name in {"spawn_agent", "send_message", "followup_task"}
                else "tool"
            )
            thread.operations.append(
                {
                    "id": operation_id,
                    "kind": kind,
                    "label": f"{name} · {operation_number:02d}",
                    "emittedByGenerationId": generation_id,
                    "startedAt": record.timestamp,
                    "yieldedAt": None,
                    "completedAt": output.timestamp,
                    "completionEventId": event_id,
                }
            )
            thread.completion_events.append(
                {
                    "id": event_id,
                    "kind": "operation-completion",
                    "label": f"{name} completed",
                    "occurredAt": output.timestamp,
                    "enqueuedAt": output.timestamp,
                    "emittedByGenerationId": None,
                    "sourceOperationId": operation_id,
                    "sourceAgentId": None,
                    "targetAgentId": thread.alias,
                    "consumedByGenerationId": "__next__",
                }
            )
            pending_event_ids = [event_id]

            if name == "spawn_agent":
                activity = next(
                    (
                        candidate
                        for candidate in records[cursor + 1 : output_index + 1]
                        if candidate.kind == "sub_agent_activity"
                        and candidate.call_id == record.call_id
                        and candidate.activity_kind == "started"
                        and candidate.child_thread_id is not None
                    ),
                    None,
                )
                if activity is not None:
                    thread.spawn_calls.append(
                        (generation_id, activity.timestamp, activity.child_thread_id)
                    )

            boundary = output.timestamp
            first_token = None
            cursor = output_index + 1

        # Some interrupted/legacy turns have no final response item.
        if not thread.generations or millis(thread.generations[-1]["completedAt"]) < millis(
            complete.timestamp
        ):
            remaining_model_items = [
                record
                for record in records[cursor:complete_index]
                if is_model_item(record)
            ]
            if remaining_model_items:
                generation_number += 1
                thread.generations.append(
                    generation(
                        thread,
                        generation_number,
                        boundary,
                        remaining_model_items[0].timestamp,
                        remaining_model_items[-1].timestamp,
                        "final",
                        pending_event_ids,
                    )
                )
                pending_event_ids = []
        index = complete_index + 1
        if generation_number >= first_generation_number:
            thread.task_start_generation_ids.append(
                f"{thread.alias}-g{first_generation_number}"
            )

    # Attach each operation completion to the next generation of its agent.
    generations_by_id = {item["id"]: item for item in thread.generations}
    for event in thread.completion_events:
        operation_id = event["sourceOperationId"]
        operation = next(item for item in thread.operations if item["id"] == operation_id)
        emitter = generations_by_id[operation["emittedByGenerationId"]]
        emitter_sequence = emitter["sequence"]
        consumer = next(
            (
                item
                for item in thread.generations
                if item["sequence"] > emitter_sequence
                and millis(item["startedAt"]) >= millis(event["enqueuedAt"])
            ),
            None,
        )
        if consumer is None:
            continue
        event["consumedByGenerationId"] = consumer["id"]

    valid_event_ids = {
        event["id"]
        for event in thread.completion_events
        if event["consumedByGenerationId"] != "__next__"
    }
    thread.completion_events = [
        event for event in thread.completion_events if event["id"] in valid_event_ids
    ]
    thread.operations = [
        operation
        for operation in thread.operations
        if operation["completionEventId"] in valid_event_ids
    ]
    for item in thread.generations:
        item["consumedEventIds"] = [
            event_id for event_id in item["consumedEventIds"] if event_id in valid_event_ids
        ]


def generation(
    thread: ThreadData,
    sequence: int,
    started_at: str,
    first_token_at: str,
    completed_at: str,
    outcome: str,
    consumed: list[str],
) -> dict[str, Any]:
    return {
        "id": f"{thread.alias}-g{sequence}",
        "agentId": thread.alias,
        "sequence": sequence,
        "startedAt": started_at,
        "firstTokenAt": max(first_token_at, started_at),
        "completedAt": max(completed_at, first_token_at, started_at),
        "outcome": outcome,
        "consumedEventIds": list(consumed),
    }


def next_generation(thread: ThreadData, timestamp: str) -> dict[str, Any] | None:
    return next(
        (
            item
            for item in thread.generations
            if millis(item["firstTokenAt"]) >= millis(timestamp)
        ),
        None,
    )


def build_trace(threads: list[ThreadData], aliases: dict[str, str]) -> dict[str, Any]:
    for thread in threads:
        parse_thread(thread)
    by_id = {thread.thread_id: thread for thread in threads}
    events: list[dict[str, Any]] = []

    for thread in threads:
        events.extend(thread.completion_events)

    # Root task starts are the only user-input facts retained.
    root = threads[0]
    root_generations = {item["id"]: item for item in root.generations}
    for index, generation_id in enumerate(root.task_start_generation_ids, start=1):
        generation_item = root_generations[generation_id]
        event_id = f"user-input-{index}"
        generation_item["consumedEventIds"].insert(0, event_id)
        events.append(
            {
                "id": event_id,
                "kind": "user-input",
                "label": "user input",
                "occurredAt": generation_item["startedAt"],
                "enqueuedAt": generation_item["startedAt"],
                "emittedByGenerationId": None,
                "sourceOperationId": None,
                "sourceAgentId": None,
                "targetAgentId": root.alias,
                "consumedByGenerationId": generation_item["id"],
            }
        )

    # Spawn events use the persisted parent activity timestamp and child turn.
    for parent in threads:
        for source_generation_id, occurred_at, child_id in parent.spawn_calls:
            child = by_id.get(child_id)
            if child is None or not child.generations:
                continue
            consumer = child.generations[0]
            event_id = f"spawn-{child.alias}"
            consumer["consumedEventIds"].insert(0, event_id)
            events.append(
                {
                    "id": event_id,
                    "kind": "agent-spawn",
                    "label": f"spawn {child.label.split(' · ')[0]}",
                    "occurredAt": occurred_at,
                    "enqueuedAt": occurred_at,
                    "emittedByGenerationId": source_generation_id,
                    "sourceOperationId": None,
                    "sourceAgentId": parent.alias,
                    "targetAgentId": child.alias,
                    "consumedByGenerationId": consumer["id"],
                }
            )

    # Cross-agent messages persisted in a recipient rollout are joined to the
    # next recipient generation. Assignment copies are omitted: spawn already
    # represents their causal edge.
    path_to_thread = {
        thread.agent_path: thread
        for thread in threads
        if thread.agent_path is not None
    }
    return_number = 0
    for target in threads:
        for record in target.records:
            if record.kind != "agent_message" or record.author is None:
                continue
            source = path_to_thread.get(record.author)
            if source is None or source.parent_id != target.thread_id:
                continue
            source_generation = source.generations[-1] if source.generations else None
            consumer = next_generation(target, record.timestamp)
            if source_generation is None or consumer is None:
                continue
            return_number += 1
            event_id = f"agent-return-{return_number}"
            consumer["startedAt"] = max(consumer["startedAt"], record.timestamp)
            consumer["firstTokenAt"] = max(
                consumer["firstTokenAt"], consumer["startedAt"]
            )
            consumer["consumedEventIds"].append(event_id)
            events.append(
                {
                    "id": event_id,
                    "kind": "agent-return",
                    "label": f"{source.label.split(' · ')[0]} return",
                    "occurredAt": source_generation["completedAt"],
                    "enqueuedAt": record.timestamp,
                    "emittedByGenerationId": source_generation["id"],
                    "sourceOperationId": None,
                    "sourceAgentId": source.alias,
                    "targetAgentId": target.alias,
                    "consumedByGenerationId": consumer["id"],
                }
            )

    generations = [item for thread in threads for item in thread.generations]
    operations = [item for thread in threads for item in thread.operations]
    if not generations:
        raise SystemExit("the selected trace contains no completed generation")
    started_at = min(item["startedAt"] for item in generations)
    captured_at = max(
        [item["completedAt"] for item in generations]
        + [item["completedAt"] for item in operations]
    )
    return {
        "schemaVersion": 1,
        "traceId": "hw1-captured-coordination",
        "startedAt": started_at,
        "capturedAt": captured_at,
        "agents": [
            {
                "id": thread.alias,
                "parentAgentId": aliases.get(thread.parent_id),
                "spawnedByGenerationId": next(
                    (
                        generation_id
                        for parent in threads
                        for generation_id, _, child_id in parent.spawn_calls
                        if child_id == thread.thread_id
                    ),
                    None,
                ),
                "label": thread.label,
                "model": thread.model,
            }
            for thread in threads
        ],
        "generations": generations,
        "operations": operations,
        "events": sorted(events, key=lambda item: item["occurredAt"]),
    }


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: export-runtime-trace.py STATE_DB ROOT_THREAD_ID")
    threads, aliases = load_threads(Path(sys.argv[1]), sys.argv[2])
    print(json.dumps(build_trace(threads, aliases), indent=2))


if __name__ == "__main__":
    main()
