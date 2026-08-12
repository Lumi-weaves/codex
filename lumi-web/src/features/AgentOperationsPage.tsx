import { useCallback, useEffect, useMemo, useState } from "react";

import {
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  ReactFlow,
} from "@xyflow/react";

import {
  AGENT_OPERATION_STATUSES,
  fetchAgentOperationsSnapshot,
} from "../api/agent-operations";
import type {
  AgentOperationNode,
  AgentOperationStatus,
  AgentOperationsResult,
} from "../api/agent-operations";
import { layoutOperationNodes, operationNodeTypes } from "./graph";

export type AgentOperationsLoader = (
  signal?: AbortSignal,
) => Promise<AgentOperationsResult>;

type PageState =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; result: AgentOperationsResult };

export interface AgentOperationsPageProps {
  /** Injectable for tests; defaults to the BFF-or-fixture adapter. */
  loader?: AgentOperationsLoader;
  /** Injectable for tests so happy-dom never drives fit-view geometry. */
  fitView?: boolean;
}

export function AgentOperationsPage({
  loader = fetchAgentOperationsSnapshot,
  fitView = true,
}: AgentOperationsPageProps) {
  const [state, setState] = useState<PageState>({ phase: "loading" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    setState({ phase: "loading" });
    loader(controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) {
          setState({ phase: "ready", result });
        }
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setState({
          phase: "error",
          message:
            error instanceof Error
              ? error.message
              : "Unable to load agent operations.",
        });
      });
    return () => controller.abort();
  }, [loader, reloadKey]);

  const operations = state.phase === "ready" ? state.result.snapshot.nodes : [];
  const selected = useMemo(
    () => operations.find((operation) => operation.id === selectedId) ?? null,
    [operations, selectedId],
  );

  const graph = useMemo(
    () =>
      state.phase === "ready"
        ? layoutOperationNodes(
            state.result.snapshot.nodes,
            selectedId,
            setSelectedId,
          )
        : { nodes: [], edges: [] },
    [selectedId, state],
  );

  const refresh = useCallback(() => {
    setSelectedId(null);
    setReloadKey((key) => key + 1);
  }, []);

  let body: React.ReactNode;
  if (state.phase === "loading") {
    body = (
      <div className="state-panel" role="status">
        Loading agent operations…
      </div>
    );
  } else if (state.phase === "error") {
    body = (
      <div className="state-panel state-panel--error" role="alert">
        <p>Could not load agent operations: {state.message}</p>
        <button type="button" className="button" onClick={refresh}>
          Try again
        </button>
      </div>
    );
  } else if (operations.length === 0) {
    body = (
      <div className="state-panel">
        {state.result.snapshot.isPartial ||
        state.result.snapshot.isTruncated ? (
          <>
            No loaded agent operations were observable.
            <SnapshotLimitation
              isPartial={state.result.snapshot.isPartial}
              isTruncated={state.result.snapshot.isTruncated}
            />
          </>
        ) : (
          "No agent operations are running right now."
        )}
      </div>
    );
  } else {
    body = (
      <div className="operations-workspace">
        <div className="operations-graph">
          <ReactFlow
            colorMode="dark"
            nodes={graph.nodes}
            edges={graph.edges}
            nodeTypes={operationNodeTypes}
            fitView={fitView}
            fitViewOptions={{ padding: 0.15 }}
            minZoom={0.2}
            maxZoom={1.5}
            nodesDraggable={false}
            nodesConnectable={false}
            onPaneClick={() => setSelectedId(null)}
            aria-label="Agent operations graph"
          >
            <Background
              variant={BackgroundVariant.Dots}
              gap={24}
              size={1}
              color="var(--grid-dot)"
            />
            <MiniMap
              pannable
              zoomable
              maskColor="rgba(10, 14, 19, 0.4)"
              nodeColor="#4cc2ff"
              nodeStrokeColor="transparent"
            />
            <Controls showInteractive={false} />
          </ReactFlow>
        </div>
        <OperationDetailPanel
          selected={selected}
          operations={operations}
          capturedAt={state.result.snapshot.capturedAt}
          isPartial={state.result.snapshot.isPartial}
          isTruncated={state.result.snapshot.isTruncated}
          source={state.result.source}
        />
      </div>
    );
  }

  return (
    <section className="page" aria-busy={state.phase === "loading"}>
      <header className="page__header">
        <div>
          <h1 className="page__title">Agent Operations</h1>
          <p className="page__subtitle">
            Read-only view of current agent runs across Lumi Codex.
          </p>
        </div>
        <div className="page__actions">
          {state.phase === "ready" ? (
            <span
              className={`source-badge source-badge--${state.result.source}`}
            >
              {state.result.source === "fixture" ? "Fixture data" : "Live BFF"}
            </span>
          ) : null}
          <button
            type="button"
            className="button"
            onClick={refresh}
            disabled={state.phase === "loading"}
          >
            Refresh
          </button>
        </div>
      </header>
      {body}
    </section>
  );
}

interface OperationDetailPanelProps {
  selected: AgentOperationNode | null;
  operations: AgentOperationNode[];
  capturedAt: string;
  isPartial: boolean;
  isTruncated: boolean;
  source: AgentOperationsResult["source"];
}

function OperationDetailPanel({
  selected,
  operations,
  capturedAt,
  isPartial,
  isTruncated,
  source,
}: OperationDetailPanelProps) {
  const counts = useMemo(() => {
    const byStatus = new Map<AgentOperationStatus, number>();
    for (const operation of operations) {
      byStatus.set(operation.status, (byStatus.get(operation.status) ?? 0) + 1);
    }
    return AGENT_OPERATION_STATUSES.map((status) => ({
      status,
      count: byStatus.get(status) ?? 0,
    })).filter((entry) => entry.count > 0);
  }, [operations]);

  return (
    <aside className="detail-panel" aria-label="Operation details">
      {selected !== null ? (
        <>
          <div className="detail-panel__header">
            <h2>{selected.label}</h2>
            <span className={`status-badge status-badge--${selected.status}`}>
              {selected.status}
            </span>
          </div>
          <dl className="detail-panel__list">
            <div>
              <dt>Role</dt>
              <dd>{selected.role}</dd>
            </div>
            <div>
              <dt>Model</dt>
              <dd>{selected.model ?? "—"}</dd>
            </div>
            <div>
              <dt>Started</dt>
              <dd>
                {selected.startedAt !== null
                  ? new Date(selected.startedAt).toLocaleString()
                  : "—"}
              </dd>
            </div>
            <div>
              <dt>Updated</dt>
              <dd>{new Date(selected.updatedAt).toLocaleString()}</dd>
            </div>
          </dl>
          <p className="detail-panel__activity">{selected.activity}</p>
          <SnapshotLimitation isPartial={isPartial} isTruncated={isTruncated} />
        </>
      ) : (
        <>
          <h2>Snapshot summary</h2>
          <p className="detail-panel__hint">
            Select an operation in the graph to inspect it.
          </p>
          <dl className="detail-panel__list">
            {counts.map(({ status, count }) => (
              <div key={status}>
                <dt>
                  <span
                    className={`operation-node__dot operation-node__dot--${status}`}
                    aria-hidden="true"
                  />
                  {status}
                </dt>
                <dd>{count}</dd>
              </div>
            ))}
          </dl>
          <p className="detail-panel__captured">
            Captured {new Date(capturedAt).toLocaleString()} ·{" "}
            {source === "fixture" ? "fixture" : "live"} source
          </p>
          <SnapshotLimitation isPartial={isPartial} isTruncated={isTruncated} />
        </>
      )}
    </aside>
  );
}

function SnapshotLimitation({
  isPartial,
  isTruncated,
}: {
  isPartial: boolean;
  isTruncated: boolean;
}) {
  if (!isPartial && !isTruncated) return null;
  return (
    <p className="detail-panel__hint" role="status">
      {isPartial && isTruncated
        ? "Partial snapshot · loaded-thread limit reached"
        : isPartial
          ? "Partial snapshot · some thread metadata was unavailable"
          : "Loaded-thread limit reached"}
    </p>
  );
}
