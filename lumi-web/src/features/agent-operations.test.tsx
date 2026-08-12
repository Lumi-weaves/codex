import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  deriveRuntimeTraceStats,
  fixtureRuntimeTrace,
  loadRuntimeTrace,
  parseRuntimeTrace,
} from "../api/runtime-trace";
import type { RuntimeTrace, RuntimeTraceResult } from "../api/runtime-trace";
import { AgentOperationsPage } from "./AgentOperationsPage";

function fixtureResult(trace = fixtureRuntimeTrace()): RuntimeTraceResult {
  return { trace, source: "fixture" };
}

describe("runtime trace contract", () => {
  it("loads the sanitized captured hw1 trace through the same closed parser", async () => {
    const result = await loadRuntimeTrace();
    expect(result.source).toBe("captured");
    expect(result.trace.traceId).toBe("hw1-captured-coordination");
    expect(result.trace.agents).toHaveLength(3);
    expect(result.trace.generations).toHaveLength(77);
    expect(result.trace.operations).toHaveLength(73);
  });

  it("closes the DTO and preserves a valid causal trace", () => {
    const fixture = fixtureRuntimeTrace();
    const withPrivateContent = {
      ...fixture,
      prompt: "must not cross the browser contract",
      generations: fixture.generations.map((generation) => ({
        ...generation,
        output: "also private",
      })),
    };

    expect(parseRuntimeTrace(withPrivateContent)).toEqual(fixture);
  });

  it("rejects duplicate identity, invalid ordering, and broken joins", () => {
    const fixture = fixtureRuntimeTrace();
    expect(
      parseRuntimeTrace({
        ...fixture,
        events: [...fixture.events, fixture.events[0]],
      }),
    ).toBeNull();

    expect(
      parseRuntimeTrace({
        ...fixture,
        generations: fixture.generations.map((generation, index) =>
          index === 0
            ? { ...generation, firstTokenAt: "2026-08-12T09:40:00Z" }
            : generation,
        ),
      }),
    ).toBeNull();

    expect(
      parseRuntimeTrace({
        ...fixture,
        events: fixture.events.map((event) =>
          event.id === "event-terminal-complete"
            ? { ...event, consumedByGenerationId: "missing-generation" }
            : event,
        ),
      }),
    ).toBeNull();

    expect(
      parseRuntimeTrace({
        ...fixture,
        operations: fixture.operations.map((operation) => ({
          ...operation,
          completionEventId: "event-user-input",
        })),
      }),
    ).toBeNull();

    expect(
      parseRuntimeTrace({
        ...fixture,
        events: fixture.events.map((event) =>
          event.id === "event-terminal-complete"
            ? { ...event, sourceOperationId: null }
            : event,
        ),
      }),
    ).toBeNull();

    expect(
      parseRuntimeTrace({
        ...fixture,
        events: fixture.events.map((event) =>
          event.id === "event-vera-return"
            ? {
                ...event,
                occurredAt: "2026-08-12T09:41:11.000Z",
                enqueuedAt: "2026-08-12T09:41:11.000Z",
              }
            : event,
        ),
      }),
    ).toBeNull();

    expect(
      parseRuntimeTrace({
        ...fixture,
        generations: fixture.generations.map((generation) =>
          generation.id === "main-g4"
            ? {
                ...generation,
                completedAt: "2026-08-12T09:41:29.000Z",
              }
            : generation,
        ),
      }),
    ).toBeNull();
  });

  it("derives deterministic workflow statistics from timestamps", () => {
    expect(deriveRuntimeTraceStats(fixtureRuntimeTrace())).toMatchObject({
      wallTimeMs: 28_000,
      mainGenerations: 4,
      totalGenerations: 8,
      asyncBranches: 3,
      agentMessages: 5,
      maxConcurrency: 3,
      asyncOverlapRatio: 0.775,
      medianQueueDelayMs: 1_950,
      maxQueueDelayMs: 5_450,
    });
  });
});

describe("Agent Operations Perfetto bridge", () => {
  it("describes and opens the native runtime trace", async () => {
    const opener = vi.fn(async () => undefined);
    render(
      <AgentOperationsPage
        loader={async () => fixtureResult()}
        perfettoOpener={opener}
      />,
    );

    expect(
      await screen.findByRole("heading", {
        name: "Open this workflow in Perfetto",
      }),
    ).not.toBeNull();
    expect(screen.getByText("28s")).not.toBeNull();
    expect(screen.getByText("78%")).not.toBeNull();
    expect(screen.getByText("Fixture trace")).not.toBeNull();
    expect(screen.getByText("Perfetto TrackEvent proto")).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Open in Perfetto" }));
    expect(opener).toHaveBeenCalledOnce();
    expect(
      await screen.findByText("Trace delivered to Perfetto in a new tab."),
    ).not.toBeNull();
  });

  it("shows an empty trace and recovers after a load error", async () => {
    const empty: RuntimeTrace = {
      ...fixtureRuntimeTrace(),
      generations: [],
      operations: [],
      events: [],
    };
    const { unmount } = render(
      <AgentOperationsPage loader={async () => fixtureResult(empty)} />,
    );
    await screen.findByText("This trace contains no generations.");
    unmount();

    let calls = 0;
    render(
      <AgentOperationsPage
        loader={async () => {
          calls += 1;
          if (calls === 1) throw new Error("trace unavailable");
          return fixtureResult();
        }}
      />,
    );
    await screen.findByText(/Could not load runtime trace: trace unavailable/);
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByRole("heading", {
      name: "Open this workflow in Perfetto",
    });
    expect(calls).toBe(2);
  });
});
