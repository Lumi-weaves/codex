import dagre from "@dagrejs/dagre";
import { Handle, Position } from "@xyflow/react";
import type { Edge, Node, NodeProps, NodeTypes } from "@xyflow/react";

import type {
  AgentOperationNode,
  AgentOperationRole,
  AgentOperationStatus,
} from "../api/agent-operations";

export const OPERATION_NODE_WIDTH = 232;
export const OPERATION_NODE_HEIGHT = 68;

export interface OperationNodeData extends Record<string, unknown> {
  label: string;
  status: AgentOperationStatus;
  role: AgentOperationRole;
  activity: string;
  model: string | null;
  onSelect: (id: string) => void;
}

export type OperationFlowNode = Node<OperationNodeData, "operation">;

/** Lay out the operation tree top-down with dagre and map it to React Flow. */
export function layoutOperationNodes(
  operations: AgentOperationNode[],
  selectedId: string | null,
  onSelect: (id: string) => void,
): { nodes: OperationFlowNode[]; edges: Edge[] } {
  const graph = new dagre.graphlib.Graph();
  graph.setDefaultEdgeLabel(() => ({}));
  graph.setGraph({
    rankdir: "LR",
    nodesep: 24,
    ranksep: 72,
    marginx: 24,
    marginy: 24,
  });

  const ids = new Set(operations.map((operation) => operation.id));
  for (const operation of operations) {
    graph.setNode(operation.id, {
      width: OPERATION_NODE_WIDTH,
      height: OPERATION_NODE_HEIGHT,
    });
  }
  for (const operation of operations) {
    if (operation.parentId !== null && ids.has(operation.parentId)) {
      graph.setEdge(operation.parentId, operation.id);
    }
  }
  dagre.layout(graph);

  const nodes: OperationFlowNode[] = operations.map((operation) => {
    const point = graph.node(operation.id) as { x: number; y: number };
    return {
      id: operation.id,
      type: "operation",
      selected: operation.id === selectedId,
      position: {
        x: point.x - OPERATION_NODE_WIDTH / 2,
        y: point.y - OPERATION_NODE_HEIGHT / 2,
      },
      data: {
        label: operation.label,
        status: operation.status,
        role: operation.role,
        activity: operation.activity,
        model: operation.model,
        onSelect,
      },
    };
  });

  const edges: Edge[] = operations
    .filter(
      (operation) => operation.parentId !== null && ids.has(operation.parentId),
    )
    .map((operation) => ({
      id: `${operation.parentId}->${operation.id}`,
      source: operation.parentId as string,
      target: operation.id,
      animated: operation.status === "running",
    }));

  return { nodes, edges };
}

export function OperationNode({
  id,
  data,
  selected,
}: NodeProps<OperationFlowNode>) {
  return (
    <div
      className={
        selected ? "operation-node operation-node--selected" : "operation-node"
      }
    >
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <button
        type="button"
        className="operation-node__button"
        aria-pressed={selected}
        onClick={() => data.onSelect(id)}
      >
        <span className="operation-node__heading">
          <span
            className={`operation-node__dot operation-node__dot--${data.status}`}
            aria-hidden="true"
          />
          <span className="operation-node__label">{data.label}</span>
        </span>
        <span className="operation-node__meta">
          <span className={`status-badge status-badge--${data.status}`}>
            {data.status}
          </span>
          {data.role === "root" ? (
            <span className="operation-node__role">root</span>
          ) : null}
          {data.model !== null ? (
            <span className="operation-node__model">{data.model}</span>
          ) : null}
        </span>
      </button>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </div>
  );
}

export const operationNodeTypes = {
  operation: OperationNode,
} satisfies NodeTypes;
