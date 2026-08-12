import { afterEach, describe, expect, it, vi } from "vitest";

import { openTraceInPerfetto } from "./perfetto";

describe("Perfetto postMessage bridge", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("waits for the official handshake before transferring trace bytes", async () => {
    const perfettoWindow = { postMessage: vi.fn() } as unknown as Window;
    vi.spyOn(window, "open").mockReturnValue(perfettoWindow);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(new Uint8Array([10, 20, 30]))),
    );

    const opened = openTraceInPerfetto({
      traceUrl: "/trace.pftrace",
      title: "Runtime trace",
      fileName: "runtime.pftrace",
    });
    await vi.waitFor(() => {
      expect(perfettoWindow.postMessage).toHaveBeenCalledWith(
        "PING",
        "https://ui.perfetto.dev",
      );
    });

    window.dispatchEvent(
      new MessageEvent("message", {
        data: "PONG",
        origin: "https://ui.perfetto.dev",
        source: perfettoWindow,
      }),
    );
    await opened;

    const delivery = vi
      .mocked(perfettoWindow.postMessage)
      .mock.calls.find(([message]) => typeof message === "object");
    expect(delivery?.[0]).toMatchObject({
      perfetto: {
        title: "Runtime trace",
        fileName: "runtime.pftrace",
        localOnly: true,
      },
    });
    expect(
      (delivery?.[0] as { perfetto: { buffer: ArrayBuffer } }).perfetto.buffer,
    ).toBeInstanceOf(ArrayBuffer);
  });
});
