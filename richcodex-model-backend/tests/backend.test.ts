import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
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

function inputOf(text: string): AsyncIterable<Uint8Array> {
  return (async function* (): AsyncGenerator<Uint8Array> {
    yield new TextEncoder().encode(text);
  })();
}

async function runBackend(input: string, stateRoot: string): Promise<{ lines: unknown[]; result: unknown; stderr: string[] }> {
  const lines: unknown[] = [];
  const stderr: string[] = [];
  const backend = createHeadlessBackend({ stateRoot, env: {} });
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
      protocolVersion: 2,
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
    const persisted = join(stateRoot, "providers", "openai", "accounts.json");
    expect(existsSync(persisted)).toBe(true);
    if (process.platform !== "win32") expect(statSync(persisted).mode & 0o777).toBe(0o600);
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
