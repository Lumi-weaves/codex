#!/usr/bin/env python3
"""Convert the sanitized Lumi runtime-trace JSON contract to Perfetto.

Requires the official ``perfetto`` Python package. The checked-in dogfood
artifact is generated with an ephemeral pinned dependency, so this helper does
not add Python runtime dependencies to Lumi Codex itself.
"""

from __future__ import annotations

import hashlib
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

from perfetto.protos.perfetto.trace.perfetto_trace_pb2 import TrackDescriptor
from perfetto.protos.perfetto.trace.perfetto_trace_pb2 import TrackEvent
from perfetto.trace_builder.proto_builder import TraceProtoBuilder


SEQUENCE_ID = 1001
TRACE_FILE_CLOCK_ID = 11


def timestamp_ns(value: str) -> int:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    return int(parsed.timestamp() * 1_000_000_000)


def stable_id(namespace: str, value: str) -> int:
    digest = hashlib.blake2b(
        f"lumi-codex:{namespace}:{value}".encode(), digest_size=8
    ).digest()
    return int.from_bytes(digest, "big") or 1


def add_annotation(event: Any, name: str, value: str | int | bool) -> None:
    annotation = event.debug_annotations.add()
    annotation.name = name
    if isinstance(value, bool):
        annotation.bool_value = value
    elif isinstance(value, int):
        annotation.int_value = value
    else:
        annotation.string_value = value


def add_descriptor(
    builder: TraceProtoBuilder,
    track_uuid: int,
    name: str,
    *,
    parent_uuid: int | None = None,
    rank: int = 0,
    counter: bool = False,
) -> None:
    descriptor = builder.add_packet().track_descriptor
    descriptor.uuid = track_uuid
    descriptor.name = name
    descriptor.sibling_order_rank = rank
    descriptor.sibling_merge_behavior = TrackDescriptor.SIBLING_MERGE_BEHAVIOR_NONE
    if parent_uuid is not None:
        descriptor.parent_uuid = parent_uuid
    if counter:
        descriptor.counter.unit = descriptor.counter.UNIT_COUNT


def add_event(
    builder: TraceProtoBuilder,
    timestamp: int,
    event_type: int,
    track_uuid: int,
    *,
    name: str | None = None,
    flow_ids: list[int] | None = None,
    terminating_flow_ids: list[int] | None = None,
    annotations: dict[str, str | int | bool] | None = None,
    counter_value: int | None = None,
) -> None:
    packet = builder.add_packet()
    packet.timestamp = timestamp
    packet.timestamp_clock_id = TRACE_FILE_CLOCK_ID
    packet.trusted_packet_sequence_id = SEQUENCE_ID
    event = packet.track_event
    event.type = event_type
    event.track_uuid = track_uuid
    if name is not None:
        event.name = name
    if flow_ids:
        event.flow_ids.extend(flow_ids)
    if terminating_flow_ids:
        event.terminating_flow_ids.extend(terminating_flow_ids)
    for key, value in (annotations or {}).items():
        add_annotation(event, key, value)
    if counter_value is not None:
        event.counter_value = counter_value


def convert(trace: dict[str, Any]) -> bytes:
    builder = TraceProtoBuilder()
    operations = {item["id"]: item for item in trace["operations"]}
    agents = {item["id"]: item for item in trace["agents"]}

    root_track = stable_id("track", "lumi-codex-runtime")
    add_descriptor(builder, root_track, "Lumi Codex runtime")

    agent_tracks: dict[str, int] = {}
    for rank, agent in enumerate(trace["agents"]):
        track_uuid = stable_id("agent-track", agent["id"])
        agent_tracks[agent["id"]] = track_uuid
        label = agent["label"]
        if agent["model"]:
            label = f"{label} · {agent['model']}"
        add_descriptor(
            builder, track_uuid, label, parent_uuid=root_track, rank=rank
        )

    operation_tracks: dict[str, int] = {}
    for rank, kind in enumerate(sorted({item["kind"] for item in operations.values()})):
        track_uuid = stable_id("operation-track", kind)
        operation_tracks[kind] = track_uuid
        add_descriptor(
            builder,
            track_uuid,
            f"Operations · {kind}",
            parent_uuid=root_track,
            rank=100 + rank,
        )

    counter_tracks = {
        "operations": stable_id("counter-track", "active-operations"),
        "subagents": stable_id("counter-track", "active-subagents"),
    }
    add_descriptor(
        builder,
        counter_tracks["operations"],
        "Active operations",
        parent_uuid=root_track,
        rank=200,
        counter=True,
    )
    add_descriptor(
        builder,
        counter_tracks["subagents"],
        "Active subagents",
        parent_uuid=root_track,
        rank=201,
        counter=True,
    )

    event_flow_ids = {
        item["id"]: stable_id("flow", item["id"]) for item in trace["events"]
    }
    incoming_flows: dict[str, list[int]] = {}
    outgoing_flows: dict[str, list[int]] = {}
    operation_start_flows: dict[str, list[int]] = {}
    operation_end_flows: dict[str, list[int]] = {}
    for event in trace["events"]:
        flow_id = event_flow_ids[event["id"]]
        incoming_flows.setdefault(event["consumedByGenerationId"], []).append(flow_id)
        if event["sourceOperationId"] is not None:
            operation_end_flows.setdefault(event["sourceOperationId"], []).append(flow_id)
        elif event["emittedByGenerationId"] is not None:
            outgoing_flows.setdefault(event["emittedByGenerationId"], []).append(flow_id)

    for operation in operations.values():
        operation_start_flows[operation["id"]] = [
            stable_id("flow", f"dispatch:{operation['id']}")
        ]
        outgoing_flows.setdefault(operation["emittedByGenerationId"], []).extend(
            operation_start_flows[operation["id"]]
        )

    for generation in trace["generations"]:
        track_uuid = agent_tracks[generation["agentId"]]
        add_event(
            builder,
            timestamp_ns(generation["startedAt"]),
            TrackEvent.TYPE_SLICE_BEGIN,
            track_uuid,
            name=f"Generation {generation['sequence']} · {generation['outcome']}",
            terminating_flow_ids=incoming_flows.get(generation["id"]),
            annotations={
                "agent": agents[generation["agentId"]]["label"],
                "sequence": generation["sequence"],
                "outcome": generation["outcome"],
                "boundary_source": "historical-rollout-inference",
            },
        )
        add_event(
            builder,
            timestamp_ns(generation["completedAt"]),
            TrackEvent.TYPE_SLICE_END,
            track_uuid,
            flow_ids=outgoing_flows.get(generation["id"]),
        )

    for operation in operations.values():
        track_uuid = operation_tracks[operation["kind"]]
        add_event(
            builder,
            timestamp_ns(operation["startedAt"]),
            TrackEvent.TYPE_SLICE_BEGIN,
            track_uuid,
            name=operation["label"],
            terminating_flow_ids=operation_start_flows[operation["id"]],
            annotations={
                "kind": operation["kind"],
                "operation_id": operation["id"],
                "boundary_source": "observed-tool-call-pair",
            },
        )
        add_event(
            builder,
            timestamp_ns(operation["completedAt"]),
            TrackEvent.TYPE_SLICE_END,
            track_uuid,
            flow_ids=operation_end_flows.get(operation["id"]),
        )

    for event in trace["events"]:
        if event["sourceOperationId"] is not None:
            track_uuid = operation_tracks[operations[event["sourceOperationId"]]["kind"]]
        elif event["sourceAgentId"] is not None:
            track_uuid = agent_tracks[event["sourceAgentId"]]
        else:
            track_uuid = agent_tracks[event["targetAgentId"]]
        add_event(
            builder,
            timestamp_ns(event["occurredAt"]),
            TrackEvent.TYPE_INSTANT,
            track_uuid,
            name=event["label"],
            flow_ids=[event_flow_ids[event["id"]]],
            annotations={
                "kind": event["kind"],
                "event_id": event["id"],
                "join_source": "historical-rollout-inference",
            },
        )

    operation_points: list[tuple[int, int]] = []
    for operation in operations.values():
        operation_points.extend(
            [
                (timestamp_ns(operation["startedAt"]), 1),
                (timestamp_ns(operation["completedAt"]), -1),
            ]
        )
    active = 0
    for timestamp, delta in sorted(operation_points, key=lambda item: (item[0], item[1])):
        active += delta
        add_event(
            builder,
            timestamp,
            TrackEvent.TYPE_COUNTER,
            counter_tracks["operations"],
            counter_value=active,
        )

    child_points: list[tuple[int, int]] = []
    for agent in trace["agents"]:
        if agent["parentAgentId"] is None:
            continue
        spans = [
            generation
            for generation in trace["generations"]
            if generation["agentId"] == agent["id"]
        ]
        if spans:
            child_points.extend(
                [
                    (min(timestamp_ns(span["startedAt"]) for span in spans), 1),
                    (max(timestamp_ns(span["completedAt"]) for span in spans), -1),
                ]
            )
    active = 0
    for timestamp, delta in sorted(child_points, key=lambda item: (item[0], item[1])):
        active += delta
        add_event(
            builder,
            timestamp,
            TrackEvent.TYPE_COUNTER,
            counter_tracks["subagents"],
            counter_value=active,
        )

    return builder.serialize()


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: runtime_trace_to_perfetto.py INPUT.json OUTPUT.pftrace")
    source = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    Path(sys.argv[2]).write_bytes(convert(source))


if __name__ == "__main__":
    main()
