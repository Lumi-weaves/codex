import { afterEach, describe, expect, test } from "bun:test";

import { ResponsesWebSocketPool } from "../src/responses-websocket";

const pools: ResponsesWebSocketPool[] = [];
const servers: Bun.Server<undefined>[] = [];

afterEach(async () => {
  for (const pool of pools.splice(0)) pool.closeAll();
  for (const server of servers.splice(0)) await server.stop(true);
});

function responseHeaders(): Headers {
  return new Headers({
    "content-type": "text/event-stream",
    "x-richcodex-account-id": "account-1",
  });
}

function requestBody(input: readonly unknown[]): Record<string, unknown> {
  return {
    model: "gpt-5.6-sol",
    input,
    instructions: "stable",
    tools: [{ type: "function", name: "query" }],
    reasoning: { effort: "high", context: "all_turns" },
    stream: true,
    store: false,
    prompt_cache_key: "vibeseed-lineage-1",
  };
}

describe("Responses WebSocket continuation", () => {
  test("reuses an exact response prefix and sends only the new delta", async () => {
    const payloads: Record<string, unknown>[] = [];
    const handshakes: Headers[] = [];
    const firstOutput = [
      {
        type: "reasoning",
        id: "reasoning-1",
        encrypted_content: "opaque-1",
        summary: [],
      },
      {
        type: "function_call",
        id: "call-item-1",
        call_id: "call-1",
        name: "query",
        arguments: "{}",
      },
    ];
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch(request, server) {
        handshakes.push(new Headers(request.headers));
        return server.upgrade(request)
          ? undefined
          : new Response("upgrade failed", { status: 400 });
      },
      websocket: {
        message(socket, message) {
          const payload = JSON.parse(String(message)) as Record<string, unknown>;
          payloads.push(payload);
          const responseId = `response-${payloads.length}`;
          const output = payloads.length === 1
            ? firstOutput
            : [{ type: "message", id: "message-2", role: "assistant", content: [] }];
          socket.send(JSON.stringify({
            type: "response.created",
            response: { id: responseId },
          }));
          socket.send(JSON.stringify({
            type: "response.completed",
            response: { id: responseId, output },
          }));
        },
      },
    });
    servers.push(server);
    const pool = new ResponsesWebSocketPool();
    pools.push(pool);
    const url = `ws://127.0.0.1:${server.port}/responses`;

    const firstInput = [{ role: "user", content: "inspect" }];
    const first = await pool.stream({
      continuationKey: "vibeseed-lineage-1",
      routeKey: "target-1",
      url,
      headers: new Headers({ authorization: "Bearer private" }),
      body: requestBody(firstInput),
      responseHeaders: responseHeaders(),
    });
    expect(await first.text()).toContain("response.completed");

    const settlement = {
      type: "function_call_output",
      call_id: "call-1",
      output: "done",
    };
    const second = await pool.stream({
      continuationKey: "vibeseed-lineage-1",
      routeKey: "target-1",
      url,
      headers: new Headers({ authorization: "Bearer private" }),
      body: requestBody([...firstInput, ...firstOutput, settlement]),
      responseHeaders: responseHeaders(),
    });
    expect(await second.text()).toContain("response.completed");

    expect(handshakes).toHaveLength(1);
    expect(handshakes[0]!.get("openai-beta")).toBe("responses_websockets=2026-02-06");
    expect(payloads).toHaveLength(2);
    expect(payloads[0]).toMatchObject({
      type: "response.create",
      input: firstInput,
    });
    expect(payloads[0]!.previous_response_id).toBeUndefined();
    expect(payloads[1]).toMatchObject({
      type: "response.create",
      previous_response_id: "response-1",
      input: [settlement],
    });
  });

  test("falls back to a full create when the logical prefix changes", async () => {
    const payloads: Record<string, unknown>[] = [];
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch(request, server) {
        return server.upgrade(request)
          ? undefined
          : new Response("upgrade failed", { status: 400 });
      },
      websocket: {
        message(socket, message) {
          const payload = JSON.parse(String(message)) as Record<string, unknown>;
          payloads.push(payload);
          const responseId = `response-${payloads.length}`;
          socket.send(JSON.stringify({
            type: "response.completed",
            response: { id: responseId, output: [] },
          }));
        },
      },
    });
    servers.push(server);
    const pool = new ResponsesWebSocketPool();
    pools.push(pool);
    const stream = (body: Record<string, unknown>): Promise<Response> => pool.stream({
      continuationKey: "vibeseed-lineage-1",
      routeKey: "target-1",
      url: `ws://127.0.0.1:${server.port}/responses`,
      headers: new Headers(),
      body,
      responseHeaders: responseHeaders(),
    });

    await (await stream(requestBody([{ role: "user", content: "first" }]))).text();
    await (await stream(requestBody([{ role: "user", content: "different" }]))).text();

    expect(payloads[1]!.previous_response_id).toBeUndefined();
    expect(payloads[1]!.input).toEqual([{ role: "user", content: "different" }]);
  });

  test("retries one stale previous-response handle with the full request", async () => {
    const payloads: Record<string, unknown>[] = [];
    const firstOutput = [{ type: "reasoning", id: "reasoning-1", encrypted_content: "opaque" }];
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch(request, server) {
        return server.upgrade(request)
          ? undefined
          : new Response("upgrade failed", { status: 400 });
      },
      websocket: {
        message(socket, message) {
          const payload = JSON.parse(String(message)) as Record<string, unknown>;
          payloads.push(payload);
          if (payloads.length === 1) {
            socket.send(JSON.stringify({
              type: "response.completed",
              response: { id: "response-1", output: firstOutput },
            }));
          } else if (payloads.length === 2) {
            socket.send(JSON.stringify({
              type: "error",
              error: { code: "previous_response_not_found" },
            }));
          } else {
            socket.send(JSON.stringify({
              type: "response.completed",
              response: { id: "response-2", output: [] },
            }));
          }
        },
      },
    });
    servers.push(server);
    const pool = new ResponsesWebSocketPool();
    pools.push(pool);
    const url = `ws://127.0.0.1:${server.port}/responses`;
    const firstInput = [{ role: "user", content: "first" }];
    const invoke = (body: Record<string, unknown>): Promise<Response> => pool.stream({
      continuationKey: "vibeseed-lineage-1",
      routeKey: "target-1",
      url,
      headers: new Headers(),
      body,
      responseHeaders: responseHeaders(),
    });

    await (await invoke(requestBody(firstInput))).text();
    const secondInput = [...firstInput, ...firstOutput, { role: "user", content: "continue" }];
    const response = await invoke(requestBody(secondInput));
    const text = await response.text();

    expect(text).not.toContain("previous_response_not_found");
    expect(payloads[1]!.previous_response_id).toBe("response-1");
    expect(payloads[2]!.previous_response_id).toBeUndefined();
    expect(payloads[2]!.input).toEqual(secondInput);
  });
});
