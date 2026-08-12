import { useEffect, useMemo, useState } from "react";

import {
  deriveRuntimeTraceStats,
  loadRuntimeTrace,
} from "../api/runtime-trace";
import type {
  RuntimeTrace,
  RuntimeTraceResult,
  RuntimeTraceStats,
} from "../api/runtime-trace";
import perfettoTraceUrl from "../fixtures/hw1-runtime-trace.pftrace?url";
import { openTraceInPerfetto } from "../integrations/perfetto";

export type RuntimeTraceLoader = () => Promise<RuntimeTraceResult>;
export type PerfettoOpener = (request: {
  traceUrl: string;
  title: string;
  fileName: string;
}) => Promise<void>;

type PageState =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; result: RuntimeTraceResult };

export interface AgentOperationsPageProps {
  loader?: RuntimeTraceLoader;
  perfettoOpener?: PerfettoOpener;
}

export function AgentOperationsPage({
  loader = loadRuntimeTrace,
  perfettoOpener = openTraceInPerfetto,
}: AgentOperationsPageProps) {
  const [state, setState] = useState<PageState>({ phase: "loading" });
  const [launchState, setLaunchState] = useState<
    "idle" | "opening" | "opened" | "error"
  >("idle");
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

  const openPerfetto = () => {
    if (state.phase !== "ready" || launchState === "opening") return;
    setLaunchState("opening");
    void perfettoOpener({
      traceUrl: perfettoTraceUrl,
      title: `Lumi Codex · ${state.result.trace.traceId}`,
      fileName: `${state.result.trace.traceId}.pftrace`,
    }).then(
      () => setLaunchState("opened"),
      () => setLaunchState("error"),
    );
  };

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
      <PerfettoTraceLauncher
        trace={state.result.trace}
        launchState={launchState}
        onOpen={openPerfetto}
      />
    );
  }

  return (
    <section className="page" aria-busy={state.phase === "loading"}>
      <header className="page__header">
        <div>
          <h1 className="page__title">Agent Operations</h1>
          <p className="page__subtitle">
            A content-free Codex runtime trace, explored with Perfetto.
          </p>
        </div>
        {state.phase === "ready" ? (
          <span className={`source-badge source-badge--${state.result.source}`}>
            {state.result.source === "captured"
              ? "Captured on hw1"
              : "Fixture trace"}
          </span>
        ) : null}
      </header>
      {body}
    </section>
  );
}

function PerfettoTraceLauncher({
  trace,
  launchState,
  onOpen,
}: {
  trace: RuntimeTrace;
  launchState: "idle" | "opening" | "opened" | "error";
  onOpen: () => void;
}) {
  const stats = useMemo(() => deriveRuntimeTraceStats(trace), [trace]);

  return (
    <div className="trace-workspace">
      <RuntimeStats stats={stats} />
      <section className="perfetto-launcher">
        <div className="perfetto-launcher__copy">
          <p className="eyebrow">Native trace viewer</p>
          <h2>Open this workflow in Perfetto</h2>
          <p>
            Generations and tool operations are slices. Agent handoffs,
            dispatches, and completions are causal flows. Active operations and
            subagents are counter tracks.
          </p>
          <p className="perfetto-launcher__privacy">
            The exported trace contains timing, aliases, kinds, and causal
            references only—never prompts, model text, commands, terminal
            output, or filesystem paths. Perfetto receives the bytes directly in
            your browser through its supported postMessage API.
          </p>
          <div className="perfetto-launcher__actions">
            <button
              type="button"
              className="button button--primary"
              onClick={onOpen}
              disabled={launchState === "opening"}
            >
              {launchState === "opening"
                ? "Opening Perfetto…"
                : "Open in Perfetto"}
            </button>
            <a
              className="button"
              href={perfettoTraceUrl}
              download={`${trace.traceId}.pftrace`}
            >
              Download .pftrace
            </a>
          </div>
          {launchState === "opened" ? (
            <p className="perfetto-launcher__status" role="status">
              Trace delivered to Perfetto in a new tab.
            </p>
          ) : null}
          {launchState === "error" ? (
            <p
              className="perfetto-launcher__status perfetto-launcher__status--error"
              role="alert"
            >
              Could not open Perfetto. Allow pop-ups for this local page or
              download the trace and open it manually.
            </p>
          ) : null}
        </div>
        <TraceManifest trace={trace} />
      </section>
      <p className="trace-provenance">
        Historical dogfood trace: tool call/output pairing is observed;
        generation boundaries and event consumption are inferred from rollout
        recorder order. Live runtime instrumentation will replace those
        inferences.
      </p>
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

function TraceManifest({ trace }: { trace: RuntimeTrace }) {
  const facts = [
    ["Trace", trace.traceId],
    ["Agents", String(trace.agents.length)],
    ["Generations", String(trace.generations.length)],
    ["Operations", String(trace.operations.length)],
    ["Causal events", String(trace.events.length)],
    ["Format", "Perfetto TrackEvent proto"],
  ];
  return (
    <aside className="trace-manifest" aria-label="Trace manifest">
      <p className="eyebrow">Export manifest</p>
      <dl className="fact-list">
        {facts.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </aside>
  );
}

function formatDuration(ms: number): string {
  if (ms < 1_000) return `${Math.round(ms)}ms`;
  return `${(ms / 1_000).toFixed(ms % 1_000 === 0 ? 0 : 2)}s`;
}
