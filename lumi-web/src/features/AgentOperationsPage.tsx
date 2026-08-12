import { useEffect, useMemo, useState } from "react";

import {
  deriveRuntimeTraceStats,
  loadRuntimeTrace,
} from "../api/runtime-trace";
import type {
  AgentLane,
  GenerationSpan,
  OperationSpan,
  RuntimeEvent,
  RuntimeTrace,
  RuntimeTraceResult,
  RuntimeTraceStats,
} from "../api/runtime-trace";

export type RuntimeTraceLoader = () => Promise<RuntimeTraceResult>;

type PageState =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; result: RuntimeTraceResult };

type Selection =
  | { type: "generation"; id: string }
  | { type: "operation"; id: string }
  | { type: "event"; id: string };

export interface AgentOperationsPageProps {
  loader?: RuntimeTraceLoader;
}

export function AgentOperationsPage({
  loader = loadRuntimeTrace,
}: AgentOperationsPageProps) {
  const [state, setState] = useState<PageState>({ phase: "loading" });
  const [selection, setSelection] = useState<Selection | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let disposed = false;
    setState({ phase: "loading" });
    loader()
      .then((result) => {
        if (!disposed) setState({ phase: "ready", result });
      })
      .catch((error: unknown) => {
        if (disposed) return;
        setState({
          phase: "error",
          message:
            error instanceof Error ? error.message : "Unable to load trace.",
        });
      });
    return () => {
      disposed = true;
    };
  }, [loader, reloadKey]);

  let body: React.ReactNode;
  if (state.phase === "loading") {
    body = (
      <div className="state-panel" role="status">
        Loading runtime trace…
      </div>
    );
  } else if (state.phase === "error") {
    body = (
      <div className="state-panel state-panel--error" role="alert">
        <p>Could not load runtime trace: {state.message}</p>
        <button
          type="button"
          className="button"
          onClick={() => setReloadKey((key) => key + 1)}
        >
          Try again
        </button>
      </div>
    );
  } else if (state.result.trace.generations.length === 0) {
    body = (
      <div className="state-panel">This trace contains no generations.</div>
    );
  } else {
    body = (
      <RuntimeTraceWorkspace
        trace={state.result.trace}
        selection={selection}
        onSelect={setSelection}
      />
    );
  }

  return (
    <section className="page" aria-busy={state.phase === "loading"}>
      <header className="page__header">
        <div>
          <h1 className="page__title">Agent Operations</h1>
          <p className="page__subtitle">
            Causal runtime trace · generations, async branches, events, joins.
          </p>
        </div>
        <div className="page__actions">
          {state.phase === "ready" ? (
            <span className="source-badge source-badge--fixture">
              Fixture trace
            </span>
          ) : null}
          <button
            type="button"
            className="button"
            onClick={() => {
              setSelection(null);
              setReloadKey((key) => key + 1);
            }}
            disabled={state.phase === "loading"}
          >
            Replay trace
          </button>
        </div>
      </header>
      {body}
    </section>
  );
}

interface RuntimeTraceWorkspaceProps {
  trace: RuntimeTrace;
  selection: Selection | null;
  onSelect: (selection: Selection | null) => void;
}

function RuntimeTraceWorkspace({
  trace,
  selection,
  onSelect,
}: RuntimeTraceWorkspaceProps) {
  const stats = useMemo(() => deriveRuntimeTraceStats(trace), [trace]);

  return (
    <div className="trace-workspace">
      <RuntimeStats stats={stats} />
      <div className="trace-workspace__body">
        <RuntimeTimeline
          trace={trace}
          selection={selection}
          onSelect={onSelect}
        />
        <TraceInspector trace={trace} selection={selection} />
      </div>
    </div>
  );
}

function RuntimeStats({ stats }: { stats: RuntimeTraceStats }) {
  const cards = [
    ["Wall time", formatDuration(stats.wallTimeMs)],
    ["Main generations", String(stats.mainGenerations)],
    ["All generations", String(stats.totalGenerations)],
    ["Async branches", String(stats.asyncBranches)],
    ["Agent messages", String(stats.agentMessages)],
    ["Max concurrency", `${stats.maxConcurrency}×`],
    ["Async coverage", `${Math.round(stats.asyncOverlapRatio * 100)}%`],
    ["Median queue", formatDuration(stats.medianQueueDelayMs)],
  ];
  return (
    <section className="trace-stats" aria-label="Runtime statistics">
      {cards.map(([label, value]) => (
        <div className="trace-stat" key={label}>
          <span>{label}</span>
          <strong>{value}</strong>
        </div>
      ))}
    </section>
  );
}

const LEFT_GUTTER = 172;
const RIGHT_GUTTER = 50;
const PX_PER_SECOND = 28;
const RULER_Y = 42;
const FIRST_LANE_Y = 104;
const LANE_GAP = 112;
const GENERATION_HEIGHT = 52;

interface LaneRow {
  key: string;
  type: "agent" | "operation";
  label: string;
  sublabel: string;
  y: number;
  agent?: AgentLane;
  operation?: OperationSpan;
}

function RuntimeTimeline({
  trace,
  selection,
  onSelect,
}: RuntimeTraceWorkspaceProps) {
  const startedAt = Date.parse(trace.startedAt);
  const durationSeconds = Math.max(
    1,
    (Date.parse(trace.capturedAt) - startedAt) / 1_000,
  );
  const width = LEFT_GUTTER + durationSeconds * PX_PER_SECOND + RIGHT_GUTTER;

  const root = trace.agents.find((agent) => agent.parentAgentId === null);
  const childAgents = trace.agents.filter(
    (agent) => agent.parentAgentId !== null,
  );
  const childAgentIds = new Set(childAgents.map((agent) => agent.id));
  const childGenerations = trace.generations
    .filter((generation) => childAgentIds.has(generation.agentId))
    .sort((a, b) => Date.parse(a.startedAt) - Date.parse(b.startedAt));
  const childTrackByGeneration = new Map<string, number>();
  const childTrackEnds: number[] = [];
  for (const generation of childGenerations) {
    const start = Date.parse(generation.startedAt);
    let track = childTrackEnds.findIndex((end) => end <= start);
    if (track === -1) track = childTrackEnds.length;
    childTrackEnds[track] = Date.parse(generation.completedAt);
    childTrackByGeneration.set(generation.id, track);
  }
  const rows: LaneRow[] = [];
  if (root !== undefined) {
    rows.push({
      key: root.id,
      type: "agent",
      label: root.label,
      sublabel: root.model,
      y: FIRST_LANE_Y,
      agent: root,
    });
  }
  for (const operation of trace.operations) {
    rows.push({
      key: operation.id,
      type: "operation",
      label: operation.label,
      sublabel: operation.kind,
      y: FIRST_LANE_Y + rows.length * LANE_GAP,
      operation,
    });
  }
  if (childAgents.length > 0) {
    rows.push({
      key: "subagents",
      type: "agent",
      label: "Subagents",
      sublabel: `${childAgents.length} resources · pooled`,
      y: FIRST_LANE_Y + rows.length * LANE_GAP,
    });
  }
  const subagentRow = rows.find((row) => row.key === "subagents");
  const childTrackCount = Math.max(1, childTrackEnds.length);
  const height =
    FIRST_LANE_Y +
    rows.length * LANE_GAP -
    24 +
    Math.max(0, childTrackCount - 1) * 64;
  const rowByAgent = new Map(
    rows
      .filter((row) => row.agent !== undefined)
      .map((row) => [row.agent!.id, row]),
  );
  const rowByOperation = new Map(
    rows
      .filter((row) => row.operation !== undefined)
      .map((row) => [row.operation!.id, row]),
  );
  const generationById = new Map(
    trace.generations.map((generation) => [generation.id, generation]),
  );
  const x = (time: string) =>
    LEFT_GUTTER + ((Date.parse(time) - startedAt) / 1_000) * PX_PER_SECOND;
  const agentY = (agentId: string, generationId?: string) => {
    const directRow = rowByAgent.get(agentId);
    if (directRow !== undefined) return directRow.y;
    const track =
      generationId === undefined
        ? 0
        : (childTrackByGeneration.get(generationId) ?? 0);
    return (
      (subagentRow?.y ?? FIRST_LANE_Y) +
      track * 64 -
      ((childTrackCount - 1) * 64) / 2
    );
  };
  const generationY = (generation: GenerationSpan) =>
    agentY(generation.agentId, generation.id);
  const eventSourceY = (event: RuntimeEvent) => {
    if (event.sourceOperationId !== null)
      return rowByOperation.get(event.sourceOperationId)?.y;
    if (event.emittedByGenerationId !== null) {
      const generation = generationById.get(event.emittedByGenerationId);
      if (generation !== undefined) return generationY(generation);
    }
    if (event.sourceAgentId !== null) return agentY(event.sourceAgentId);
    return agentY(event.targetAgentId);
  };
  const ticks = Array.from(
    { length: Math.floor(durationSeconds / 5) + 1 },
    (_, index) => index * 5,
  );

  return (
    <section className="timeline-panel" aria-label="Causal runtime timeline">
      <div className="timeline-panel__legend" aria-label="Timeline legend">
        <span>
          <i className="legend-mark legend-mark--generation" />
          generation
        </span>
        <span>
          <i className="legend-mark legend-mark--operation" />
          operation
        </span>
        <span>
          <i className="legend-mark legend-mark--message" />
          agent event
        </span>
        <span>
          <i className="legend-mark legend-mark--completion" />
          completion
        </span>
      </div>
      <div className="timeline-scroll">
        <div
          className="timeline-canvas"
          style={{ width, height }}
          onClick={(event) => {
            if (event.target === event.currentTarget) onSelect(null);
          }}
        >
          <svg
            className="timeline-lines"
            width={width}
            height={height}
            aria-hidden="true"
          >
            <defs>
              <marker
                id="arrow-message"
                viewBox="0 0 10 10"
                refX="8"
                refY="5"
                markerWidth="5"
                markerHeight="5"
                orient="auto-start-reverse"
              >
                <path d="M 0 0 L 10 5 L 0 10 z" />
              </marker>
              <marker
                id="arrow-completion"
                viewBox="0 0 10 10"
                refX="8"
                refY="5"
                markerWidth="5"
                markerHeight="5"
                orient="auto-start-reverse"
              >
                <path d="M 0 0 L 10 5 L 0 10 z" />
              </marker>
            </defs>
            {ticks.map((tick) => (
              <g key={tick}>
                <line
                  className="timeline-gridline"
                  x1={LEFT_GUTTER + tick * PX_PER_SECOND}
                  x2={LEFT_GUTTER + tick * PX_PER_SECOND}
                  y1={RULER_Y}
                  y2={height}
                />
                <text
                  className="timeline-tick"
                  x={LEFT_GUTTER + tick * PX_PER_SECOND}
                  y={RULER_Y - 12}
                >
                  +{tick}s
                </text>
              </g>
            ))}
            {rows.map((row) => (
              <line
                key={row.key}
                className="timeline-lane-line"
                x1={LEFT_GUTTER}
                x2={width - RIGHT_GUTTER}
                y1={row.y}
                y2={row.y}
              />
            ))}
            {trace.events
              .filter((event) => event.kind !== "user-input")
              .map((event) => {
                const consumer = generationById.get(
                  event.consumedByGenerationId,
                );
                if (consumer === undefined) return null;
                const sourceY = eventSourceY(event);
                if (sourceY === undefined) return null;
                const x1 = x(event.occurredAt);
                const y1 = sourceY;
                const x2 = x(consumer.startedAt) + 2;
                const y2 = generationY(consumer);
                const curve = Math.max(20, Math.abs(x2 - x1) * 0.45);
                const completion = event.kind === "operation-completion";
                return (
                  <path
                    key={event.id}
                    className={
                      completion
                        ? "causal-arrow causal-arrow--completion"
                        : "causal-arrow causal-arrow--message"
                    }
                    d={`M ${x1} ${y1} C ${x1 + curve} ${y1}, ${x2 - curve} ${y2}, ${x2} ${y2}`}
                    markerEnd={`url(#${completion ? "arrow-completion" : "arrow-message"})`}
                  />
                );
              })}
            {trace.operations.map((operation) => {
              const emitter = generationById.get(
                operation.emittedByGenerationId,
              );
              const row = rowByOperation.get(operation.id);
              if (emitter === undefined || row === undefined) return null;
              const x1 = x(operation.startedAt);
              const y1 = generationY(emitter);
              return (
                <path
                  key={`branch-${operation.id}`}
                  className="causal-arrow causal-arrow--branch"
                  d={`M ${x1} ${y1} C ${x1 + 18} ${y1}, ${x1 - 18} ${row.y}, ${x1} ${row.y}`}
                />
              );
            })}
          </svg>

          {rows.map((row) => (
            <div
              className={`timeline-lane-label timeline-lane-label--${row.type}`}
              style={{ top: row.y - 22 }}
              key={row.key}
            >
              <strong>{row.label}</strong>
              <span>{row.sublabel}</span>
            </div>
          ))}

          {trace.generations.map((generation) => {
            const agent = trace.agents.find(
              (item) => item.id === generation.agentId,
            );
            const left = x(generation.startedAt);
            const spanWidth = Math.max(
              74,
              x(generation.completedAt) - x(generation.startedAt),
            );
            const selected =
              selection?.type === "generation" &&
              selection.id === generation.id;
            return (
              <button
                type="button"
                className={`generation-span generation-span--${generation.outcome}${selected ? " is-selected" : ""}`}
                style={{
                  left,
                  top: generationY(generation) - GENERATION_HEIGHT / 2,
                  width: spanWidth,
                  height: GENERATION_HEIGHT,
                }}
                aria-pressed={selected}
                onClick={() =>
                  onSelect({ type: "generation", id: generation.id })
                }
                key={generation.id}
              >
                <strong>{generation.id}</strong>
                <span>
                  {agent !== undefined && agent.parentAgentId !== null
                    ? `${agent?.label.split(" · ")[0] ?? generation.agentId} · `
                    : ""}
                  {generation.outcome}
                </span>
              </button>
            );
          })}

          {trace.operations.map((operation) => {
            const row = rowByOperation.get(operation.id);
            if (row === undefined) return null;
            const left = x(operation.startedAt);
            const opWidth = x(operation.completedAt) - left;
            const selected =
              selection?.type === "operation" && selection.id === operation.id;
            return (
              <button
                type="button"
                className={`operation-span${selected ? " is-selected" : ""}`}
                style={{ left, top: row.y - 12, width: opWidth }}
                aria-pressed={selected}
                aria-label={`${operation.label}, ${formatDuration(Date.parse(operation.completedAt) - Date.parse(operation.startedAt))}`}
                onClick={() =>
                  onSelect({ type: "operation", id: operation.id })
                }
                key={operation.id}
              >
                <i
                  className="operation-span__yield"
                  style={{
                    left: `${((Date.parse(operation.yieldedAt) - Date.parse(operation.startedAt)) / (Date.parse(operation.completedAt) - Date.parse(operation.startedAt))) * 100}%`,
                  }}
                />
              </button>
            );
          })}

          {trace.events.map((event) => {
            const sourceY = eventSourceY(event);
            if (sourceY === undefined) return null;
            const selected =
              selection?.type === "event" && selection.id === event.id;
            return (
              <button
                type="button"
                className={`runtime-event runtime-event--${event.kind}${selected ? " is-selected" : ""}`}
                style={{ left: x(event.occurredAt) - 7, top: sourceY - 7 }}
                aria-label={`${event.kind}: ${event.label}`}
                aria-pressed={selected}
                onClick={() => onSelect({ type: "event", id: event.id })}
                key={event.id}
              />
            );
          })}
          {trace.events
            .filter(
              (event) =>
                event.kind !== "user-input" &&
                event.kind !== "tool-yield" &&
                event.kind !== "agent-spawn",
            )
            .map((event) => {
              const sourceY = eventSourceY(event);
              if (sourceY === undefined) return null;
              return (
                <span
                  className={`runtime-event-label runtime-event-label--${event.kind}`}
                  style={{
                    left: x(event.occurredAt) + 8,
                    top:
                      sourceY + (event.sourceOperationId !== null ? 17 : -31),
                  }}
                  key={`label-${event.id}`}
                >
                  {event.label}
                </span>
              );
            })}
        </div>
      </div>
    </section>
  );
}

function TraceInspector({
  trace,
  selection,
}: {
  trace: RuntimeTrace;
  selection: Selection | null;
}) {
  if (selection === null) {
    return (
      <aside className="trace-inspector" aria-label="Trace inspector">
        <p className="eyebrow">Trace</p>
        <h2>{trace.traceId}</h2>
        <p className="trace-inspector__hint">
          Select a generation, operation, or event. The trace exposes timing and
          causality only; content stays behind references.
        </p>
        <FactList
          facts={[
            ["Agents", String(trace.agents.length)],
            ["Operations", String(trace.operations.length)],
            ["Events", String(trace.events.length)],
            ["Captured", formatClock(trace.capturedAt)],
          ]}
        />
      </aside>
    );
  }

  if (selection.type === "generation") {
    const generation = trace.generations.find(
      (item) => item.id === selection.id,
    );
    if (generation === undefined) return null;
    const agent = trace.agents.find((item) => item.id === generation.agentId);
    return (
      <aside className="trace-inspector" aria-label="Trace inspector">
        <p className="eyebrow">Generation</p>
        <h2>{generation.id}</h2>
        <span className={`fact-badge fact-badge--${generation.outcome}`}>
          {generation.outcome}
        </span>
        <FactList
          facts={[
            ["Agent", agent?.label ?? generation.agentId],
            ["Started", formatOffset(trace, generation.startedAt)],
            [
              "TTFT",
              formatDuration(
                Date.parse(generation.firstTokenAt) -
                  Date.parse(generation.startedAt),
              ),
            ],
            [
              "Sampling",
              formatDuration(
                Date.parse(generation.completedAt) -
                  Date.parse(generation.firstTokenAt),
              ),
            ],
            [
              "Duration",
              formatDuration(
                Date.parse(generation.completedAt) -
                  Date.parse(generation.startedAt),
              ),
            ],
            ["Events joined", String(generation.consumedEventIds.length)],
          ]}
        />
        {generation.consumedEventIds.length > 0 ? (
          <div className="joined-events">
            <span>Consumed at start</span>
            {generation.consumedEventIds.map((id) => (
              <code key={id}>{id}</code>
            ))}
          </div>
        ) : null}
      </aside>
    );
  }

  if (selection.type === "operation") {
    const operation = trace.operations.find((item) => item.id === selection.id);
    if (operation === undefined) return null;
    return (
      <aside className="trace-inspector" aria-label="Trace inspector">
        <p className="eyebrow">Async operation</p>
        <h2>{operation.label}</h2>
        <FactList
          facts={[
            ["Kind", operation.kind],
            ["Emitted by", operation.emittedByGenerationId],
            ["Started", formatOffset(trace, operation.startedAt)],
            ["Yielded", formatOffset(trace, operation.yieldedAt)],
            ["Completed", formatOffset(trace, operation.completedAt)],
            [
              "Wall time",
              formatDuration(
                Date.parse(operation.completedAt) -
                  Date.parse(operation.startedAt),
              ),
            ],
            ["Completion event", operation.completionEventId],
          ]}
        />
      </aside>
    );
  }

  const event = trace.events.find((item) => item.id === selection.id);
  if (event === undefined) return null;
  const consumer = trace.generations.find(
    (item) => item.id === event.consumedByGenerationId,
  );
  const queueDelay =
    consumer === undefined
      ? 0
      : Date.parse(consumer.startedAt) - Date.parse(event.enqueuedAt);
  return (
    <aside className="trace-inspector" aria-label="Trace inspector">
      <p className="eyebrow">External event</p>
      <h2>{event.label}</h2>
      <span className={`fact-badge fact-badge--${event.kind}`}>
        {event.kind}
      </span>
      <FactList
        facts={[
          ["Occurred", formatOffset(trace, event.occurredAt)],
          ["Enqueued", formatOffset(trace, event.enqueuedAt)],
          ["Consumed by", event.consumedByGenerationId],
          ["Queue delay", formatDuration(queueDelay)],
          ["Target", event.targetAgentId],
          ["Source", event.sourceOperationId ?? event.sourceAgentId ?? "user"],
        ]}
      />
    </aside>
  );
}

function FactList({ facts }: { facts: [string, string][] }) {
  return (
    <dl className="fact-list">
      {facts.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function formatDuration(ms: number): string {
  if (ms < 1_000) return `${Math.round(ms)}ms`;
  return `${(ms / 1_000).toFixed(ms % 1_000 === 0 ? 0 : 2)}s`;
}

function formatOffset(trace: RuntimeTrace, timestamp: string): string {
  return `+${formatDuration(Date.parse(timestamp) - Date.parse(trace.startedAt))}`;
}

function formatClock(timestamp: string): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(timestamp));
}
