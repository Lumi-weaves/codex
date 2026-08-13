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
import type {
  ModelExecutionCandidate,
  ModelPlaneStore,
  ProviderAccountStatus,
  StoredOAuthCredential,
} from "../src/model-plane";

const TEST_DATA_PLANE_CAPABILITY = "richcodex-test-capability-0123456789abcdef";

function inputOf(text: string): AsyncIterable<Uint8Array> {
  return (async function* (): AsyncGenerator<Uint8Array> {
    yield new TextEncoder().encode(text);
  })();
}

async function runBackend(input: string, stateRoot: string): Promise<{ lines: unknown[]; result: unknown; stderr: string[] }> {
  const lines: unknown[] = [];
  const stderr: string[] = [];
  const backend = createHeadlessBackend({
    stateRoot,
    env: {},
    dataPlaneCapability: TEST_DATA_PLANE_CAPABILITY,
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
  credential: StoredOAuthCredential,
): ModelExecutionCandidate {
  return {
    modelTag: "my-fast-model",
    semanticModel: "gpt-5.6-luna",
    targetId,
    providerId: "openai",
    accountId,
    upstreamModelId: "gpt-5.6-luna",
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
    createModelRoute: () => { throw new Error("not used"); },
    retireModelRoute: () => { throw new Error("not used"); },
    resolveExecutionCandidates: modelTag => modelTag === "my-fast-model" ? candidates : [],
    replaceOAuthCredential: onReplace,
    markAccountStatus: onStatus,
  };
}

describe("RichCodex private model data plane", () => {
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
        return new Response('data: {"type":"response.completed"}\n\n', {
          status: 200,
          headers: { "content-type": "text/event-stream" },
        });
      }) as typeof fetch,
    });

    const unauthorized = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));
    expect(unauthorized.status).toBe(401);
    expect(calls).toHaveLength(0);

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        authorization: `Bearer ${TEST_DATA_PLANE_CAPABILITY}`,
        "content-type": "application/json",
        "x-codex-turn-state": "turn-state",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [{ role: "user" }] }),
    }));
    expect(response.status).toBe(200);
    expect(response.headers.get("x-richcodex-model-tag")).toBe("my-fast-model");
    expect(response.headers.get("x-richcodex-account-id")).toBe("account-1");
    expect(await response.text()).toContain("response.completed");
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe("https://chatgpt.com/backend-api/codex/responses");
    expect(new Headers(calls[0]!.init?.headers).get("authorization"))
      .toBe("Bearer private-access-token");
    expect(new Headers(calls[0]!.init?.headers).get("chatgpt-account-id"))
      .toBe("workspace-account");
    expect(JSON.parse(String(calls[0]!.init?.body))).toMatchObject({
      model: "gpt-5.6-luna",
      input: [{ role: "user" }],
    });
    expect(statuses).toEqual([["account-1", "ready"]]);
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
          headers: { "content-type": "text/event-stream" },
        });
      }) as typeof fetch,
    });

    const response = await plane.handle(new Request("http://127.0.0.1/v1/responses", {
      method: "POST",
      headers: {
        authorization: `Bearer ${TEST_DATA_PLANE_CAPABILITY}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({ model: "my-fast-model", input: [] }),
    }));
    expect(response.status).toBe(200);
    expect(response.headers.get("x-richcodex-account-id")).toBe("account-2");
    expect(response.headers.get("x-richcodex-route-attempt")).toBe("2");
    expect(responseAccounts).toEqual(["workspace-1", "workspace-2"]);
    expect(replacements).toEqual([{
      kind: "oauth",
      accessToken: "refreshed-access",
      refreshToken: "rotated-refresh",
      chatgptAccountId: "workspace-1",
      expiresAt: now + 3_600_000,
    }]);
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
      protocolVersion: 4,
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
