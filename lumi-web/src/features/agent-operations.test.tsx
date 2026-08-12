import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  fixtureAgentOperationsSnapshot,
  isAgentOperationsSnapshot,
  parseAgentOperationsSnapshot,
} from "../api/agent-operations";
import type { AgentOperationsResult } from "../api/agent-operations";
import { AgentOperationsPage } from "./AgentOperationsPage";

function fixtureResult(
  snapshot = fixtureAgentOperationsSnapshot(),
): AgentOperationsResult {
  return { snapshot, source: "fixture" };
}

describe("agent operations fixture contract", () => {
  it("produces a valid v1 snapshot with a root and several worker states", () => {
    const snapshot = fixtureAgentOperationsSnapshot();
    expect(isAgentOperationsSnapshot(snapshot)).toBe(true);
    expect(snapshot.nodes.some((node) => node.role === "root")).toBe(true);
    const statuses = new Set(snapshot.nodes.map((node) => node.status));
    for (const status of [
      "queued",
      "running",
      "waiting",
      "succeeded",
      "failed",
      "cancelled",
    ]) {
      expect(statuses.has(status as never)).toBe(true);
    }
  });

  it("closes the DTO and rejects invalid graph identity, time, and topology", () => {
    const fixture = fixtureAgentOperationsSnapshot();
    const withPrivateExtra = {
      ...fixture,
      prompt: "must not cross the browser contract",
      nodes: fixture.nodes.map((node) => ({ ...node, cwd: "/private/path" })),
    };
    expect(parseAgentOperationsSnapshot(withPrivateExtra)).toEqual(fixture);

    const duplicate = {
      ...fixture,
      nodes: [...fixture.nodes, fixture.nodes[0]],
    };
    expect(parseAgentOperationsSnapshot(duplicate)).toBeNull();

    const badTime = {
      ...fixture,
      nodes: fixture.nodes.map((node, index) =>
        index === 0 ? { ...node, updatedAt: "not-a-time" } : node,
      ),
    };
    expect(parseAgentOperationsSnapshot(badTime)).toBeNull();

    const cycle = {
      ...fixture,
      nodes: fixture.nodes.map((node) => {
        if (node.id === "op-root") return { ...node, parentId: "op-plan" };
        return node;
      }),
    };
    expect(parseAgentOperationsSnapshot(cycle)).toBeNull();
  });
});

describe("Agent Operations page", () => {
  it("renders every fixture operation and labels fixture data honestly", async () => {
    render(
      <AgentOperationsPage
        loader={async () => fixtureResult()}
        fitView={false}
      />,
    );

    for (const node of fixtureAgentOperationsSnapshot().nodes) {
      expect(await screen.findByText(node.label)).not.toBeNull();
    }
    expect(screen.getByText("Fixture data")).not.toBeNull();
    expect(screen.getByText("Snapshot summary")).not.toBeNull();
  });

  it("selects an operation and shows its details", async () => {
    render(
      <AgentOperationsPage
        loader={async () => fixtureResult()}
        fitView={false}
      />,
    );

    expect(await screen.findByText("Snapshot summary")).not.toBeNull();
    const workerLabel = await screen.findByText("Repo beta patch");
    const worker = workerLabel.closest("button");
    expect(worker).not.toBeNull();
    if (worker === null) throw new Error("operation label is not in a button");
    fireEvent.click(worker);

    expect(worker.getAttribute("aria-pressed")).toBe("true");
    expect(screen.queryByText("Snapshot summary")).toBeNull();
    expect(
      await screen.findByText("Awaiting maintainer approval"),
    ).not.toBeNull();
    expect(
      screen.getByRole("heading", { name: "Repo beta patch" }),
    ).not.toBeNull();
  });

  it("shows an empty state when the snapshot has no operations", async () => {
    const empty = { ...fixtureAgentOperationsSnapshot(), nodes: [] };
    render(
      <AgentOperationsPage
        loader={async () => fixtureResult(empty)}
        fitView={false}
      />,
    );

    await screen.findByText("No agent operations are running right now.");
    expect(screen.queryByLabelText("Agent operations graph")).toBeNull();
  });

  it("shows an error state and recovers on retry", async () => {
    let calls = 0;
    const loader = async () => {
      calls += 1;
      if (calls === 1) throw new Error("bff unreachable");
      return fixtureResult();
    };

    render(<AgentOperationsPage loader={loader} fitView={false} />);

    await screen.findByText(/Could not load agent operations: bff unreachable/);
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByText("Snapshot summary");
    expect(calls).toBe(2);
  });
});
