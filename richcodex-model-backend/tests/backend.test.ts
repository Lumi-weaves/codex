import { describe, expect, test } from "bun:test";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

import {
  createHeadlessBackend,
  parseHeadlessBackendArgs,
  resolveBackendStateRoot,
  RICHCODEX_BACKEND_MAX_MESSAGE_BYTES,
  RICHCODEX_BACKEND_STATE_ROOT_ENV,
} from "../src/backend";
import { RICHCODEX_BACKEND_KERNEL } from "../src/kernel-manifest";
import { createModelDataPlane } from "../src/data-plane";
import { createDeviceOAuthCoordinator } from "../src/device-oauth";
import type { DeviceOAuthCoordinator, SafeProviderLogin } from "../src/device-oauth";
import { createBrowserOAuthCoordinator } from "../src/browser-oauth";
import { createModelPlaneStore } from "../src/model-plane";
import type {
  ModelExecutionCandidate,
  ModelPlaneStore,
  ProviderAccountStatus,
  StoredProviderCredential,
  StoredOAuthCredential,
} from "../src/model-plane";

const TEST_DATA_PLANE_CAPABILITY = "richcodex-test-capability-0123456789abcdef";
const TEST_CLIENT_ATTEMPT_ID = "018f0000-0000-4000-8000-000000000001";

function executionReceipt(response: Response): Record<string, unknown> {
  const encoded = response.headers.get("x-richcodex-execution-receipt");
  if (!encoded) throw new Error("missing RichCodex execution receipt");
  return JSON.parse(Buffer.from(encoded, "base64url").toString("utf8"));
}

function inputOf(text: string): AsyncIterable<Uint8Array> {
  return (async function* (): AsyncGenerator<Uint8Array> {
    yield new TextEncoder().encode(text);
  })();
}

async function runBackend(
  input: string,
  stateRoot: string,
  deviceOAuthCoordinator?: DeviceOAuthCoordinator,
  browserOAuthCoordinator?: DeviceOAuthCoordinator,
): Promise<{ lines: unknown[]; result: unknown; stderr: string[] }> {
  const lines: unknown[] = [];
  const stderr: string[] = [];
  const backend = createHeadlessBackend({
    stateRoot,
    env: {},
    dataPlaneCapability: TEST_DATA_PLANE_CAPABILITY,
    deviceOAuthCoordinator,
    browserOAuthCoordinator,
  });
  const result = await backend.run({
    stdin: inputOf(input),
    stdout: line => { lines.push(JSON.parse(line)); },
    stderr: line => { stderr.push(line); },
  });
  return { lines, result, stderr };
}

function jwt(payload: Record<string, unknown>): string {
  return `header.${Buffer.from(JSON.stringify(payload)).toString("base64url")}.signature`;
}

function codexAuthJson(accountId: string, expiresAtMs: number): string {
  return JSON.stringify({
    tokens: {
      access_token: jwt({ exp: Math.floor(expiresAtMs / 1000), chatgpt_account_id: accountId }),
      refresh_token: `refresh-${accountId}`,
      account_id: accountId,
    },
  });
}

function codexAuthJsonWithDistinctAccountMetadata(expiresAtMs: number): string {
  return JSON.stringify({
    tokens: {
      id_token: jwt({
        exp: Math.floor(expiresAtMs / 1000),
        "https://api.openai.com/auth": { chatgpt_account_id: "id-token-workspace" },
      }),
      access_token: jwt({
        exp: Math.floor(expiresAtMs / 1000),
        chatgpt_account_id: "access-token-workspace",
      }),
      refresh_token: "refresh-distinct-workspaces",
      account_id: "selected-workspace",
    },
  });
}

function executionCandidate(
  accountId: string,
  targetId: string,
  priority: number,
  credential: StoredProviderCredential,
  providerId = "openai",
  apiBaseUrl = "https://api.openai.com/v1",
): ModelExecutionCandidate {
  return {
    modelTag: "my-fast-model",
    semanticModel: "gpt-5.6-luna",
    targetId,
    providerId,
    accountId,
    upstreamModelId: "gpt-5.6-luna",
    apiBaseUrl,
    priority,
    credential,
  };
}

function executionStore(
  candidates: readonly ModelExecutionCandidate[],
  onStatus: (accountId: string, status: ProviderAccountStatus) => void = () => undefined,
  onReplace: (
    accountId: string,
    expectedRefreshToken: string,
    credential: StoredOAuthCredential,
  ) => boolean = () => true,
): ModelPlaneStore {
  return {
    snapshot: () => ({
      desiredStateRevision: 0,
      catalogRevision: 0,
      providers: [],
      accounts: [],
      modelRoutes: [],
    }),
    importCodexAuthJson: () => { throw new Error("not used"); },
    addOAuthAccount: () => { throw new Error("not used"); },
    installClientAuthTokens: () => { throw new Error("not used"); },
    readOAuthAccessToken: () => { throw new Error("not used"); },
    addApiKeyAccount: () => { throw new Error("not used"); },
    previewAccountRemoval: () => { throw new Error("not used"); },
    removeAccount: () => { throw new Error("not used"); },
    renameAccount: () => { throw new Error("not used"); },
    replaceApiKeyCredential: () => { throw new Error("not used"); },
    reauthenticateOAuthAccount: () => { throw new Error("not used"); },
    createModelRoute: () => { throw new Error("not used"); },
    setModelRouteTargets: () => { throw new Error("not used"); },
    retireModelRoute: () => { throw new Error("not used"); },
    resolveExecutionCandidates: modelTag => modelTag === "my-fast-model" ? candidates : [],
    replaceOAuthCredential: onReplace,
    markAccountStatus: onStatus,
  };
}

describe("RichCodex browser OAuth coordinator", () => {
  test("validates callback state and persists only exchanged credentials", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-browser-oauth-"));
    const now = 1_900_000_000_000;
    const store = createModelPlaneStore(root, { now: () => now });
    const coordinator = createBrowserOAuthCoordinator({
      modelPlaneStore: store,
      env: {},
      now: () => now,
      callbackPort: 0,
      createLoginId: () => "browser-login-safe",
      fetch: async (input, init) => {
        expect(String(input)).toBe("https://auth.openai.com/oauth/token");
        const body = new URLSearchParams(String(init?.body));
        expect(body.get("code")).toBe("callback-code");
        expect(body.get("code_verifier")).toBeTruthy();
        return new Response(JSON.stringify({
          id_token: jwt({
            exp: Math.floor((now + 3_600_000) / 1000),
            chatgpt_account_id: "browser-workspace",
          }),
          access_token: jwt({ exp: Math.floor((now + 3_600_000) / 1000) }),
          refresh_token: "browser-refresh-token",
        }), { status: 200, headers: { "content-type": "application/json" } });
      },
    });

    const started = await coordinator.start("Browser Account");
    const authorization = new URL(started.verificationUrl!);
    expect(authorization.origin + authorization.pathname)
      .toBe("https://auth.openai.com/oauth/authorize");
    const state = authorization.searchParams.get("state");
    const redirect = new URL(authorization.searchParams.get("redirect_uri")!);
    const wrong = await fetch(
      `http://127.0.0.1:${redirect.port}${redirect.pathname}?code=wrong&state=wrong`,
    );
    expect(wrong.status).toBe(400);
    expect(await wrong.text()).toContain("Return to VibeSeed");
    expect(coordinator.status(started.loginId).status).toBe("awaitingUser");

    const accepted = await fetch(
      `http://127.0.0.1:${redirect.port}${redirect.pathname}?code=callback-code&state=${state}`,
    );
    expect(accepted.status).toBe(200);
    const acceptedPage = await accepted.text();
    expect(acceptedPage).toContain("<title>VibeSeed</title>");
    expect(acceptedPage).toContain("return to VibeSeed to finish signing in");
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (coordinator.status(started.loginId).status === "completed") break;
      await Bun.sleep(10);
    }
    const completed = coordinator.status(started.loginId);
    expect(completed.status).toBe("completed");
    expect(completed.account?.userLabel).toBe("Browser Account");
    expect(JSON.stringify(completed)).not.toContain("callback-code");
    expect(JSON.stringify(completed)).not.toContain("browser-refresh-token");
    coordinator.shutdown();
  });
});

describe("RichCodex private model data plane", () => {
  test("returns a client-owned token 401 without attempting provider refresh", async () => {
    const credential: StoredOAuthCredential = {
      kind: "oauth",
      accessToken: "client-owned-access-token",
      refreshToken: null,
      chatgptAccountId: "client-workspace",
      expiresAt: null,
    };
    const calls: string[] = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore([
        executionCandidate("client-account", "client-target", 0, credential),
      ]),
      fetch: (async (input, init) => {
        calls.push(String(input));
        expect(new Headers(init?.headers).get("authorization"))
          .toBe("Bearer client-owned-access-token");
        return new Response("unauthorized", { status: 401 });
      }) as typeof fetch,
    });
    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
        "content-type": "application/json",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));
    expect(response.status).toBe(401);
    expect(calls).toEqual(["https://chatgpt.com/backend-api/codex/responses"]);
  });

  test("requires its process capability and rewrites only the stable model tag", async () => {
    const now = 1_000_000;
    const credential: StoredOAuthCredential = {
      kind: "oauth",
      accessToken: "private-access-token",
      refreshToken: "private-refresh-token",
      chatgptAccountId: "workspace-account",
      expiresAt: now + 3_600_000,
    };
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const statuses: Array<[string, ProviderAccountStatus]> = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore(
        [executionCandidate("account-1", "target-1", 0, credential)],
        (accountId, status) => { statuses.push([accountId, status]); },
      ),
      now: () => now,
      fetch: (async (input, init) => {
        calls.push({ url: String(input), init });
        return new Response(
          'data: {"type":"response.created","response":{"id":"resp-1"}}\n\n'
            + 'data: {"type":"response.completed"}\n\n',
          {
          status: 200,
          headers: { "content-type": "text/event-stream" },
          },
        );
      }) as typeof fetch,
    });

    const unauthorized = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: { authorization: `Bearer ${TEST_DATA_PLANE_CAPABILITY}` },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));
    expect(unauthorized.status).toBe(401);
    expect(calls).toHaveLength(0);

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
        "content-type": "application/json",
        "x-codex-client-attempt-id": TEST_CLIENT_ATTEMPT_ID,
        "x-codex-turn-state": "turn-state",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [{ role: "user" }] }),
    }));
    expect(response.status).toBe(200);
    expect(response.headers.get("x-richcodex-model-tag")).toBe("my-fast-model");
    expect(response.headers.get("x-richcodex-account-id")).toBe("account-1");
    expect(response.headers.get("x-richcodex-client-attempt-id")).toBe(TEST_CLIENT_ATTEMPT_ID);
    expect(executionReceipt(response)).toEqual({
      modelTag: "my-fast-model",
      resolvedModel: "gpt-5.6-luna",
      providerId: "openai",
      accountId: "account-1",
      targetId: "target-1",
      attempt: 1,
    });
    expect(await response.text()).toContain("response.completed");
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe("https://chatgpt.com/backend-api/codex/responses");
    expect(new Headers(calls[0]!.init?.headers).get("authorization"))
      .toBe("Bearer private-access-token");
    expect(new Headers(calls[0]!.init?.headers).get("x-richcodex-data-plane-token"))
      .toBeNull();
    expect(new Headers(calls[0]!.init?.headers).get("x-codex-client-attempt-id"))
      .toBeNull();
    expect(new Headers(calls[0]!.init?.headers).get("chatgpt-account-id"))
      .toBe("workspace-account");
    expect(JSON.parse(String(calls[0]!.init?.body))).toMatchObject({
      model: "gpt-5.6-luna",
      input: [{ role: "user" }],
    });
    expect(statuses).toEqual([["account-1", "ready"]]);
  });

  test("rejects malformed or non-v4 client attempt identity before dispatch", async () => {
    let dispatched = false;
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore([]),
      fetch: async () => {
        dispatched = true;
        return new Response(null, { status: 200 });
      },
    });

    for (const clientAttemptId of [
      "not-a-client-attempt",
      "018f0000-0000-1000-8000-000000000001",
      "018f0000-0000-4000-7000-000000000001",
      "018F0000-0000-4000-8000-000000000001",
    ]) {
      const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
        method: "POST",
        headers: {
          "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
          "content-type": "application/json",
          "x-codex-client-attempt-id": clientAttemptId,
        },
        body: JSON.stringify({ model: "my-fast-model", input: [] }),
      }));

      expect(response.status).toBe(400);
      expect(await response.json()).toMatchObject({
        error: { code: "invalid_client_attempt_id" },
      });
    }
    expect(dispatched).toBe(false);
  });

  test("refreshes expiring OAuth and falls through quota-limited targets", async () => {
    const now = 2_000_000;
    const first: StoredOAuthCredential = {
      kind: "oauth",
      accessToken: "expired-soon-access",
      refreshToken: "first-refresh",
      chatgptAccountId: "workspace-1",
      expiresAt: now + 1,
    };
    const second: StoredOAuthCredential = {
      kind: "oauth",
      accessToken: "second-access",
      refreshToken: "second-refresh",
      chatgptAccountId: "workspace-2",
      expiresAt: now + 3_600_000,
    };
    const replacements: StoredOAuthCredential[] = [];
    const responseAccounts: string[] = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore(
        [
          executionCandidate("account-1", "target-1", 0, first),
          executionCandidate("account-2", "target-2", 1, second),
        ],
        () => undefined,
        (_accountId, _expectedRefreshToken, credential) => {
          replacements.push(credential);
          return true;
        },
      ),
      now: () => now,
      fetch: (async (input, init) => {
        if (String(input).includes("/oauth/token")) {
          return Response.json({
            access_token: "refreshed-access",
            refresh_token: "rotated-refresh",
            expires_in: 3600,
          });
        }
        const account = new Headers(init?.headers).get("chatgpt-account-id")!;
        responseAccounts.push(account);
        if (account === "workspace-1") {
          return new Response("quota", { status: 429, headers: { "retry-after": "60" } });
        }
        return new Response('data: {"type":"response.completed"}\n\n', {
          status: 200,
          headers: {
            "content-type": "text/event-stream",
            "x-richcodex-client-attempt-id": "018f0000-0000-4000-8000-000000000099",
          },
        });
      }) as typeof fetch,
    });

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
        "content-type": "application/json",
        "x-codex-client-attempt-id": TEST_CLIENT_ATTEMPT_ID,
      },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));
    expect(response.status).toBe(200);
    expect(response.headers.get("x-richcodex-account-id")).toBe("account-2");
    expect(response.headers.get("x-richcodex-route-attempt")).toBe("2");
    expect(response.headers.get("x-richcodex-client-attempt-id")).toBe(TEST_CLIENT_ATTEMPT_ID);
    expect(responseAccounts).toEqual(["workspace-1", "workspace-2"]);
    expect(replacements).toEqual([{
      kind: "oauth",
      accessToken: "refreshed-access",
      refreshToken: "rotated-refresh",
      chatgptAccountId: "workspace-1",
      expiresAt: now + 3_600_000,
    }]);
  });

  test("routes API-key accounts to the OpenAI API without forwarding private admission", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const statuses: Array<[string, ProviderAccountStatus]> = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore(
        [executionCandidate("api-account", "api-target", 0, {
          kind: "apiKey",
          apiKey: "sk-private-api-key",
        })],
        (accountId, status) => { statuses.push([accountId, status]); },
      ),
      fetch: (async (input, init) => {
        calls.push({ url: String(input), init });
        return new Response('data: {"type":"response.completed"}\n\n', {
          status: 200,
          headers: { "content-type": "text/event-stream" },
        });
      }) as typeof fetch,
    });

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
        "content-type": "application/json",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));

    expect(response.status).toBe(200);
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe("https://api.openai.com/v1/responses");
    const headers = new Headers(calls[0]!.init?.headers);
    expect(headers.get("authorization")).toBe("Bearer sk-private-api-key");
    expect(headers.get("chatgpt-account-id")).toBeNull();
    expect(headers.get("x-richcodex-data-plane-token")).toBeNull();
    expect(response.headers.get("x-richcodex-client-attempt-id")).toBeNull();
    expect(statuses).toEqual([["api-account", "ready"]]);
  });

  test("does not OAuth-refresh a rejected API key and falls through by target priority", async () => {
    const calls: string[] = [];
    const statuses: Array<[string, ProviderAccountStatus]> = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore(
        [
          executionCandidate("rejected-api", "first-target", 0, {
            kind: "apiKey",
            apiKey: "sk-rejected",
          }),
          executionCandidate("working-api", "second-target", 1, {
            kind: "apiKey",
            apiKey: "sk-working",
          }),
        ],
        (accountId, status) => { statuses.push([accountId, status]); },
      ),
      fetch: (async (input, init) => {
        calls.push(String(input));
        const authorization = new Headers(init?.headers).get("authorization");
        return authorization === "Bearer sk-rejected"
          ? new Response("rejected", { status: 401 })
          : new Response('data: {"type":"response.completed"}\n\n', {
            status: 200,
            headers: { "content-type": "text/event-stream" },
          });
      }) as typeof fetch,
    });

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
        "content-type": "application/json",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));

    expect(response.status).toBe(200);
    expect(calls).toEqual([
      "https://api.openai.com/v1/responses",
      "https://api.openai.com/v1/responses",
    ]);
    expect(calls).not.toContain("https://auth.openai.com/oauth/token");
    expect(statuses).toEqual([
      ["rejected-api", "reauthenticationRequired"],
      ["working-api", "ready"],
    ]);
  });

  test("falls through ordered targets across distinct API-key providers", async () => {
    const calls: Array<{ url: string; authorization: string | null }> = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore([
        executionCandidate("alibaba-account", "alibaba-target", 0, {
          kind: "apiKey",
          apiKey: "private-alibaba-key",
        }, "alibaba", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
        executionCandidate("openrouter-account", "openrouter-target", 1, {
          kind: "apiKey",
          apiKey: "private-openrouter-key",
        }, "openrouter", "https://openrouter.ai/api/v1"),
      ]),
      fetch: (async (input, init) => {
        calls.push({
          url: String(input),
          authorization: new Headers(init?.headers).get("authorization"),
        });
        return calls.length === 1
          ? new Response("busy", { status: 429, headers: { "retry-after": "60" } })
          : new Response('data: {"type":"response.completed"}\n\n', {
              status: 200,
              headers: { "content-type": "text/event-stream" },
            });
      }) as typeof fetch,
    });

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
        "content-type": "application/json",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));

    expect(response.status).toBe(200);
    expect(response.headers.get("x-richcodex-provider-id")).toBe("openrouter");
    expect(response.headers.get("x-richcodex-route-attempt")).toBe("2");
    expect(calls).toEqual([
      {
        url: "https://dashscope.aliyuncs.com/compatible-mode/v1/responses",
        authorization: "Bearer private-alibaba-key",
      },
      {
        url: "https://openrouter.ai/api/v1/responses",
        authorization: "Bearer private-openrouter-key",
      },
    ]);
  });

  test("prefers the soonest trustworthy quota reset and expires evidence back to declared order", async () => {
    let now = 1_000_000;
    const calls: string[] = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore([
        executionCandidate("declared-first", "target-first", 0, {
          kind: "apiKey",
          apiKey: "sk-declared-first",
        }),
        executionCandidate("sooner-reset", "target-sooner", 0, {
          kind: "apiKey",
          apiKey: "sk-sooner-reset",
        }),
      ]),
      now: () => now,
      fetch: (async (_input, init) => {
        const authorization = new Headers(init?.headers).get("authorization")!;
        calls.push(authorization);
        if (calls.length === 1) {
          return new Response("quota", {
            status: 429,
            headers: {
              "retry-after": "60",
              "x-codex-primary-used-percent": "90",
              "x-codex-primary-reset-at": "4000",
            },
          });
        }
        return new Response('data: {"type":"response.completed"}\n\n', {
          status: 200,
          headers: {
            "content-type": "text/event-stream",
            "x-codex-primary-used-percent": "50",
            "x-codex-primary-reset-at": "3000",
          },
        });
      }) as typeof fetch,
    });
    const request = (): Promise<Response> => plane.handle(new Request(
      "http://127.0.0.1/v1/responses",
      {
        method: "POST",
        headers: {
          "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
          "content-type": "application/json",
        },
        body: JSON.stringify({ model: "my-fast-model", input: [] }),
      },
    ));

    expect((await request()).status).toBe(200);
    now += 61_000;
    expect((await request()).headers.get("x-richcodex-account-id")).toBe("sooner-reset");
    now += 16 * 60_000;
    expect((await request()).headers.get("x-richcodex-account-id")).toBe("declared-first");
    expect(calls).toEqual([
      "Bearer sk-declared-first",
      "Bearer sk-sooner-reset",
      "Bearer sk-sooner-reset",
      "Bearer sk-declared-first",
    ]);
  });

  test("keeps declared order when quota evidence is incomplete", async () => {
    let now = 1_000_000;
    const calls: string[] = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore([
        executionCandidate("first-account", "first-target", 0, {
          kind: "apiKey",
          apiKey: "sk-first",
        }),
        executionCandidate("second-account", "second-target", 0, {
          kind: "apiKey",
          apiKey: "sk-second",
        }),
      ]),
      now: () => now,
      fetch: (async (_input, init) => {
        const authorization = new Headers(init?.headers).get("authorization")!;
        calls.push(authorization);
        return calls.length === 1
          ? new Response("quota", {
              status: 429,
              headers: {
                "retry-after": "60",
                "x-codex-primary-used-percent": "90",
                "x-codex-primary-reset-at": "3000",
              },
            })
          : new Response('data: {"type":"response.completed"}\n\n', {
              status: 200,
              headers: { "content-type": "text/event-stream" },
            });
      }) as typeof fetch,
    });
    const request = (): Promise<Response> => plane.handle(new Request(
      "http://127.0.0.1/v1/responses",
      {
        method: "POST",
        headers: {
          "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
          "content-type": "application/json",
        },
        body: JSON.stringify({ model: "my-fast-model", input: [] }),
      },
    ));

    expect((await request()).status).toBe(200);
    now += 61_000;
    expect((await request()).headers.get("x-richcodex-account-id")).toBe("first-account");
    expect(calls).toEqual([
      "Bearer sk-first",
      "Bearer sk-second",
      "Bearer sk-first",
    ]);
  });

  test("never treats OAuth access-token expiry as routing preference", async () => {
    const now = 1_000_000;
    const calls: string[] = [];
    const plane = createModelDataPlane({
      capability: TEST_DATA_PLANE_CAPABILITY,
      modelPlaneStore: executionStore([
        executionCandidate("declared-first", "target-first", 0, {
          kind: "oauth",
          accessToken: "later-expiry",
          refreshToken: "refresh-later",
          chatgptAccountId: "workspace-later",
          expiresAt: now + 2 * 60 * 60_000,
        }),
        executionCandidate("earlier-expiry", "target-earlier", 0, {
          kind: "oauth",
          accessToken: "earlier-expiry",
          refreshToken: "refresh-earlier",
          chatgptAccountId: "workspace-earlier",
          expiresAt: now + 10 * 60_000,
        }),
      ]),
      now: () => now,
      fetch: (async (_input, init) => {
        calls.push(new Headers(init?.headers).get("authorization")!);
        return new Response('data: {"type":"response.completed"}\n\n', {
          status: 200,
          headers: { "content-type": "text/event-stream" },
        });
      }) as typeof fetch,
    });

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        "x-richcodex-data-plane-token": TEST_DATA_PLANE_CAPABILITY,
        "content-type": "application/json",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));
    expect(response.headers.get("x-richcodex-account-id")).toBe("declared-first");
    expect(calls).toEqual(["Bearer later-expiry"]);
  });
});

describe("RichCodex headless backend composition root", () => {
  test("emits a bounded ready shape and resolves only an explicit RichCodex root", () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-headless-"));
    expect(() => resolveBackendStateRoot({
      env: {
        OPENCODEX_HOME: join(root, "legacy-home"),
        CODEX_HOME: join(root, "codex-home"),
      },
    })).toThrow("state_root_missing");
    expect(() => resolveBackendStateRoot({ stateRoot: "relative/backend", env: {} })).toThrow("state_root_not_absolute");

    const normalized = resolveBackendStateRoot({
      stateRoot: join(root, "nested", "..", "backend"),
      env: {
        OPENCODEX_HOME: join(root, "legacy-home"),
        CODEX_HOME: join(root, "codex-home"),
      },
    });
    expect(normalized).toBe(join(root, "backend"));

    const fromEnv = resolveBackendStateRoot({
      env: { [RICHCODEX_BACKEND_STATE_ROOT_ENV]: join(root, "env-backend") },
    });
    expect(fromEnv).toBe(join(root, "env-backend"));
    expect(resolveBackendStateRoot(undefined, {
      [RICHCODEX_BACKEND_STATE_ROOT_ENV]: join(root, "supplied-env-backend"),
    })).toBe(join(root, "supplied-env-backend"));

    const parsed = parseHeadlessBackendArgs(["--state-root", normalized]);
    expect(parsed).toEqual({ stateRoot: normalized });
    expect(parseHeadlessBackendArgs([normalized])).toEqual({ stateRoot: normalized });
  });

  test("correlates shutdown and exits cleanly on EOF", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-headless-shutdown-"));
    const shutdown = await runBackend('{"type":"shutdown","requestId":"request-42"}\n', root);
    expect(shutdown.lines).toHaveLength(2);
    expect(shutdown.lines[0]).toMatchObject({
      type: "ready",
      protocolVersion: 14,
      kernel: RICHCODEX_BACKEND_KERNEL,
      desiredStateRevision: 0,
      catalogRevision: 0,
      providers: [{ id: "openai", displayName: "OpenAI", accountCount: 0, status: "needsAccount" }],
      models: [],
    });
    expect(JSON.stringify(shutdown.lines[0]).length).toBeLessThanOrEqual(RICHCODEX_BACKEND_MAX_MESSAGE_BYTES);
    expect((shutdown.lines[0] as { instanceId: string }).instanceId).toMatch(/^[0-9a-f-]{36}$/);
    expect(shutdown.lines[1]).toEqual({ type: "shutdownComplete", requestId: "request-42" });
    expect(shutdown.result).toEqual({ exitCode: 0, reason: "shutdown" });

    const eof = await runBackend("", root);
    expect(eof.lines).toHaveLength(1);
    expect(eof.result).toEqual({ exitCode: 0, reason: "eof" });
    expect((eof.lines[0] as { instanceId: string }).instanceId)
      .not.toBe((shutdown.lines[0] as { instanceId: string }).instanceId);
  });

  test("rejects malformed and oversized input without reflecting secrets", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-headless-protocol-"));
    const secret = "private-value-that-must-not-appear";
    const oversized = "x".repeat(RICHCODEX_BACKEND_MAX_MESSAGE_BYTES + 1);
    const result = await runBackend(
      `${JSON.stringify({ type: "unknown", secret })}\nnot-json-${secret}\n${oversized}\n{"type":"shutdown","requestId":"request-43"}\n`,
      root,
    );
    expect(result.lines).toHaveLength(5);
    expect(result.lines[1]).toEqual({ type: "protocolError", code: "unknown_message_type", message: "message type is not supported" });
    expect(result.lines[2]).toEqual({ type: "protocolError", code: "malformed_message", message: "message is not a valid protocol object" });
    expect(result.lines[3]).toEqual({ type: "protocolError", code: "message_too_large", message: "message exceeds protocol limit" });
    expect(result.lines[4]).toEqual({ type: "shutdownComplete", requestId: "request-43" });
    expect(JSON.stringify(result.lines)).not.toContain(secret);
    expect(result.stderr).toEqual([]);
  });

  test("correlates provider device-login lifecycle messages without credential fields", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-headless-device-login-"));
    const calls: string[] = [];
    const login = (
      status: SafeProviderLogin["status"],
      verificationUrl: string | null,
      userCode: string | null,
    ): SafeProviderLogin => ({
      loginId: "login-safe-handle",
      status,
      verificationUrl,
      userCode,
      expiresAt: 2_000,
      failure: null,
      account: null,
      desiredStateRevision: 0,
      catalogRevision: 0,
    });
    const deviceOAuthCoordinator: DeviceOAuthCoordinator = {
      start: async userLabel => {
        calls.push(`start:${userLabel}`);
        return login("awaitingUser", "https://auth.openai.com/codex/device", "SAFE-CODE");
      },
      status: loginId => {
        calls.push(`status:${loginId}`);
        return login("exchanging", null, null);
      },
      cancel: loginId => {
        calls.push(`cancel:${loginId}`);
        return login("cancelled", null, null);
      },
      shutdown: () => { calls.push("shutdown"); },
    };

    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountLoginStart",
        requestId: "login-start",
        userLabel: "Third Codex",
        mode: "deviceCode",
      })}\n${JSON.stringify({
        type: "providerAccountLoginStatus",
        requestId: "login-status",
        loginId: "login-safe-handle",
      })}\n${JSON.stringify({
        type: "providerAccountLoginCancel",
        requestId: "login-cancel",
        loginId: "login-safe-handle",
      })}\n${JSON.stringify({ type: "shutdown", requestId: "login-shutdown" })}\n`,
      root,
      deviceOAuthCoordinator,
    );

    expect(result.lines.slice(1)).toEqual([
      {
        type: "providerAccountLoginStartResult",
        requestId: "login-start",
        ...login("awaitingUser", "https://auth.openai.com/codex/device", "SAFE-CODE"),
      },
      {
        type: "providerAccountLoginStatusResult",
        requestId: "login-status",
        ...login("exchanging", null, null),
      },
      {
        type: "providerAccountLoginCancelResult",
        requestId: "login-cancel",
        ...login("cancelled", null, null),
      },
      { type: "shutdownComplete", requestId: "login-shutdown" },
    ]);
    expect(calls).toEqual([
      "start:Third Codex",
      "status:login-safe-handle",
      "cancel:login-safe-handle",
      "shutdown",
    ]);
    expect(JSON.stringify(result.lines)).not.toContain("access_token");
    expect(JSON.stringify(result.lines)).not.toContain("refresh_token");
  });

  test("routes browser-login lifecycle messages to the browser coordinator", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-headless-browser-login-"));
    const calls: string[] = [];
    const login = (status: SafeProviderLogin["status"], url: string | null): SafeProviderLogin => ({
      loginId: "browser-safe-handle",
      status,
      verificationUrl: url,
      userCode: null,
      expiresAt: 2_000,
      failure: null,
      account: null,
      desiredStateRevision: 0,
      catalogRevision: 0,
    });
    const browserOAuthCoordinator: DeviceOAuthCoordinator = {
      start: async userLabel => {
        calls.push(`start:${userLabel}`);
        return login("awaitingUser", "https://auth.openai.com/oauth/authorize?safe=1");
      },
      status: loginId => {
        calls.push(`status:${loginId}`);
        return login("exchanging", null);
      },
      cancel: loginId => {
        calls.push(`cancel:${loginId}`);
        return login("cancelled", null);
      },
      shutdown: () => { calls.push("shutdown"); },
    };

    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountLoginStart",
        requestId: "browser-start",
        userLabel: "Browser Codex",
        mode: "browser",
      })}\n${JSON.stringify({
        type: "providerAccountLoginStatus",
        requestId: "browser-status",
        loginId: "browser-safe-handle",
      })}\n${JSON.stringify({
        type: "providerAccountLoginCancel",
        requestId: "browser-cancel",
        loginId: "browser-safe-handle",
      })}\n${JSON.stringify({ type: "shutdown", requestId: "browser-shutdown" })}\n`,
      root,
      undefined,
      browserOAuthCoordinator,
    );

    expect(result.lines.slice(1)).toEqual([
      {
        type: "providerAccountLoginStartResult",
        requestId: "browser-start",
        ...login("awaitingUser", "https://auth.openai.com/oauth/authorize?safe=1"),
      },
      {
        type: "providerAccountLoginStatusResult",
        requestId: "browser-status",
        ...login("exchanging", null),
      },
      {
        type: "providerAccountLoginCancelResult",
        requestId: "browser-cancel",
        ...login("cancelled", null),
      },
      { type: "shutdownComplete", requestId: "browser-shutdown" },
    ]);
    expect(calls).toEqual([
      "start:Browser Codex",
      "status:browser-safe-handle",
      "cancel:browser-safe-handle",
      "shutdown",
    ]);
  });

  test("imports an explicitly selected Codex login and lists only safe account state", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-provider-account-"));
    const stateRoot = join(root, "backend-state");
    const source = join(root, "selected-auth.json");
    const accountId = "provider-account-identity-canary";
    const sourceBytes = codexAuthJson(accountId, Date.now() + 3_600_000);
    writeFileSync(source, sourceBytes, { mode: 0o600 });

    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountImport",
        requestId: "import-1",
        authJsonPath: source,
        userLabel: "Secondary ChatGPT",
      })}\n${JSON.stringify({
        type: "providerAccountList",
        requestId: "list-1",
        cursor: null,
        limit: 10,
      })}\n${JSON.stringify({ type: "shutdown", requestId: "shutdown-1" })}\n`,
      stateRoot,
    );

    expect(result.result).toEqual({ exitCode: 0, reason: "shutdown" });
    expect(result.stderr).toEqual([]);
    expect(result.lines[0]).toMatchObject({
      type: "ready",
      desiredStateRevision: 0,
      catalogRevision: 0,
      providers: [{ id: "openai", accountCount: 0, status: "needsAccount" }],
    });
    expect(result.lines[1]).toMatchObject({
      type: "providerAccountImportResult",
      requestId: "import-1",
      desiredStateRevision: 1,
      catalogRevision: 1,
      account: {
        providerId: "openai",
        userLabel: "Secondary ChatGPT",
        status: "verificationRequired",
      },
    });
    expect(result.lines[2]).toMatchObject({
      type: "providerAccountListResult",
      requestId: "list-1",
      desiredStateRevision: 1,
      catalogRevision: 1,
      providers: [{ id: "openai", accountCount: 1, status: "ready" }],
      data: [{ providerId: "openai", userLabel: "Secondary ChatGPT", status: "verificationRequired" }],
      nextCursor: null,
    });
    const output = JSON.stringify(result.lines);
    expect(output).not.toContain(accountId);
    expect(output).not.toContain("refresh-");
    expect(output).not.toContain(source);
    expect(readFileSync(source, "utf8")).toBe(sourceBytes);
    const persisted = join(stateRoot, "model-plane.json");
    expect(existsSync(persisted)).toBe(true);
    if (process.platform !== "win32") expect(statSync(persisted).mode & 0o777).toBe(0o600);
  });

  test("installs client-owned ChatGPT tokens without reflecting the credential", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-client-auth-tokens-"));
    const token = "private-client-token-canary";
    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAuthTokensInstall",
        requestId: "client-token-install",
        accessToken: token,
        chatgptAccountId: "client-workspace",
        chatgptPlanType: "pro",
        userLabel: "Codex Desktop",
        accountId: null,
      })}\n${JSON.stringify({
        type: "providerAccountList",
        requestId: "client-token-list",
        cursor: null,
        limit: 10,
      })}\n${JSON.stringify({ type: "shutdown", requestId: "client-token-shutdown" })}\n`,
      root,
    );
    expect(result.lines[0]).toMatchObject({ type: "ready", protocolVersion: 14 });
    expect(result.lines[1]).toMatchObject({
      type: "providerAccountAuthTokensInstallResult",
      requestId: "client-token-install",
      account: {
        providerId: "openai",
        userLabel: "Codex Desktop",
        status: "verificationRequired",
        planType: "pro",
      },
    });
    expect(result.lines[2]).toMatchObject({
      type: "providerAccountListResult",
      data: [{ providerId: "openai", planType: "pro" }],
    });
    expect(JSON.stringify(result.lines)).not.toContain(token);
    expect(result.stderr).toEqual([]);
  });

  test("returns only the explicitly requested OAuth access token", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-auth-token-read-"));
    const token = "private-client-token-canary";
    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAuthTokensInstall",
        requestId: "client-token-install",
        accessToken: token,
        chatgptAccountId: "client-workspace",
        chatgptPlanType: "pro",
        userLabel: "Codex Desktop",
        accountId: null,
      })}\n${JSON.stringify({ type: "shutdown", requestId: "client-token-shutdown" })}\n`,
      root,
    );
    const installed = result.lines[1] as {
      account: { id: string };
    };
    expect(installed.account.id).toBeTruthy();

    const replay = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAuthTokenRead",
        requestId: "client-token-read",
        accountId: installed.account.id,
      })}\n${JSON.stringify({ type: "shutdown", requestId: "client-token-shutdown" })}\n`,
      root,
    );
    expect(replay.lines[1]).toEqual({
      type: "providerAccountAuthTokenReadResult",
      requestId: "client-token-read",
      accessToken: token,
    });
    expect(JSON.stringify(replay.lines)).not.toContain("refreshToken");
    expect(replay.stderr).toEqual([]);
  });

  test("replaces client-owned tokens without duplicating the durable account", () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-client-auth-replace-"));
    const firstStore = createModelPlaneStore(root, {
      createAccountId: () => "client-account-stable",
    });
    const first = firstStore.installClientAuthTokens({
      accessToken: "first-client-token",
      chatgptAccountId: "client-workspace",
      chatgptPlanType: "plus",
      userLabel: "Codex Desktop",
      accountId: null,
    });
    const replaced = firstStore.installClientAuthTokens({
      accessToken: "second-client-token",
      chatgptAccountId: "client-workspace",
      chatgptPlanType: "pro",
      userLabel: "Codex Desktop",
      accountId: first.id,
    });
    const restarted = createModelPlaneStore(root).installClientAuthTokens({
      accessToken: "third-client-token",
      chatgptAccountId: "client-workspace",
      chatgptPlanType: null,
      userLabel: "Codex Desktop",
      accountId: null,
    });
    expect(replaced.id).toBe("client-account-stable");
    expect(restarted).toMatchObject({ id: "client-account-stable", planType: "pro" });
    expect(createModelPlaneStore(root).snapshot().accounts).toHaveLength(1);
  });

  test("accepts Codex login metadata for distinct token and selected workspaces", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-provider-account-workspaces-"));
    const stateRoot = join(root, "backend-state");
    const source = join(root, "selected-auth.json");
    writeFileSync(source, codexAuthJsonWithDistinctAccountMetadata(Date.now() + 3_600_000), {
      mode: 0o600,
    });

    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountImport",
        requestId: "import-distinct-workspaces",
        authJsonPath: source,
        userLabel: "Distinct Workspaces",
      })}\n${JSON.stringify({ type: "shutdown", requestId: "shutdown-distinct-workspaces" })}\n`,
      stateRoot,
    );

    expect(result.result).toEqual({ exitCode: 0, reason: "shutdown" });
    expect(result.stderr).toEqual([]);
    expect(result.lines[1]).toMatchObject({
      type: "providerAccountImportResult",
      requestId: "import-distinct-workspaces",
      desiredStateRevision: 1,
      catalogRevision: 1,
      account: { providerId: "openai", status: "verificationRequired" },
    });
  });

  test("adds an API-key account without reflecting its secret", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-provider-api-key-"));
    const stateRoot = join(root, "backend-state");
    const apiKey = "sk-private-provider-key";

    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAddApiKey",
        requestId: "add-api-key",
        providerId: "openai",
        providerDisplayName: "OpenAI",
        apiBaseUrl: "https://api.openai.com/v1",
        apiKey,
        userLabel: "OpenAI API",
      })}\n${JSON.stringify({ type: "providerAccountList", requestId: "list-api-key" })}\n`,
      stateRoot,
    );

    expect(result.lines[1]).toMatchObject({
      type: "providerAccountAddApiKeyResult",
      desiredStateRevision: 1,
      catalogRevision: 1,
      account: {
        providerId: "openai",
        userLabel: "OpenAI API",
        credentialKind: "apiKey",
        status: "verificationRequired",
      },
    });
    expect(result.lines[2]).toMatchObject({
      type: "providerAccountListResult",
      data: [{ credentialKind: "apiKey", userLabel: "OpenAI API" }],
    });
    expect(JSON.stringify(result.lines)).not.toContain(apiKey);
    expect(result.stderr.join("\n")).not.toContain(apiKey);
    expect(readFileSync(join(stateRoot, "model-plane.json"), "utf8")).toContain(apiKey);
    if (process.platform !== "win32") {
      expect(statSync(join(stateRoot, "model-plane.json")).mode & 0o777).toBe(0o600);
    }
  });

  test("persists a distinct compatible provider and binds it to a stable model tag", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-compatible-provider-"));
    const stateRoot = join(root, "backend-state");
    const apiKey = "private-dashscope-key";
    const added = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAddApiKey",
        requestId: "add-dashscope",
        providerId: "alibaba",
        providerDisplayName: "Alibaba Model Studio",
        apiBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        apiKey,
        userLabel: "Alibaba Primary",
      })}\n`,
      stateRoot,
    );
    const accountId = (added.lines[1] as { account: { id: string } }).account.id;
    const created = await runBackend(
      `${JSON.stringify({
        type: "modelRouteCreate",
        requestId: "create-cross-provider-route",
        expectedRevision: 1,
        modelTag: "qwen-coder",
        displayName: "Qwen Coder",
        semanticModel: "gpt-5.6-luna",
        providerId: "alibaba",
        accountId,
        upstreamModelId: "qwen3-coder-plus",
      })}\n`,
      stateRoot,
    );

    expect(added.lines[0]).toMatchObject({
      providers: [{ id: "openai", status: "needsAccount" }],
    });
    expect(added.lines[1]).toMatchObject({
      account: {
        providerId: "alibaba",
        userLabel: "Alibaba Primary",
        credentialKind: "apiKey",
      },
    });
    expect(created.lines[0]).toMatchObject({
      providers: [
        { id: "openai", accountCount: 0, status: "needsAccount" },
        { id: "alibaba", displayName: "Alibaba Model Studio", accountCount: 1, status: "ready" },
      ],
    });
    expect(created.lines[1]).toMatchObject({
      type: "modelRouteCreateResult",
      route: {
        modelTag: "qwen-coder",
        targets: [{
          providerId: "alibaba",
          accountId,
          upstreamModelId: "qwen3-coder-plus",
        }],
      },
    });
    const safeOutput = JSON.stringify([...added.lines, ...created.lines]);
    expect(safeOutput).not.toContain(apiKey);
    expect(safeOutput).not.toContain("dashscope.aliyuncs.com");
  });

  test("creates, reads, retires, and restores one account-bound model route", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-model-route-"));
    const stateRoot = join(root, "backend-state");
    const source = join(root, "selected-auth.json");
    writeFileSync(source, codexAuthJson("route-account", Date.now() + 3_600_000), { mode: 0o600 });

    const imported = await runBackend(
      `${JSON.stringify({
        type: "providerAccountImport",
        requestId: "import-route-account",
        authJsonPath: source,
        userLabel: "Route Account",
      })}\n`,
      stateRoot,
    );
    const importedAccount = (imported.lines[1] as { account: { id: string } }).account;
    const created = await runBackend(
      `${JSON.stringify({
        type: "modelRouteCreate",
        requestId: "create-route",
        expectedRevision: 1,
        modelTag: "gpt-primary",
        displayName: "GPT Primary",
        semanticModel: "openai/gpt-primary",
        providerId: "openai",
        accountId: importedAccount.id,
        upstreamModelId: "gpt-primary-2026-08-13",
      })}\n${JSON.stringify({ type: "modelRouteRead", requestId: "read-route" })}\n`,
      stateRoot,
    );

    expect(created.lines[0]).toMatchObject({
      desiredStateRevision: 1,
      catalogRevision: 1,
      models: [],
    });
    expect(created.lines[1]).toMatchObject({
      type: "modelRouteCreateResult",
      desiredStateRevision: 2,
      catalogRevision: 2,
      route: {
        modelTag: "gpt-primary",
        displayName: "GPT Primary",
        retired: false,
        semanticModel: "openai/gpt-primary",
        targets: [{
          providerId: "openai",
          accountId: importedAccount.id,
          upstreamModelId: "gpt-primary-2026-08-13",
          priority: 0,
          status: "unverified",
        }],
      },
    });
    expect(created.lines[2]).toMatchObject({
      type: "modelRouteReadResult",
      desiredStateRevision: 2,
      catalogRevision: 2,
      data: [{ modelTag: "gpt-primary", retired: false }],
    });

    const retired = await runBackend(
      `${JSON.stringify({
        type: "modelRouteRetire",
        requestId: "retire-route",
        expectedRevision: 2,
        modelTag: "gpt-primary",
      })}\n${JSON.stringify({ type: "shutdown", requestId: "shutdown-route" })}\n`,
      stateRoot,
    );
    expect(retired.lines[0]).toMatchObject({
      desiredStateRevision: 2,
      catalogRevision: 2,
      models: [{ modelTag: "gpt-primary", retired: false }],
    });
    expect(retired.lines[1]).toMatchObject({
      type: "modelRouteRetireResult",
      desiredStateRevision: 3,
      catalogRevision: 3,
      route: { modelTag: "gpt-primary", retired: true },
    });

    const restored = await runBackend(
      `${JSON.stringify({ type: "modelRouteRead", requestId: "read-restored-route" })}\n`,
      stateRoot,
    );
    expect(restored.lines[0]).toMatchObject({
      desiredStateRevision: 3,
      catalogRevision: 3,
      models: [],
    });
    expect(restored.lines[1]).toMatchObject({
      type: "modelRouteReadResult",
      data: [{ modelTag: "gpt-primary", retired: true, targets: [{ accountId: importedAccount.id }] }],
    });
    const serialized = JSON.stringify([...created.lines, ...retired.lines, ...restored.lines]);
    expect(serialized).not.toContain("refresh-route-account");
    expect(serialized).not.toContain("route-account");
  });

  test("owns device OAuth tokens while exposing only a safe multi-account login handle", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-device-oauth-"));
    const stateRoot = join(root, "backend-state");
    const now = 1_000_000;
    const calls: Array<{
      url: string;
      body: string;
      proxy?: string;
    }> = [];
    const responses = [
      new Response(JSON.stringify({
        device_auth_id: "private-device-auth-id",
        user_code: "ABCD-EFGH",
        interval: "1",
      }), { status: 200 }),
      new Response(JSON.stringify({
        authorization_code: "private-authorization-code",
        code_challenge: "private-code-challenge",
        code_verifier: "private-code-verifier",
      }), { status: 200 }),
      new Response(JSON.stringify({
        id_token: jwt({
          exp: Math.floor(now / 1000) + 3600,
          "https://api.openai.com/auth": {
            chatgpt_account_id: "private-upstream-account",
          },
        }),
        access_token: jwt({ exp: Math.floor(now / 1000) + 3600 }),
        refresh_token: "private-refresh-token",
      }), { status: 200 }),
    ];
    const modelPlaneStore = createModelPlaneStore(stateRoot, {
      now: () => now,
      createAccountId: () => "account-local-handle",
    });
    const coordinator = createDeviceOAuthCoordinator({
      modelPlaneStore,
      env: { HTTPS_PROXY: "http://127.0.0.1:7890" },
      now: () => now,
      createLoginId: () => "login-local-handle",
      fetch: async (input, init) => {
        calls.push({
          url: String(input),
          body: String(init?.body ?? ""),
          proxy: init?.proxy,
        });
        const response = responses.shift();
        if (!response) throw new Error("unexpected OAuth fetch");
        return response;
      },
    });

    const started = await coordinator.start("Second Codex");
    expect(started).toMatchObject({
      loginId: "login-local-handle",
      verificationUrl: "https://auth.openai.com/codex/device",
      userCode: "ABCD-EFGH",
      expiresAt: 1_900,
      failure: null,
      account: null,
    });

    let completed = coordinator.status(started.loginId);
    for (let attempt = 0; attempt < 20 && completed.status !== "completed"; attempt += 1) {
      await new Promise(resolve => setTimeout(resolve, 0));
      completed = coordinator.status(started.loginId);
    }
    expect(completed).toEqual({
      loginId: "login-local-handle",
      status: "completed",
      verificationUrl: null,
      userCode: null,
      expiresAt: 1_900,
      failure: null,
      account: {
        id: "account-local-handle",
        providerId: "openai",
        userLabel: "Second Codex",
        credentialKind: "oauth",
        status: "verificationRequired",
        addedAt: 1_000,
      },
      desiredStateRevision: 1,
      catalogRevision: 1,
    });
    expect(calls.map(call => call.url)).toEqual([
      "https://auth.openai.com/api/accounts/deviceauth/usercode",
      "https://auth.openai.com/api/accounts/deviceauth/token",
      "https://auth.openai.com/oauth/token",
    ]);
    expect(calls.every(call => call.proxy === "http://127.0.0.1:7890")).toBe(true);
    expect(calls[0]!.body).toBe(JSON.stringify({
      client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    }));
    expect(calls[2]!.body).toContain("redirect_uri=https%3A%2F%2Fauth.openai.com%2Fdeviceauth%2Fcallback");

    const safeSerialization = JSON.stringify({ started, completed });
    expect(safeSerialization).not.toContain("private-device-auth-id");
    expect(safeSerialization).not.toContain("private-authorization-code");
    expect(safeSerialization).not.toContain("private-code-verifier");
    expect(safeSerialization).not.toContain("private-refresh-token");
    expect(safeSerialization).not.toContain("private-upstream-account");
    coordinator.shutdown();
  });

  test("keeps two OAuth accounts across restart and fails them independently", () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-multiple-oauth-"));
    const stateRoot = join(root, "backend-state");
    const now = 1_000_000;
    const accountIds = ["oauth-account-a", "oauth-account-b"];
    const store = createModelPlaneStore(stateRoot, {
      now: () => now,
      createAccountId: () => accountIds.shift()!,
    });
    store.addOAuthAccount({
      kind: "oauth",
      accessToken: "private-access-a",
      refreshToken: "private-refresh-a",
      chatgptAccountId: "private-workspace-a",
      expiresAt: now + 3_600_000,
    }, "Codex A");
    store.addOAuthAccount({
      kind: "oauth",
      accessToken: "private-access-b",
      refreshToken: "private-refresh-b",
      chatgptAccountId: "private-workspace-b",
      expiresAt: now + 3_600_000,
    }, "Codex B");
    store.markAccountStatus("oauth-account-a", "reauthenticationRequired");

    const restored = createModelPlaneStore(stateRoot, { now: () => now }).snapshot();
    expect(restored.accounts).toEqual([
      {
        id: "oauth-account-a",
        providerId: "openai",
        userLabel: "Codex A",
        credentialKind: "oauth",
        status: "reauthenticationRequired",
        addedAt: 1_000,
      },
      {
        id: "oauth-account-b",
        providerId: "openai",
        userLabel: "Codex B",
        credentialKind: "oauth",
        status: "verificationRequired",
        addedAt: 1_000,
      },
    ]);
    expect(restored.providers).toEqual([{
      id: "openai",
      displayName: "OpenAI",
      accountCount: 2,
      status: "ready",
    }]);
    const safeSnapshot = JSON.stringify(restored);
    expect(safeSnapshot).not.toContain("private-access");
    expect(safeSnapshot).not.toContain("private-refresh");
    expect(safeSnapshot).not.toContain("private-workspace");
  });

  test("previews account removal, refuses referenced accounts, and reauthenticates in place", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-account-lifecycle-"));
    const stateRoot = join(root, "backend-state");
    const added = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAddApiKey",
        requestId: "add-bound",
        providerId: "openai",
        providerDisplayName: "OpenAI",
        apiBaseUrl: "https://api.openai.com/v1",
        apiKey: "sk-bound-old",
        userLabel: "Bound API",
      })}\n${JSON.stringify({
        type: "providerAccountAddApiKey",
        requestId: "add-unused",
        providerId: "openai",
        providerDisplayName: "OpenAI",
        apiBaseUrl: "https://api.openai.com/v1",
        apiKey: "sk-unused",
        userLabel: "Unused API",
      })}\n`,
      stateRoot,
    );
    const boundId = (added.lines[1] as { account: { id: string } }).account.id;
    const unusedId = (added.lines[2] as { account: { id: string } }).account.id;
    await runBackend(`${JSON.stringify({
      type: "modelRouteCreate",
      requestId: "create-bound-route",
      expectedRevision: 2,
      modelTag: "bound-model",
      displayName: "Bound Model",
      semanticModel: "gpt-5.4",
      providerId: "openai",
      accountId: boundId,
      upstreamModelId: "gpt-5.4",
    })}\n`, stateRoot);

    const lifecycle = await runBackend(
      `${JSON.stringify({
        type: "providerAccountRemovalPreview",
        requestId: "preview-bound",
        accountId: boundId,
      })}\n${JSON.stringify({
        type: "providerAccountRemove",
        requestId: "remove-bound",
        expectedRevision: 3,
        accountId: boundId,
      })}\n${JSON.stringify({
        type: "providerAccountReplaceApiKey",
        requestId: "replace-bound-key",
        expectedRevision: 3,
        accountId: boundId,
        apiKey: "sk-bound-new",
      })}\n${JSON.stringify({
        type: "providerAccountRemovalPreview",
        requestId: "preview-unused",
        accountId: unusedId,
      })}\n${JSON.stringify({
        type: "providerAccountRemove",
        requestId: "remove-unused",
        expectedRevision: 4,
        accountId: unusedId,
      })}\n${JSON.stringify({ type: "providerAccountList", requestId: "list-after" })}\n`,
      stateRoot,
    );

    expect(lifecycle.lines[1]).toMatchObject({
      type: "providerAccountRemovalPreviewResult",
      desiredStateRevision: 3,
      canRemove: false,
      affectedTargets: [{ modelTag: "bound-model", upstreamModelId: "gpt-5.4" }],
    });
    expect(lifecycle.lines[2]).toMatchObject({ type: "operationError", code: "account_in_use" });
    expect(lifecycle.lines[3]).toMatchObject({
      type: "providerAccountReplaceApiKeyResult",
      desiredStateRevision: 4,
      account: { id: boundId, status: "verificationRequired" },
    });
    expect(lifecycle.lines[4]).toMatchObject({ canRemove: true, affectedTargets: [] });
    expect(lifecycle.lines[5]).toMatchObject({
      type: "providerAccountRemoveResult",
      desiredStateRevision: 5,
      account: { id: unusedId },
    });
    expect(lifecycle.lines[6]).toMatchObject({
      desiredStateRevision: 5,
      data: [{ id: boundId }],
    });
    expect(JSON.stringify(lifecycle.lines)).not.toContain("sk-bound");
    expect(JSON.stringify(lifecycle.lines)).not.toContain("sk-unused");
  });

  test("OAuth reauthentication preserves the opaque handle and upstream identity", () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-oauth-reauth-"));
    const now = 1_000_000;
    const store = createModelPlaneStore(join(root, "backend-state"), {
      now: () => now,
      createAccountId: () => "oauth-stable-handle",
    });
    const original = store.addOAuthAccount({
      kind: "oauth",
      accessToken: "private-old-access",
      refreshToken: "private-old-refresh",
      chatgptAccountId: "private-upstream-account",
      expiresAt: now + 3_600_000,
    }, "Codex Primary");
    const replaced = store.reauthenticateOAuthAccount(original.id, {
      kind: "oauth",
      accessToken: "private-new-access",
      refreshToken: "private-new-refresh",
      chatgptAccountId: "private-upstream-account",
      expiresAt: now + 7_200_000,
    });
    expect(replaced).toEqual({ ...original, status: "verificationRequired" });
    expect(store.snapshot().desiredStateRevision).toBe(2);
    expect(() => store.reauthenticateOAuthAccount(original.id, {
      kind: "oauth",
      accessToken: "private-other-access",
      refreshToken: "private-other-refresh",
      chatgptAccountId: "different-private-account",
      expiresAt: now + 7_200_000,
    })).toThrow("account_identity_mismatch");
  });

  test("device OAuth reauthentication rejects a non-OAuth handle before contacting OpenAI", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-oauth-reauth-preflight-"));
    const modelPlaneStore = createModelPlaneStore(join(root, "backend-state"), {
      createAccountId: () => "api-key-handle",
    });
    const account = modelPlaneStore.addApiKeyAccount({
      providerId: "openai",
      providerDisplayName: "OpenAI",
      apiBaseUrl: "https://api.openai.com/v1",
      apiKey: "private-api-key",
      userLabel: "API key",
    });
    let contactedOpenAi = false;
    const coordinator = createDeviceOAuthCoordinator({
      modelPlaneStore,
      fetch: async () => {
        contactedOpenAi = true;
        return new Response("", { status: 500 });
      },
    });

    await expect(coordinator.start("API key", account.id)).rejects.toThrow(
      "credential_kind_mismatch",
    );
    expect(contactedOpenAi).toBe(false);
    coordinator.shutdown();
  });

  test("cancels a pending device OAuth flow without publishing private state", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-device-oauth-cancel-"));
    const modelPlaneStore = createModelPlaneStore(join(root, "backend-state"));
    const coordinator = createDeviceOAuthCoordinator({
      modelPlaneStore,
      createLoginId: () => "login-cancel",
      fetch: async input => String(input).endsWith("/usercode")
        ? new Response(JSON.stringify({
            device_auth_id: "private-device-id",
            user_code: "CANCEL-ME",
            interval: "1",
          }), { status: 200 })
        : new Response("", { status: 403 }),
      sleep: (_milliseconds, signal) => signal.aborted
        ? Promise.reject(new DOMException("Aborted", "AbortError"))
        : new Promise((_resolve, reject) => {
            signal.addEventListener(
              "abort",
              () => reject(new DOMException("Aborted", "AbortError")),
              { once: true },
            );
          }),
    });

    const started = await coordinator.start("Cancelled Codex");
    const cancelled = coordinator.cancel(started.loginId);
    expect(cancelled).toMatchObject({
      loginId: "login-cancel",
      status: "cancelled",
      verificationUrl: null,
      userCode: null,
      failure: null,
      account: null,
    });
    expect(JSON.stringify(cancelled)).not.toContain("private-device-id");
    coordinator.shutdown();
  });

  test("atomically replaces ordered model targets while preserving explicit target identity", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-model-targets-"));
    const stateRoot = join(root, "backend-state");
    const accounts = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAddApiKey",
        requestId: "add-primary-account",
        providerId: "openai",
        providerDisplayName: "OpenAI",
        apiBaseUrl: "https://api.openai.com/v1",
        apiKey: "sk-primary-target",
        userLabel: "Primary API",
      })}\n${JSON.stringify({
        type: "providerAccountAddApiKey",
        requestId: "add-secondary-account",
        providerId: "openai",
        providerDisplayName: "OpenAI",
        apiBaseUrl: "https://api.openai.com/v1",
        apiKey: "sk-secondary-target",
        userLabel: "Secondary API",
      })}\n`,
      stateRoot,
    );
    const primaryAccountId = (accounts.lines[1] as { account: { id: string } }).account.id;
    const secondaryAccountId = (accounts.lines[2] as { account: { id: string } }).account.id;
    const created = await runBackend(
      `${JSON.stringify({
        type: "modelRouteCreate",
        requestId: "create-target-route",
        expectedRevision: 2,
        modelTag: "ordered-route",
        displayName: "Ordered Route",
        semanticModel: "gpt-5.4",
        providerId: "openai",
        accountId: primaryAccountId,
        upstreamModelId: "gpt-primary",
      })}\n`,
      stateRoot,
    );
    const originalTargetId = (created.lines[1] as {
      route: { targets: [{ id: string }] };
    }).route.targets[0].id;

    const updated = await runBackend(
      `${JSON.stringify({
        type: "modelRouteSetTargets",
        requestId: "set-ordered-targets",
        expectedRevision: 3,
        modelTag: "ordered-route",
        targets: [
          {
            id: null,
            providerId: "openai",
            accountId: secondaryAccountId,
            upstreamModelId: "gpt-secondary",
          },
          {
            id: originalTargetId,
            providerId: "openai",
            accountId: primaryAccountId,
            upstreamModelId: "gpt-primary-revised",
          },
        ],
      })}\n${JSON.stringify({ type: "modelRouteRead", requestId: "read-ordered-targets" })}\n`,
      stateRoot,
    );

    expect(updated.lines[1]).toMatchObject({
      type: "modelRouteSetTargetsResult",
      desiredStateRevision: 4,
      catalogRevision: 4,
      route: {
        modelTag: "ordered-route",
        targets: [
          {
            accountId: secondaryAccountId,
            upstreamModelId: "gpt-secondary",
            priority: 0,
          },
          {
            id: originalTargetId,
            accountId: primaryAccountId,
            upstreamModelId: "gpt-primary-revised",
            priority: 1,
          },
        ],
      },
    });
    const newTargetId = (updated.lines[1] as {
      route: { targets: [{ id: string }, { id: string }] };
    }).route.targets[0].id;
    expect(newTargetId).not.toBe(originalTargetId);
    expect(updated.lines[2]).toMatchObject({
      type: "modelRouteReadResult",
      desiredStateRevision: 4,
      data: [{ modelTag: "ordered-route", targets: [{ id: newTargetId }, { id: originalTargetId }] }],
    });
  });

  test("renames a provider account without changing its handle, credential, or model targets", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-account-rename-"));
    const stateRoot = join(root, "backend-state");
    const added = await runBackend(
      `${JSON.stringify({
        type: "providerAccountAddApiKey",
        requestId: "add-account",
        providerId: "openai",
        providerDisplayName: "OpenAI",
        apiBaseUrl: "https://api.openai.com/v1",
        apiKey: "sk-rename-preserved-secret",
        userLabel: "Before",
      })}\n`,
      stateRoot,
    );
    const accountId = (added.lines[1] as { account: { id: string } }).account.id;
    const result = await runBackend(
      `${JSON.stringify({
        type: "modelRouteCreate",
        requestId: "create-route",
        expectedRevision: 1,
        modelTag: "rename-route",
        displayName: "Rename Route",
        semanticModel: "gpt-5.4",
        providerId: "openai",
        accountId,
        upstreamModelId: "gpt-5.4",
      })}\n${JSON.stringify({
        type: "providerAccountRename",
        requestId: "rename-account",
        expectedRevision: 2,
        accountId,
        userLabel: "After",
      })}\n${JSON.stringify({
        type: "providerAccountList",
        requestId: "list-accounts",
        cursor: null,
        limit: null,
      })}\n${JSON.stringify({
        type: "modelRouteRead",
        requestId: "read-routes",
      })}\n`,
      stateRoot,
    );

    expect(result.lines[2]).toMatchObject({
      type: "providerAccountRenameResult",
      desiredStateRevision: 3,
      account: { id: accountId, userLabel: "After" },
    });
    expect(result.lines[3]).toMatchObject({
      data: [{ id: accountId, userLabel: "After" }],
    });
    expect(result.lines[4]).toMatchObject({
      data: [{ modelTag: "rename-route", targets: [{ accountId }] }],
    });
    expect(JSON.stringify(result.lines)).not.toContain("sk-rename-preserved-secret");
  });

  test("rejects stale route creation without changing the model plane", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-model-route-conflict-"));
    const stateRoot = join(root, "backend-state");
    const source = join(root, "selected-auth.json");
    writeFileSync(source, codexAuthJson("conflict-account", Date.now() + 3_600_000), { mode: 0o600 });
    const imported = await runBackend(
      `${JSON.stringify({
        type: "providerAccountImport",
        requestId: "import-conflict-account",
        authJsonPath: source,
        userLabel: "Conflict Account",
      })}\n`,
      stateRoot,
    );
    const accountId = (imported.lines[1] as { account: { id: string } }).account.id;
    const result = await runBackend(
      `${JSON.stringify({
        type: "modelRouteCreate",
        requestId: "stale-route",
        expectedRevision: 0,
        modelTag: "stale-route",
        displayName: "Stale Route",
        semanticModel: "openai/stale-route",
        providerId: "openai",
        accountId,
        upstreamModelId: "stale-route",
      })}\n${JSON.stringify({ type: "modelRouteRead", requestId: "read-after-conflict" })}\n`,
      stateRoot,
    );
    expect(result.lines[1]).toEqual({
      type: "operationError",
      requestId: "stale-route",
      code: "revision_conflict",
      message: "model plane revision does not match",
    });
    expect(result.lines[2]).toMatchObject({
      desiredStateRevision: 1,
      catalogRevision: 1,
      data: [],
    });
  });

  test("migrates the Slice 1A account document into one revisioned model plane", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-model-plane-migration-"));
    const stateRoot = join(root, "backend-state");
    const legacyDirectory = join(stateRoot, "providers", "openai");
    const legacyPath = join(legacyDirectory, "accounts.json");
    mkdirSync(legacyDirectory, { recursive: true, mode: 0o700 });
    writeFileSync(legacyPath, JSON.stringify({
      schemaVersion: 1,
      desiredStateRevision: 4,
      catalogRevision: 4,
      accounts: [{
        id: "account-legacy",
        providerId: "openai",
        userLabel: "Legacy",
        status: "verificationRequired",
        addedAt: 1,
        credential: {
          accessToken: jwt({ exp: Math.floor(Date.now() / 1000) + 3600, chatgpt_account_id: "legacy" }),
          refreshToken: "refresh-legacy",
          chatgptAccountId: "legacy",
          expiresAt: Date.now() + 3_600_000,
        },
      }],
    }), { mode: 0o600 });

    const migrated = await runBackend(
      `${JSON.stringify({ type: "modelRouteRead", requestId: "read-migrated" })}\n`,
      stateRoot,
    );
    expect(migrated.lines[0]).toMatchObject({ desiredStateRevision: 4, catalogRevision: 4 });
    expect(migrated.lines[1]).toMatchObject({
      type: "modelRouteReadResult",
      desiredStateRevision: 4,
      catalogRevision: 4,
      data: [],
    });
    expect(existsSync(join(stateRoot, "model-plane.json"))).toBe(true);
    expect(existsSync(legacyPath)).toBe(false);
  });

  test("failed imports preserve revisions and do not reflect credential-shaped input", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-provider-account-failure-"));
    const stateRoot = join(root, "backend-state");
    const source = join(root, "invalid-auth.json");
    const canary = "credential-canary-must-not-be-reflected";
    writeFileSync(source, JSON.stringify({ tokens: { access_token: canary } }), { mode: 0o600 });

    const result = await runBackend(
      `${JSON.stringify({
        type: "providerAccountImport",
        requestId: "import-failed",
        authJsonPath: source,
        userLabel: "Invalid",
      })}\n${JSON.stringify({ type: "providerAccountList", requestId: "list-after-failure" })}\n`,
      stateRoot,
    );

    expect(result.lines[1]).toEqual({
      type: "operationError",
      requestId: "import-failed",
      code: "invalid_auth_document",
      message: "selected credential source is not a supported Codex login",
    });
    expect(result.lines[2]).toMatchObject({
      type: "providerAccountListResult",
      desiredStateRevision: 0,
      catalogRevision: 0,
      data: [],
    });
    expect(JSON.stringify(result.lines)).not.toContain(canary);
    expect(existsSync(stateRoot)).toBe(false);
  });

  test("does not create backend or legacy artifacts in an isolated home", async () => {
    const root = mkdtempSync(join(tmpdir(), "richcodex-headless-artifacts-"));
    const stateRoot = join(root, "backend-state");
    const backend = createHeadlessBackend({
      stateRoot,
      dataPlaneCapability: TEST_DATA_PLANE_CAPABILITY,
      env: {
        HOME: root,
        OPENCODEX_HOME: join(root, ".opencodex"),
        CODEX_HOME: join(root, ".codex"),
      },
    });
    await backend.run({
      stdin: inputOf(""),
      stdout: () => {},
    });
    expect(existsSync(stateRoot)).toBe(false);
    expect(readdirSync(root)).toEqual([]);
  });
});
