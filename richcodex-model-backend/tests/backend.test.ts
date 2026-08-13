import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readdirSync } from "node:fs";
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
      protocolVersion: 1,
      kernel: RICHCODEX_BACKEND_KERNEL,
      catalogRevision: 0,
      providers: [],
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
