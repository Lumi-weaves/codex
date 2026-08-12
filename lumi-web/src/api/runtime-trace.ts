/**
 * Closed, content-free contract for visualising one agent runtime trace.
 *
 * It intentionally carries timing and causality, but never prompt text, model
 * output, terminal output, filesystem paths, or message bodies.
 */

import capturedRuntimeTracePayload from "../fixtures/hw1-runtime-trace.json";

export const RUNTIME_TRACE_SCHEMA_VERSION = 1;

export type GenerationOutcome = "tool-call" | "message" | "final";
export type OperationKind = "terminal" | "wait" | "agent-control" | "tool";
export type RuntimeEventKind =
  | "user-input"
  | "tool-yield"
  | "operation-completion"
  | "agent-spawn"
  | "agent-message"
  | "agent-return";

export interface AgentLane {
  id: string;
  parentAgentId: string | null;
  spawnedByGenerationId: string | null;
  label: string;
  model: string;
}

export interface GenerationSpan {
  id: string;
  agentId: string;
  sequence: number;
  startedAt: string;
  firstTokenAt: string;
  completedAt: string;
  outcome: GenerationOutcome;
  consumedEventIds: string[];
}

export interface OperationSpan {
  id: string;
  kind: OperationKind;
  label: string;
  emittedByGenerationId: string;
  startedAt: string;
  yieldedAt: string | null;
  completedAt: string;
  completionEventId: string;
}

export interface RuntimeEvent {
  id: string;
  kind: RuntimeEventKind;
  label: string;
  occurredAt: string;
  enqueuedAt: string;
  emittedByGenerationId: string | null;
  sourceOperationId: string | null;
  sourceAgentId: string | null;
  targetAgentId: string;
  consumedByGenerationId: string;
}

export interface RuntimeTrace {
  schemaVersion: typeof RUNTIME_TRACE_SCHEMA_VERSION;
  traceId: string;
  startedAt: string;
  capturedAt: string;
  agents: AgentLane[];
  generations: GenerationSpan[];
  operations: OperationSpan[];
  events: RuntimeEvent[];
}

export interface RuntimeTraceResult {
  trace: RuntimeTrace;
  source: "captured" | "fixture";
}

export interface RuntimeTraceStats {
  wallTimeMs: number;
  mainGenerations: number;
  totalGenerations: number;
  asyncBranches: number;
  agentMessages: number;
  maxConcurrency: number;
  asyncOverlapRatio: number;
  medianQueueDelayMs: number;
  maxQueueDelayMs: number;
}

const BASE = Date.parse("2026-08-12T09:41:00.000Z");
const at = (seconds: number) => new Date(BASE + seconds * 1_000).toISOString();

/** A deterministic trace shaped like the async terminal/subagent workflow. */
export function fixtureRuntimeTrace(): RuntimeTrace {
  return {
    schemaVersion: RUNTIME_TRACE_SCHEMA_VERSION,
    traceId: "trace-async-terminal-and-subagent",
    startedAt: at(0),
    capturedAt: at(28),
    agents: [
      {
        id: "agent-main",
        parentAgentId: null,
        spawnedByGenerationId: null,
        label: "Main agent",
        model: "gpt-5.6-sol",
      },
      {
        id: "agent-luna",
        parentAgentId: "agent-main",
        spawnedByGenerationId: "main-g1",
        label: "Luna · validator",
        model: "gpt-5.6-luna",
      },
      {
        id: "agent-vera",
        parentAgentId: "agent-main",
        spawnedByGenerationId: "main-g1",
        label: "Vera · investigator",
        model: "deepseek-v4-flash",
      },
    ],
    generations: [
      {
        id: "main-g1",
        agentId: "agent-main",
        sequence: 1,
        startedAt: at(0),
        firstTokenAt: at(0.45),
        completedAt: at(2.8),
        outcome: "tool-call",
        consumedEventIds: ["event-user-input"],
      },
      {
        id: "main-g2",
        agentId: "agent-main",
        sequence: 2,
        startedAt: at(3),
        firstTokenAt: at(3.35),
        completedAt: at(5.2),
        outcome: "message",
        consumedEventIds: ["event-terminal-yield"],
      },
      {
        id: "main-g3",
        agentId: "agent-main",
        sequence: 3,
        startedAt: at(16),
        firstTokenAt: at(16.5),
        completedAt: at(19.8),
        outcome: "message",
        consumedEventIds: [
          "event-terminal-complete",
          "event-luna-checkpoint",
          "event-vera-return",
        ],
      },
      {
        id: "main-g4",
        agentId: "agent-main",
        sequence: 4,
        startedAt: at(25),
        firstTokenAt: at(25.4),
        completedAt: at(28),
        outcome: "final",
        consumedEventIds: ["event-luna-return"],
      },
      {
        id: "luna-g1",
        agentId: "agent-luna",
        sequence: 1,
        startedAt: at(3.2),
        firstTokenAt: at(3.6),
        completedAt: at(7.2),
        outcome: "tool-call",
        consumedEventIds: ["event-luna-spawn"],
      },
      {
        id: "luna-g2",
        agentId: "agent-luna",
        sequence: 2,
        startedAt: at(8),
        firstTokenAt: at(8.4),
        completedAt: at(13),
        outcome: "message",
        consumedEventIds: ["event-main-followup"],
      },
      {
        id: "luna-g3",
        agentId: "agent-luna",
        sequence: 3,
        startedAt: at(20),
        firstTokenAt: at(20.35),
        completedAt: at(24),
        outcome: "final",
        consumedEventIds: ["event-main-review-request"],
      },
      {
        id: "vera-g1",
        agentId: "agent-vera",
        sequence: 1,
        startedAt: at(5.8),
        firstTokenAt: at(6.2),
        completedAt: at(10.8),
        outcome: "final",
        consumedEventIds: ["event-vera-spawn"],
      },
    ],
    operations: [
      {
        id: "terminal-183",
        kind: "terminal",
        label: "terminal · session 183",
        emittedByGenerationId: "main-g1",
        startedAt: at(2.3),
        yieldedAt: at(2.75),
        completedAt: at(14),
        completionEventId: "event-terminal-complete",
      },
    ],
    events: [
      {
        id: "event-user-input",
        kind: "user-input",
        label: "user input",
        occurredAt: at(0),
        enqueuedAt: at(0),
        emittedByGenerationId: null,
        sourceOperationId: null,
        sourceAgentId: null,
        targetAgentId: "agent-main",
        consumedByGenerationId: "main-g1",
      },
      {
        id: "event-terminal-yield",
        kind: "tool-yield",
        label: "session id yielded",
        occurredAt: at(2.75),
        enqueuedAt: at(2.76),
        emittedByGenerationId: null,
        sourceOperationId: "terminal-183",
        sourceAgentId: null,
        targetAgentId: "agent-main",
        consumedByGenerationId: "main-g2",
      },
      {
        id: "event-luna-spawn",
        kind: "agent-spawn",
        label: "spawn Luna",
        occurredAt: at(2.9),
        enqueuedAt: at(2.9),
        emittedByGenerationId: "main-g1",
        sourceOperationId: null,
        sourceAgentId: "agent-main",
        targetAgentId: "agent-luna",
        consumedByGenerationId: "luna-g1",
      },
      {
        id: "event-main-followup",
        kind: "agent-message",
        label: "follow-up",
        occurredAt: at(5),
        enqueuedAt: at(5.05),
        emittedByGenerationId: "main-g2",
        sourceOperationId: null,
        sourceAgentId: "agent-main",
        targetAgentId: "agent-luna",
        consumedByGenerationId: "luna-g2",
      },
      {
        id: "event-vera-spawn",
        kind: "agent-spawn",
        label: "spawn Vera",
        occurredAt: at(2.95),
        enqueuedAt: at(2.95),
        emittedByGenerationId: "main-g1",
        sourceOperationId: null,
        sourceAgentId: "agent-main",
        targetAgentId: "agent-vera",
        consumedByGenerationId: "vera-g1",
      },
      {
        id: "event-vera-return",
        kind: "agent-return",
        label: "investigation return",
        occurredAt: at(10.5),
        enqueuedAt: at(10.55),
        emittedByGenerationId: "vera-g1",
        sourceOperationId: null,
        sourceAgentId: "agent-vera",
        targetAgentId: "agent-main",
        consumedByGenerationId: "main-g3",
      },
      {
        id: "event-luna-checkpoint",
        kind: "agent-message",
        label: "checkpoint",
        occurredAt: at(12.8),
        enqueuedAt: at(12.85),
        emittedByGenerationId: "luna-g2",
        sourceOperationId: null,
        sourceAgentId: "agent-luna",
        targetAgentId: "agent-main",
        consumedByGenerationId: "main-g3",
      },
      {
        id: "event-terminal-complete",
        kind: "operation-completion",
        label: "exit 0",
        occurredAt: at(14),
        enqueuedAt: at(14.05),
        emittedByGenerationId: null,
        sourceOperationId: "terminal-183",
        sourceAgentId: null,
        targetAgentId: "agent-main",
        consumedByGenerationId: "main-g3",
      },
      {
        id: "event-main-review-request",
        kind: "agent-message",
        label: "review request",
        occurredAt: at(19.5),
        enqueuedAt: at(19.55),
        emittedByGenerationId: "main-g3",
        sourceOperationId: null,
        sourceAgentId: "agent-main",
        targetAgentId: "agent-luna",
        consumedByGenerationId: "luna-g3",
      },
      {
        id: "event-luna-return",
        kind: "agent-return",
        label: "final checkpoint",
        occurredAt: at(23.7),
        enqueuedAt: at(23.75),
        emittedByGenerationId: "luna-g3",
        sourceOperationId: null,
        sourceAgentId: "agent-luna",
        targetAgentId: "agent-main",
        consumedByGenerationId: "main-g4",
      },
    ],
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function timestamp(value: unknown): string | null {
  return typeof value === "string" && !Number.isNaN(Date.parse(value))
    ? value
    : null;
}

function stringArray(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((item) => typeof item === "string")
    ? value
    : null;
}

/** Validate references and ordering, then copy into the closed Web DTO. */
export function parseRuntimeTrace(value: unknown): RuntimeTrace | null {
  if (
    !isRecord(value) ||
    value.schemaVersion !== RUNTIME_TRACE_SCHEMA_VERSION ||
    typeof value.traceId !== "string" ||
    timestamp(value.startedAt) === null ||
    timestamp(value.capturedAt) === null ||
    !Array.isArray(value.agents) ||
    !Array.isArray(value.generations) ||
    !Array.isArray(value.operations) ||
    !Array.isArray(value.events)
  ) {
    return null;
  }

  const agents: AgentLane[] = [];
  for (const raw of value.agents) {
    if (
      !isRecord(raw) ||
      typeof raw.id !== "string" ||
      (raw.parentAgentId !== null && typeof raw.parentAgentId !== "string") ||
      (raw.spawnedByGenerationId !== null &&
        typeof raw.spawnedByGenerationId !== "string") ||
      typeof raw.label !== "string" ||
      typeof raw.model !== "string"
    ) {
      return null;
    }
    agents.push({
      id: raw.id,
      parentAgentId: raw.parentAgentId,
      spawnedByGenerationId: raw.spawnedByGenerationId,
      label: raw.label,
      model: raw.model,
    });
  }

  const generations: GenerationSpan[] = [];
  for (const raw of value.generations) {
    const consumedEventIds = isRecord(raw)
      ? stringArray(raw.consumedEventIds)
      : null;
    if (
      !isRecord(raw) ||
      typeof raw.id !== "string" ||
      typeof raw.agentId !== "string" ||
      typeof raw.sequence !== "number" ||
      !Number.isInteger(raw.sequence) ||
      timestamp(raw.startedAt) === null ||
      timestamp(raw.firstTokenAt) === null ||
      timestamp(raw.completedAt) === null ||
      !(["tool-call", "message", "final"] as unknown[]).includes(raw.outcome) ||
      consumedEventIds === null
    ) {
      return null;
    }
    generations.push({
      id: raw.id,
      agentId: raw.agentId,
      sequence: raw.sequence,
      startedAt: raw.startedAt as string,
      firstTokenAt: raw.firstTokenAt as string,
      completedAt: raw.completedAt as string,
      outcome: raw.outcome as GenerationOutcome,
      consumedEventIds,
    });
  }

  const operations: OperationSpan[] = [];
  for (const raw of value.operations) {
    if (
      !isRecord(raw) ||
      typeof raw.id !== "string" ||
      !(["terminal", "wait", "agent-control", "tool"] as unknown[]).includes(
        raw.kind,
      ) ||
      typeof raw.label !== "string" ||
      typeof raw.emittedByGenerationId !== "string" ||
      timestamp(raw.startedAt) === null ||
      (raw.yieldedAt !== null && timestamp(raw.yieldedAt) === null) ||
      timestamp(raw.completedAt) === null ||
      typeof raw.completionEventId !== "string"
    ) {
      return null;
    }
    operations.push({
      id: raw.id,
      kind: raw.kind as OperationKind,
      label: raw.label,
      emittedByGenerationId: raw.emittedByGenerationId,
      startedAt: raw.startedAt as string,
      yieldedAt: raw.yieldedAt as string | null,
      completedAt: raw.completedAt as string,
      completionEventId: raw.completionEventId,
    });
  }

  const events: RuntimeEvent[] = [];
  for (const raw of value.events) {
    if (
      !isRecord(raw) ||
      typeof raw.id !== "string" ||
      !(
        [
          "user-input",
          "tool-yield",
          "operation-completion",
          "agent-spawn",
          "agent-message",
          "agent-return",
        ] as unknown[]
      ).includes(raw.kind) ||
      typeof raw.label !== "string" ||
      timestamp(raw.occurredAt) === null ||
      timestamp(raw.enqueuedAt) === null ||
      (raw.emittedByGenerationId !== null &&
        typeof raw.emittedByGenerationId !== "string") ||
      (raw.sourceOperationId !== null &&
        typeof raw.sourceOperationId !== "string") ||
      (raw.sourceAgentId !== null && typeof raw.sourceAgentId !== "string") ||
      typeof raw.targetAgentId !== "string" ||
      typeof raw.consumedByGenerationId !== "string"
    ) {
      return null;
    }
    events.push({
      id: raw.id,
      kind: raw.kind as RuntimeEventKind,
      label: raw.label,
      occurredAt: raw.occurredAt as string,
      enqueuedAt: raw.enqueuedAt as string,
      emittedByGenerationId: raw.emittedByGenerationId,
      sourceOperationId: raw.sourceOperationId,
      sourceAgentId: raw.sourceAgentId,
      targetAgentId: raw.targetAgentId,
      consumedByGenerationId: raw.consumedByGenerationId,
    });
  }

  const unique = (items: { id: string }[]) =>
    new Set(items.map((item) => item.id)).size === items.length;
  if (
    !unique(agents) ||
    !unique(generations) ||
    !unique(operations) ||
    !unique(events)
  )
    return null;

  const agentIds = new Set(agents.map((agent) => agent.id));
  const generationById = new Map(
    generations.map((generation) => [generation.id, generation]),
  );
  const operationById = new Map(
    operations.map((operation) => [operation.id, operation]),
  );
  const eventById = new Map(events.map((event) => [event.id, event]));
  const start = Date.parse(value.startedAt as string);
  const end = Date.parse(value.capturedAt as string);
  if (
    end < start ||
    agents.filter((agent) => agent.parentAgentId === null).length !== 1
  )
    return null;

  for (const agent of agents) {
    if (
      (agent.parentAgentId !== null && !agentIds.has(agent.parentAgentId)) ||
      (agent.spawnedByGenerationId !== null &&
        !generationById.has(agent.spawnedByGenerationId)) ||
      (agent.parentAgentId === null) !== (agent.spawnedByGenerationId === null)
    )
      return null;

    if (agent.parentAgentId !== null) {
      const spawningGeneration = generationById.get(
        agent.spawnedByGenerationId!,
      );
      if (spawningGeneration?.agentId !== agent.parentAgentId) return null;
    }

    const ancestors = new Set<string>([agent.id]);
    let parentId = agent.parentAgentId;
    while (parentId !== null) {
      if (ancestors.has(parentId)) return null;
      ancestors.add(parentId);
      parentId = agents.find(
        (candidate) => candidate.id === parentId,
      )!.parentAgentId;
    }
  }

  const generationSequences = new Set<string>();
  for (const generation of generations) {
    const generationStart = Date.parse(generation.startedAt);
    const firstToken = Date.parse(generation.firstTokenAt);
    const generationEnd = Date.parse(generation.completedAt);
    const sequenceKey = `${generation.agentId}:${generation.sequence}`;
    if (
      !agentIds.has(generation.agentId) ||
      generation.sequence < 1 ||
      generationSequences.has(sequenceKey) ||
      generationStart < start ||
      generationEnd > end ||
      generationStart > firstToken ||
      firstToken > generationEnd ||
      new Set(generation.consumedEventIds).size !==
        generation.consumedEventIds.length ||
      generation.consumedEventIds.some(
        (id) => eventById.get(id)?.consumedByGenerationId !== generation.id,
      )
    )
      return null;
    generationSequences.add(sequenceKey);
  }
  for (const operation of operations) {
    const emitter = generationById.get(operation.emittedByGenerationId);
    const operationStart = Date.parse(operation.startedAt);
    const yieldedAt =
      operation.yieldedAt === null
        ? operationStart
        : Date.parse(operation.yieldedAt);
    const operationEnd = Date.parse(operation.completedAt);
    const completion = eventById.get(operation.completionEventId);
    if (
      emitter === undefined ||
      operationStart < start ||
      operationEnd > end ||
      operationStart < Date.parse(emitter.startedAt) ||
      operationStart > Date.parse(emitter.completedAt) ||
      operationStart > yieldedAt ||
      yieldedAt > operationEnd ||
      completion?.kind !== "operation-completion" ||
      completion.sourceOperationId !== operation.id ||
      Date.parse(completion.occurredAt) !== operationEnd
    )
      return null;
  }
  for (const event of events) {
    const consumer = generationById.get(event.consumedByGenerationId);
    const emitter =
      event.emittedByGenerationId === null
        ? undefined
        : generationById.get(event.emittedByGenerationId);
    const sourceOperation =
      event.sourceOperationId === null
        ? undefined
        : operationById.get(event.sourceOperationId);
    const occurredAt = Date.parse(event.occurredAt);
    const enqueuedAt = Date.parse(event.enqueuedAt);
    if (
      consumer === undefined ||
      consumer.agentId !== event.targetAgentId ||
      !agentIds.has(event.targetAgentId) ||
      (event.sourceAgentId !== null && !agentIds.has(event.sourceAgentId)) ||
      (event.emittedByGenerationId !== null && emitter === undefined) ||
      (event.sourceOperationId !== null && sourceOperation === undefined) ||
      occurredAt < start ||
      enqueuedAt > end ||
      occurredAt > enqueuedAt ||
      enqueuedAt > Date.parse(consumer.startedAt) ||
      !consumer.consumedEventIds.includes(event.id)
    )
      return null;

    if (event.kind === "user-input") {
      if (
        event.emittedByGenerationId !== null ||
        event.sourceOperationId !== null ||
        event.sourceAgentId !== null
      )
        return null;
    } else if (
      event.kind === "tool-yield" ||
      event.kind === "operation-completion"
    ) {
      if (
        sourceOperation === undefined ||
        event.emittedByGenerationId !== null ||
        event.sourceAgentId !== null ||
        occurredAt < Date.parse(sourceOperation.startedAt) ||
        occurredAt > Date.parse(sourceOperation.completedAt) ||
        (event.kind === "tool-yield" &&
          (sourceOperation.yieldedAt === null ||
            occurredAt !== Date.parse(sourceOperation.yieldedAt))) ||
        (event.kind === "operation-completion" &&
          sourceOperation.completionEventId !== event.id)
      )
        return null;
    } else {
      if (
        emitter === undefined ||
        event.sourceOperationId !== null ||
        event.sourceAgentId !== emitter.agentId ||
        occurredAt < Date.parse(emitter.startedAt)
      )
        return null;

      if (event.kind === "agent-spawn") {
        const spawnedAgent = agents.find(
          (agent) => agent.id === event.targetAgentId,
        );
        if (
          spawnedAgent?.parentAgentId !== event.sourceAgentId ||
          spawnedAgent.spawnedByGenerationId !== event.emittedByGenerationId ||
          occurredAt < Date.parse(emitter.completedAt)
        )
          return null;
      } else if (occurredAt > Date.parse(emitter.completedAt)) return null;
    }
  }

  return {
    schemaVersion: RUNTIME_TRACE_SCHEMA_VERSION,
    traceId: value.traceId,
    startedAt: value.startedAt as string,
    capturedAt: value.capturedAt as string,
    agents,
    generations,
    operations,
    events,
  };
}

export function deriveRuntimeTraceStats(
  trace: RuntimeTrace,
): RuntimeTraceStats {
  const root = trace.agents.find((agent) => agent.parentAgentId === null);
  const wallTimeMs = Date.parse(trace.capturedAt) - Date.parse(trace.startedAt);
  const asyncIntervals = [
    ...trace.operations.map(
      (operation) =>
        [
          Date.parse(operation.startedAt),
          Date.parse(operation.completedAt),
        ] as const,
    ),
    ...trace.agents
      .filter((agent) => agent.parentAgentId !== null)
      .flatMap((agent) => {
        const spans = trace.generations.filter(
          (generation) => generation.agentId === agent.id,
        );
        if (spans.length === 0) return [];
        return [
          [
            Math.min(...spans.map((span) => Date.parse(span.startedAt))),
            Math.max(...spans.map((span) => Date.parse(span.completedAt))),
          ] as const,
        ];
      }),
  ];

  const points = [
    ...trace.generations.flatMap((span) => [
      { time: Date.parse(span.startedAt), delta: 1 },
      { time: Date.parse(span.completedAt), delta: -1 },
    ]),
    ...trace.operations.flatMap((span) => [
      { time: Date.parse(span.startedAt), delta: 1 },
      { time: Date.parse(span.completedAt), delta: -1 },
    ]),
  ].sort((a, b) => a.time - b.time || a.delta - b.delta);
  let concurrent = 0;
  let maxConcurrency = 0;
  for (const point of points) {
    concurrent += point.delta;
    maxConcurrency = Math.max(maxConcurrency, concurrent);
  }

  const sortedIntervals = [...asyncIntervals].sort((a, b) => a[0] - b[0]);
  let overlapMs = 0;
  let current: readonly [number, number] | null = null;
  for (const interval of sortedIntervals) {
    if (current === null) current = interval;
    else if (interval[0] <= current[1])
      current = [current[0], Math.max(current[1], interval[1])];
    else {
      overlapMs += current[1] - current[0];
      current = interval;
    }
  }
  if (current !== null) overlapMs += current[1] - current[0];

  const queueDelays = trace.events
    .filter((event) => event.kind !== "user-input")
    .map(
      (event) =>
        Date.parse(
          trace.generations.find(
            (generation) => generation.id === event.consumedByGenerationId,
          )!.startedAt,
        ) - Date.parse(event.enqueuedAt),
    )
    .sort((a, b) => a - b);
  const middle = Math.floor(queueDelays.length / 2);
  const medianQueueDelayMs =
    queueDelays.length === 0
      ? 0
      : queueDelays.length % 2 === 0
        ? ((queueDelays[middle - 1] ?? 0) + (queueDelays[middle] ?? 0)) / 2
        : (queueDelays[middle] ?? 0);

  return {
    wallTimeMs,
    mainGenerations: trace.generations.filter(
      (generation) => generation.agentId === root?.id,
    ).length,
    totalGenerations: trace.generations.length,
    asyncBranches:
      trace.operations.length +
      trace.agents.filter((agent) => agent.parentAgentId !== null).length,
    agentMessages: trace.events.filter(
      (event) =>
        event.kind === "agent-message" || event.kind === "agent-return",
    ).length,
    maxConcurrency,
    asyncOverlapRatio: wallTimeMs === 0 ? 0 : overlapMs / wallTimeMs,
    medianQueueDelayMs,
    maxQueueDelayMs: queueDelays.at(-1) ?? 0,
  };
}

export async function loadRuntimeTrace(): Promise<RuntimeTraceResult> {
  const trace = parseRuntimeTrace(capturedRuntimeTracePayload);
  if (trace === null) throw new Error("captured runtime trace is invalid");
  return { trace, source: "captured" };
}
