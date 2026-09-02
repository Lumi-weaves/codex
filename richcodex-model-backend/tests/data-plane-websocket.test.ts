import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createModelDataPlane } from "../src/data-plane";
import { createModelPlaneStore } from "../src/model-plane";

const CAPABILITY = "richcodex-websocket-capability-0123456789abcdef";
const servers: Bun.Server<undefined>[] = [];

afterEach(async () => {
  for (const server of servers.splice(0)) await server.stop(true);
});

function configuredStore() {
  const root = mkdtempSync(join(tmpdir(), "richcodex-data-plane-ws-"));
  const store = createModelPlaneStore(join(root, "state"), {
    createAccountId: () => "account-1",
    createTargetId: () => "target-1",
  });
  const account = store.addApiKeyAccount({
    providerId: "openai",
    providerDisplayName: "OpenAI",
    apiBaseUrl: "https://api.openai.com/v1",
    apiKey: "private-api-key",
    userLabel: "OpenAI",
  });
  store.createModelRoute({
    expectedRevision: 1,
    modelTag: "lumi",
    displayName: "Lumi",
    semanticModel: "gpt-5.6-sol",
    providerId: "openai",
    accountId: account.id,
    upstreamModelId: "gpt-5.6-sol",
  });
  return store;
}

function request(): Request {
  return new Request("http://127.0.0.1/v1/responses", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-richcodex-data-plane-token": CAPABILITY,
    },
    body: JSON.stringify({
      model: "lumi",
      input: [{ role: "user", content: "hello" }],
      stream: true,
      store: false,
      prompt_cache_key: "vibeseed-session-thread-generation-model",
    }),
  });
}

describe("model data plane Responses WebSocket", () => {
  test("keeps credentials in the Enclave while returning the ordinary SSE surface", async () => {
    const payloads: Record<string, unknown>[] = [];
    const upstreamHeaders: Headers[] = [];
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch(request, server) {
        upstreamHeaders.push(new Headers(request.headers));
        return server.upgrade(request)
          ? undefined
          : new Response("upgrade failed", { status: 400 });
      },
      websocket: {
        message(socket, message) {
          payloads.push(JSON.parse(String(message)) as Record<string, unknown>);
          socket.send(JSON.stringify({
            type: "response.completed",
            response: { id: "response-1", output: [] },
          }));
        },
      },
    });
    servers.push(server);
    const requestedUrls: string[] = [];
    const requestedProxies: Array<string | undefined> = [];
    let fetched = false;
    const plane = createModelDataPlane({
      capability: CAPABILITY,
      modelPlaneStore: configuredStore(),
      fetch: async () => {
        fetched = true;
        return new Response(null, { status: 500 });
      },
      responsesWebSocketProxy: "http://127.0.0.1:7890",
      responsesWebSocketFactory: (url, options) => {
        requestedUrls.push(url);
        requestedProxies.push(options.proxy);
        return Reflect.construct(WebSocket, [
          `ws://127.0.0.1:${server.port}/responses`,
          { headers: Object.fromEntries(options.headers) },
        ]) as WebSocket;
      },
    });

    const response = await plane.handle(request());

    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("text/event-stream");
    expect(response.headers.get("x-richcodex-account-id")).toBe("account-1");
    expect(await response.text()).toContain("response.completed");
    expect(fetched).toBe(false);
    expect(requestedUrls).toEqual(["wss://api.openai.com/v1/responses"]);
    expect(requestedProxies).toEqual(["http://127.0.0.1:7890"]);
    expect(upstreamHeaders[0]!.get("authorization")).toBe("Bearer private-api-key");
    expect(upstreamHeaders[0]!.get("openai-beta")).toBe("responses_websockets=2026-02-06");
    expect(payloads[0]).toMatchObject({
      type: "response.create",
      model: "gpt-5.6-sol",
      input: [{ role: "user", content: "hello" }],
    });
  });

  test("uses full HTTP replay when the ephemeral WebSocket cannot open", async () => {
    const fetchBodies: Record<string, unknown>[] = [];
    const fetchProxies: Array<string | undefined> = [];
    const resolvedUrls: string[] = [];
    const plane = createModelDataPlane({
      capability: CAPABILITY,
      modelPlaneStore: configuredStore(),
      fetch: async (_input, init) => {
        fetchBodies.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
        fetchProxies.push(init?.proxy);
        return new Response('data: {"type":"response.completed"}\n\n', {
          status: 200,
          headers: { "content-type": "text/event-stream" },
        });
      },
      networkRouteResolver: {
        resolve: async url => {
          resolvedUrls.push(url);
          return { kind: "proxy", url: "http://127.0.0.1:7897" };
        },
      },
      responsesWebSocketFactory: () => {
        throw new Error("offline");
      },
    });

    const response = await plane.handle(request());

    expect(response.status).toBe(200);
    expect(await response.text()).toContain("response.completed");
    expect(fetchBodies).toHaveLength(1);
    expect(fetchBodies[0]).toMatchObject({
      model: "gpt-5.6-sol",
      input: [{ role: "user", content: "hello" }],
    });
    expect(fetchProxies).toEqual(["http://127.0.0.1:7897"]);
    expect(resolvedUrls).toEqual([
      "wss://api.openai.com/v1/responses",
      "https://api.openai.com/v1/responses",
    ]);
  });
});
