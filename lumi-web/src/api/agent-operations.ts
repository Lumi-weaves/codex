/**
 * The only backend contract this frontend owns. It is deliberately narrow:
 * a versioned snapshot of agent operations for display. It never models or
 * imports the full app-server protocol, and it never carries prompt text,
 * model output, or filesystem paths.
 */

import { AGENT_OPERATIONS_ENDPOINT } from "./paths";

export const AGENT_OPERATIONS_SCHEMA_VERSION = 1;

export const AGENT_OPERATION_STATUSES = [
  "running",
  "waiting",
  "failed",
  "idle",
] as const;

export type AgentOperationStatus = (typeof AGENT_OPERATION_STATUSES)[number];

export type AgentOperationRole = "root" | "worker";

export interface AgentOperationNode {
  id: string;
  /** Parent operation id, or null for the root of the operation tree. */
  parentId: string | null;
  role: AgentOperationRole;
  label: string;
  status: AgentOperationStatus;
  /** Short human-readable activity line. Never prompt/output content. */
  activity: string;
  model: string | null;
  startedAt: string | null;
  updatedAt: string;
}

export interface AgentOperationsSnapshot {
  schemaVersion: typeof AGENT_OPERATIONS_SCHEMA_VERSION;
  capturedAt: string;
  /** Some loaded thread metadata could not be observed. */
  isPartial: boolean;
  /** More loaded threads exist than the bounded snapshot can display. */
  isTruncated: boolean;
  nodes: AgentOperationNode[];
}

export type AgentOperationsSource = "bff" | "fixture";

export interface AgentOperationsResult {
  snapshot: AgentOperationsSnapshot;
  source: AgentOperationsSource;
}

/** Deterministic development fixture: one root plus several worker states. */
export function fixtureAgentOperationsSnapshot(): AgentOperationsSnapshot {
  const capturedAt = "2026-08-11T16:00:00.000Z";
  const node = (
    id: string,
    parentId: string | null,
    role: AgentOperationRole,
    label: string,
    status: AgentOperationStatus,
    activity: string,
    model: string | null,
    startedAt: string | null,
  ): AgentOperationNode => ({
    id,
    parentId,
    role,
    label,
    status,
    activity,
    model,
    startedAt,
    updatedAt: capturedAt,
  });

  return {
    schemaVersion: AGENT_OPERATIONS_SCHEMA_VERSION,
    capturedAt,
    isPartial: false,
    isTruncated: false,
    nodes: [
      node(
        "op-root",
        null,
        "root",
        "Release triage run",
        "running",
        "Coordinating 5 workers",
        "lumi-core",
        "2026-08-11T15:31:04.000Z",
      ),
      node(
        "op-plan",
        "op-root",
        "worker",
        "Plan survey",
        "idle",
        "Thread idle",
        "lumi-core",
        "2026-08-11T15:31:20.000Z",
      ),
      node(
        "op-repo-a",
        "op-root",
        "worker",
        "Repo alpha patch",
        "running",
        "Applying review feedback",
        "lumi-core",
        "2026-08-11T15:33:41.000Z",
      ),
      node(
        "op-repo-b",
        "op-root",
        "worker",
        "Repo beta patch",
        "waiting",
        "Awaiting maintainer approval",
        "lumi-core",
        "2026-08-11T15:34:02.000Z",
      ),
      node(
        "op-docs",
        "op-root",
        "worker",
        "Docs refresh",
        "failed",
        "Retry available after lint error",
        "lumi-mini",
        "2026-08-11T15:35:12.000Z",
      ),
      node(
        "op-metrics",
        "op-root",
        "worker",
        "Metrics digest",
        "idle",
        "Thread idle",
        null,
        null,
      ),
      node(
        "op-archive",
        "op-root",
        "worker",
        "Archive old runs",
        "idle",
        "Thread idle",
        "lumi-mini",
        "2026-08-11T15:36:55.000Z",
      ),
      node(
        "op-repo-a-tests",
        "op-repo-a",
        "worker",
        "Alpha test sweep",
        "running",
        "Running narrowed test suite",
        "lumi-mini",
        "2026-08-11T15:40:10.000Z",
      ),
    ],
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseTimestamp(value: unknown): string | null {
  return typeof value === "string" && !Number.isNaN(Date.parse(value))
    ? value
    : null;
}

function parseAgentOperationNode(value: unknown): AgentOperationNode | null {
  if (!isRecord(value)) return null;
  if (
    typeof value.id === "string" &&
    value.id.length > 0 &&
    (value.parentId === null || typeof value.parentId === "string") &&
    (value.role === "root" || value.role === "worker") &&
    typeof value.label === "string" &&
    value.label.length > 0 &&
    (AGENT_OPERATION_STATUSES as readonly string[]).includes(
      value.status as string,
    ) &&
    typeof value.activity === "string" &&
    (value.model === null || typeof value.model === "string") &&
    (value.startedAt === null || parseTimestamp(value.startedAt) !== null) &&
    parseTimestamp(value.updatedAt) !== null
  ) {
    return {
      id: value.id,
      parentId: value.parentId,
      role: value.role,
      label: value.label,
      status: value.status as AgentOperationStatus,
      activity: value.activity,
      model: value.model,
      startedAt: value.startedAt as string | null,
      updatedAt: value.updatedAt as string,
    };
  }
  return null;
}

/** Parse into a closed DTO, dropping every field outside the Web contract. */
export function parseAgentOperationsSnapshot(
  value: unknown,
): AgentOperationsSnapshot | null {
  if (
    !isRecord(value) ||
    value.schemaVersion !== AGENT_OPERATIONS_SCHEMA_VERSION ||
    parseTimestamp(value.capturedAt) === null ||
    typeof value.isPartial !== "boolean" ||
    typeof value.isTruncated !== "boolean" ||
    !Array.isArray(value.nodes)
  ) {
    return null;
  }

  const nodes: AgentOperationNode[] = [];
  const byId = new Map<string, AgentOperationNode>();
  for (const rawNode of value.nodes) {
    const node = parseAgentOperationNode(rawNode);
    if (node === null || byId.has(node.id)) return null;
    nodes.push(node);
    byId.set(node.id, node);
  }

  for (const node of nodes) {
    if (
      node.parentId !== null &&
      (node.parentId === node.id || !byId.has(node.parentId))
    ) {
      return null;
    }

    const ancestors = new Set<string>([node.id]);
    let parentId = node.parentId;
    while (parentId !== null) {
      if (ancestors.has(parentId)) return null;
      ancestors.add(parentId);
      parentId = byId.get(parentId)?.parentId ?? null;
    }
  }

  return {
    schemaVersion: AGENT_OPERATIONS_SCHEMA_VERSION,
    capturedAt: value.capturedAt as string,
    isPartial: value.isPartial,
    isTruncated: value.isTruncated,
    nodes,
  };
}

export function isAgentOperationsSnapshot(
  value: unknown,
): value is AgentOperationsSnapshot {
  return parseAgentOperationsSnapshot(value) !== null;
}

/**
 * Read-only data adapter. Fetches the snapshot from the BFF when one is
 * reachable and otherwise falls back to the deterministic fixture, so the
 * shell is always usable in development and tests.
 */
export async function fetchAgentOperationsSnapshot(
  signal?: AbortSignal,
): Promise<AgentOperationsResult> {
  const useDevelopmentFixture =
    import.meta.env.DEV && import.meta.env.VITE_LUMI_WEB_BFF !== "true";
  if (useDevelopmentFixture) {
    return { snapshot: fixtureAgentOperationsSnapshot(), source: "fixture" };
  }

  try {
    const response = await fetch(AGENT_OPERATIONS_ENDPOINT, {
      headers: { accept: "application/json" },
      signal,
    });
    if (!response.ok) {
      throw new Error(`BFF responded with ${response.status}`);
    }
    const body: unknown = await response.json();
    const snapshot = parseAgentOperationsSnapshot(body);
    if (snapshot === null) {
      throw new Error("BFF payload did not match AgentOperationsSnapshot v1");
    }
    return { snapshot, source: "bff" };
  } catch (error) {
    if (signal?.aborted) throw error;
    if (import.meta.env.DEV) {
      return { snapshot: fixtureAgentOperationsSnapshot(), source: "fixture" };
    }
    throw error;
  }
}
