import { randomUUID } from "node:crypto";
import { isAbsolute, resolve } from "node:path";
import { RICHCODEX_BACKEND_KERNEL, type RichCodexBackendKernel } from "./kernel-manifest";

/** The first private RichCodex/backend protocol revision. */
export const RICHCODEX_BACKEND_PROTOCOL_VERSION = 1 as const;

/**
 * The canonical state-root slot for the supervised backend.
 *
 * The backend deliberately has no implicit home.  The aliases below are kept
 * RichCodex-specific as well, so a caller can choose a less verbose spelling
 * without crossing into another product's state namespace.
 */
export const RICHCODEX_BACKEND_STATE_ROOT_ENV = "RICHCODEX_BACKEND_STATE_ROOT" as const;

/** Maximum UTF-8 bytes in one newline-delimited input message. */
export const RICHCODEX_BACKEND_MAX_MESSAGE_BYTES = 64 * 1024;

/** Maximum UTF-8 bytes in an explicit state-root argument or environment value. */
export const RICHCODEX_BACKEND_MAX_STATE_ROOT_BYTES = 4 * 1024;

/** Maximum UTF-8 bytes in a shutdown correlation id. */
export const RICHCODEX_BACKEND_MAX_REQUEST_ID_BYTES = 256;

export { RICHCODEX_BACKEND_KERNEL } from "./kernel-manifest";
export type { RichCodexBackendKernel } from "./kernel-manifest";

const CONTROL_CHARACTER = /[\u0000-\u001f\u007f]/;

type BackendEnvironment = Readonly<Record<string, string | undefined>>;

export type HeadlessBackendConfigurationCode =
  | "state_root_missing"
  | "state_root_invalid"
  | "state_root_not_absolute"
  | "state_root_too_large"
  | "instance_id_invalid";

/** A configuration failure whose message never includes caller-provided data. */
export class HeadlessBackendConfigurationError extends Error {
  readonly code: HeadlessBackendConfigurationCode;

  constructor(code: HeadlessBackendConfigurationCode) {
    super(code);
    this.name = "HeadlessBackendConfigurationError";
    this.code = code;
  }
}

export interface HeadlessBackendOptions {
  /** Explicit state root wins over all supported environment slots. */
  readonly stateRoot?: string;
  /** Injected for tests and embedding; production callers may omit it. */
  readonly env?: BackendEnvironment;
  /** Internal deterministic seam; production uses a fresh random UUID. */
  readonly createInstanceId?: () => string;
}

export interface HeadlessBackendRunIo {
  readonly stdin: HeadlessBackendInput;
  /** Receives one JSON line without its trailing newline. */
  readonly stdout: HeadlessBackendLineSink;
  /** Optional diagnostics sink. Protocol data is never sent here. */
  readonly stderr?: HeadlessBackendLineSink;
}

export type HeadlessBackendLineSink = (line: string) => void | Promise<void>;

export type HeadlessBackendInput =
  | AsyncIterable<Uint8Array>
  | ReadableStream<Uint8Array>;

export interface HeadlessBackendRunResult {
  readonly exitCode: 0 | 1;
  readonly reason: "shutdown" | "eof" | "input_error" | "output_error";
}

export interface HeadlessBackend {
  readonly stateRoot: string;
  readonly instanceId: string;
  run(io: HeadlessBackendRunIo): Promise<HeadlessBackendRunResult>;
}

export interface HeadlessBackendLaunchOptions {
  readonly stateRoot?: string;
}

export type HeadlessReadyMessage = {
  readonly type: "ready";
  readonly protocolVersion: typeof RICHCODEX_BACKEND_PROTOCOL_VERSION;
  readonly instanceId: string;
  readonly kernel: RichCodexBackendKernel;
  readonly catalogRevision: 0;
  readonly providers: readonly [];
  readonly models: readonly [];
};

export type HeadlessShutdownCompleteMessage = {
  readonly type: "shutdownComplete";
  readonly requestId: string;
};

export type HeadlessProtocolErrorCode =
  | "message_too_large"
  | "invalid_encoding"
  | "malformed_message"
  | "unknown_message_type"
  | "malformed_shutdown";

export type HeadlessProtocolErrorMessage = {
  readonly type: "protocolError";
  readonly code: HeadlessProtocolErrorCode;
  readonly message: string;
};

type HeadlessInboundMessage = {
  readonly type: "shutdown";
  readonly requestId: string;
};

type EncodedInputLine =
  | { readonly oversized: true }
  | { readonly oversized: false; readonly bytes: Uint8Array };

/**
 * Resolve the only state-root sources this composition root understands.
 *
 * This function is intentionally filesystem-free.  The root is normalized for
 * stable ownership comparisons, but it is not created, read, or canonicalized
 * through the filesystem.
 */
export function resolveBackendStateRoot(
  explicitOrOptions?: string | { readonly stateRoot?: string; readonly env?: BackendEnvironment },
  suppliedEnv?: BackendEnvironment,
): string {
  const explicit = typeof explicitOrOptions === "string"
    ? explicitOrOptions
    : explicitOrOptions?.stateRoot;
  const env = typeof explicitOrOptions === "string"
    ? suppliedEnv ?? process.env
    : explicitOrOptions?.env ?? suppliedEnv ?? process.env;
  const candidate = explicit !== undefined
    ? explicit
    : firstEnvironmentValue(env);

  if (candidate === undefined || candidate.length === 0) {
    throw new HeadlessBackendConfigurationError("state_root_missing");
  }
  return normalizeBackendStateRoot(candidate);
}

function firstEnvironmentValue(env: BackendEnvironment): string | undefined {
  const value = env[RICHCODEX_BACKEND_STATE_ROOT_ENV];
  return value !== undefined && value.length > 0 ? value : undefined;
}

function normalizeBackendStateRoot(candidate: string): string {
  if (typeof candidate !== "string" || candidate.length === 0) {
    throw new HeadlessBackendConfigurationError("state_root_missing");
  }
  if (Buffer.byteLength(candidate, "utf8") > RICHCODEX_BACKEND_MAX_STATE_ROOT_BYTES) {
    throw new HeadlessBackendConfigurationError("state_root_too_large");
  }
  if (CONTROL_CHARACTER.test(candidate) || candidate.includes("\u0000")) {
    throw new HeadlessBackendConfigurationError("state_root_invalid");
  }
  if (!isAbsolute(candidate)) {
    throw new HeadlessBackendConfigurationError("state_root_not_absolute");
  }

  return resolve(candidate);
}

/** Parse the small, explicit command-line surface owned by this entry point. */
export function parseHeadlessBackendArgs(args: readonly string[]): HeadlessBackendLaunchOptions {
  let stateRoot: string | undefined;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--state-root" || arg === "--backend-state-root") {
      if (stateRoot !== undefined || index + 1 >= args.length) {
        throw new HeadlessBackendConfigurationError("state_root_invalid");
      }
      stateRoot = args[index + 1];
      index += 1;
      continue;
    }
    if (arg.startsWith("--state-root=") || arg.startsWith("--backend-state-root=")) {
      if (stateRoot !== undefined) {
        throw new HeadlessBackendConfigurationError("state_root_invalid");
      }
      stateRoot = arg.slice(arg.indexOf("=") + 1);
      continue;
    }
    if (stateRoot === undefined && !arg.startsWith("-") && args.length === 1) {
      stateRoot = arg;
      continue;
    }
    throw new HeadlessBackendConfigurationError("state_root_invalid");
  }
  return { stateRoot };
}

function defaultInstanceId(): string {
  return randomUUID();
}

function encodeMessage(message:
  | HeadlessReadyMessage
  | HeadlessShutdownCompleteMessage
  | HeadlessProtocolErrorMessage,
): string {
  const line = JSON.stringify(message);
  if (Buffer.byteLength(line, "utf8") > RICHCODEX_BACKEND_MAX_MESSAGE_BYTES) {
    // This can only happen if an embedding supplies an invalid instance-id
    // factory.  It is kept as a local invariant rather than emitting an
    // unbounded diagnostic.
    throw new Error("backend protocol output exceeded limit");
  }
  return line;
}

function readyMessage(instanceId: string): HeadlessReadyMessage {
  return {
    type: "ready",
    protocolVersion: RICHCODEX_BACKEND_PROTOCOL_VERSION,
    instanceId,
    kernel: RICHCODEX_BACKEND_KERNEL,
    catalogRevision: 0,
    providers: [],
    models: [],
  };
}

function protocolError(code: HeadlessProtocolErrorCode): HeadlessProtocolErrorMessage {
  const messages: Record<HeadlessProtocolErrorCode, string> = {
    message_too_large: "message exceeds protocol limit",
    invalid_encoding: "message is not valid UTF-8",
    malformed_message: "message is not a valid protocol object",
    unknown_message_type: "message type is not supported",
    malformed_shutdown: "shutdown request is invalid",
  };
  return { type: "protocolError", code, message: messages[code] };
}

function ownKeys(value: Record<string, unknown>): string[] {
  return Object.keys(value);
}

function parseInboundMessage(text: string):
  | { readonly ok: true; readonly message: HeadlessInboundMessage }
  | { readonly ok: false; readonly error: HeadlessProtocolErrorMessage } {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { ok: false, error: protocolError("malformed_message") };
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return { ok: false, error: protocolError("malformed_message") };
  }
  const record = parsed as Record<string, unknown>;
  if (typeof record.type !== "string") {
    return { ok: false, error: protocolError("malformed_message") };
  }
  if (record.type !== "shutdown") {
    return { ok: false, error: protocolError("unknown_message_type") };
  }
  if (ownKeys(record).length !== 2 || typeof record.requestId !== "string") {
    return { ok: false, error: protocolError("malformed_shutdown") };
  }

  const requestId = record.requestId;
  if (
    !isBoundedOpaqueText(requestId, RICHCODEX_BACKEND_MAX_REQUEST_ID_BYTES)
  ) {
    return { ok: false, error: protocolError("malformed_shutdown") };
  }
  return { ok: true, message: { type: "shutdown", requestId } };
}

function isBoundedOpaqueText(value: unknown, maxBytes: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && Buffer.byteLength(value, "utf8") <= maxBytes
    && !CONTROL_CHARACTER.test(value);
}

async function* streamChunks(input: HeadlessBackendInput): AsyncGenerator<Uint8Array> {
  if (typeof (input as AsyncIterable<Uint8Array>)[Symbol.asyncIterator] === "function") {
    for await (const chunk of input as AsyncIterable<Uint8Array>) {
      yield chunk;
    }
    return;
  }

  const reader = (input as ReadableStream<Uint8Array>).getReader();
  try {
    for (;;) {
      const result = await reader.read();
      if (result.done) return;
      yield result.value;
    }
  } finally {
    reader.releaseLock();
  }
}

/** Decode input one bounded newline-delimited message at a time. */
async function* boundedLines(input: HeadlessBackendInput): AsyncGenerator<EncodedInputLine> {
  const line = new Uint8Array(RICHCODEX_BACKEND_MAX_MESSAGE_BYTES);
  let length = 0;
  let oversized = false;

  for await (const chunk of streamChunks(input)) {
    if (!(chunk instanceof Uint8Array)) {
      yield { oversized: false, bytes: new Uint8Array() };
      return;
    }
    for (const byte of chunk) {
      if (byte === 0x0a) {
        yield oversized
          ? { oversized: true }
          : { oversized: false, bytes: line.slice(0, length) };
        length = 0;
        oversized = false;
        continue;
      }
      if (oversized) continue;
      if (length >= line.length) {
        oversized = true;
        continue;
      }
      line[length] = byte;
      length += 1;
    }
  }

  if (length > 0 || oversized) {
    yield oversized
      ? { oversized: true }
      : { oversized: false, bytes: line.slice(0, length) };
  }
}

function decodeInputLine(bytes: Uint8Array): string | null {
  try {
    const decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return decoded.endsWith("\r") ? decoded.slice(0, -1) : decoded;
  } catch {
    return null;
  }
}

function defaultStderrMessage(error: unknown): string {
  return error instanceof HeadlessBackendConfigurationError
    ? "backend startup configuration is invalid"
    : "backend process failed";
}

/** Create the filesystem-free RichCodex composition root. */
export function createHeadlessBackend(options: HeadlessBackendOptions = {}): HeadlessBackend {
  const stateRoot = resolveBackendStateRoot(options);
  const instanceId = options.createInstanceId?.() ?? defaultInstanceId();
  if (!isBoundedOpaqueText(instanceId, RICHCODEX_BACKEND_MAX_MESSAGE_BYTES)) {
    throw new HeadlessBackendConfigurationError("instance_id_invalid");
  }

  return {
    stateRoot,
    instanceId,
    async run(io): Promise<HeadlessBackendRunResult> {
      let readyWritten = false;
      const write = async (
        message: HeadlessReadyMessage | HeadlessShutdownCompleteMessage | HeadlessProtocolErrorMessage,
      ): Promise<boolean> => {
        try {
          await io.stdout(encodeMessage(message));
          return true;
        } catch (error) {
          try {
            await io.stderr?.(defaultStderrMessage(error));
          } catch {
            // A diagnostics sink cannot change the protocol result.
          }
          return false;
        }
      };

      if (!await write(readyMessage(instanceId))) {
        return { exitCode: 1, reason: "output_error" };
      }
      readyWritten = true;

      try {
        for await (const encoded of boundedLines(io.stdin)) {
          if (!readyWritten) break;
          if (encoded.oversized) {
            if (!await write(protocolError("message_too_large"))) {
              return { exitCode: 1, reason: "output_error" };
            }
            continue;
          }

          const text = decodeInputLine(encoded.bytes);
          if (text === null) {
            if (!await write(protocolError("invalid_encoding"))) {
              return { exitCode: 1, reason: "output_error" };
            }
            continue;
          }

          const parsed = parseInboundMessage(text);
          if (!parsed.ok) {
            if (!await write(parsed.error)) {
              return { exitCode: 1, reason: "output_error" };
            }
            continue;
          }

          if (!await write({ type: "shutdownComplete", requestId: parsed.message.requestId })) {
            return { exitCode: 1, reason: "output_error" };
          }
          return { exitCode: 0, reason: "shutdown" };
        }
      } catch (error) {
        try {
          await io.stderr?.(defaultStderrMessage(error));
        } catch {
          // Diagnostics are best effort and must not expose stream details.
        }
        return { exitCode: 1, reason: "input_error" };
      }
      return { exitCode: 0, reason: "eof" };
    },
  };
}
