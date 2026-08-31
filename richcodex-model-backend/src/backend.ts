import { randomUUID } from "node:crypto";
import { isAbsolute, resolve } from "node:path";
import { RICHCODEX_BACKEND_KERNEL, type RichCodexBackendKernel } from "./kernel-manifest";
import { createModelDataPlane } from "./data-plane";
import { createBrowserOAuthCoordinator } from "./browser-oauth";
import {
  createDeviceOAuthCoordinator,
  type DeviceOAuthCoordinator,
  type SafeProviderLogin,
} from "./device-oauth";
import {
  createModelPlaneStore,
  ModelPlaneError,
  type ModelPlaneStore,
  type ModelRouteMutationCode,
  type ProviderAccountImportCode,
  type AccountRemovalPreview,
  type SafeProviderAccount,
  type SafeModelRoute,
  type SafeProviderSummary,
} from "./model-plane";

/** Protocol 14 adds explicit local-frontend access-token projection. */
export const RICHCODEX_BACKEND_PROTOCOL_VERSION = 14 as const;

/**
 * The canonical state-root slot for the supervised backend.
 *
 * The backend deliberately has no implicit home.  The aliases below are kept
 * RichCodex-specific as well, so a caller can choose a less verbose spelling
 * without crossing into another product's state namespace.
 */
export const RICHCODEX_BACKEND_STATE_ROOT_ENV = "RICHCODEX_BACKEND_STATE_ROOT" as const;
export const RICHCODEX_BACKEND_DATA_PLANE_TOKEN_ENV = "RICHCODEX_BACKEND_DATA_PLANE_TOKEN" as const;

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
  | "instance_id_invalid"
  | "data_plane_capability_missing"
  | "data_plane_capability_invalid";

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
  /** Internal persistence seam for focused tests and future embeddings. */
  readonly modelPlaneStore?: ModelPlaneStore;
  /** Private bearer shared only with the supervising app-server process. */
  readonly dataPlaneCapability?: string;
  /** Internal deterministic seam for provider-login lifecycle tests. */
  readonly deviceOAuthCoordinator?: DeviceOAuthCoordinator;
  /** Internal deterministic seam for browser-login lifecycle tests. */
  readonly browserOAuthCoordinator?: DeviceOAuthCoordinator;
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
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly dataPlanePort: number;
  readonly providers: readonly SafeProviderSummary[];
  readonly models: readonly SafeModelRoute[];
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

export type HeadlessProviderAccountListResultMessage = {
  readonly type: "providerAccountListResult";
  readonly requestId: string;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly providers: readonly SafeProviderSummary[];
  readonly data: readonly SafeProviderAccount[];
  readonly nextCursor: string | null;
};

export type HeadlessProviderAccountImportResultMessage = {
  readonly type:
    | "providerAccountImportResult"
    | "providerAccountAddApiKeyResult"
    | "providerAccountAuthTokensInstallResult"
    | "providerAccountRenameResult"
    | "providerAccountReplaceApiKeyResult"
    | "providerAccountRemoveResult";
  readonly requestId: string;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly account: SafeProviderAccount;
};

export type HeadlessProviderAccountAuthTokenReadResultMessage = {
  readonly type: "providerAccountAuthTokenReadResult";
  readonly requestId: string;
  readonly accessToken: string;
};

export type HeadlessProviderAccountRemovalPreviewResultMessage = AccountRemovalPreview & {
  readonly type: "providerAccountRemovalPreviewResult";
  readonly requestId: string;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
};

export type HeadlessProviderAccountLoginResultMessage = SafeProviderLogin & {
  readonly type:
    | "providerAccountLoginStartResult"
    | "providerAccountLoginStatusResult"
    | "providerAccountLoginCancelResult";
  readonly requestId: string;
};

export type HeadlessModelRouteReadResultMessage = {
  readonly type: "modelRouteReadResult";
  readonly requestId: string;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly data: readonly SafeModelRoute[];
};

export type HeadlessModelRouteMutationResultMessage = {
  readonly type: "modelRouteCreateResult" | "modelRouteSetTargetsResult" | "modelRouteRetireResult";
  readonly requestId: string;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly route: SafeModelRoute;
};

export type HeadlessOperationErrorMessage = {
  readonly type: "operationError";
  readonly requestId: string;
  readonly code: ProviderAccountImportCode | ModelRouteMutationCode;
  readonly message: string;
};

type HeadlessInboundMessage =
  | { readonly type: "shutdown"; readonly requestId: string }
  | {
    readonly type: "providerAccountList";
    readonly requestId: string;
    readonly cursor: string | null;
    readonly limit: number | null;
  }
  | {
    readonly type: "providerAccountImport";
    readonly requestId: string;
    readonly authJsonPath: string;
    readonly userLabel: string;
  }
  | {
    readonly type: "providerAccountAddApiKey";
    readonly requestId: string;
    readonly providerId: string;
    readonly providerDisplayName: string;
    readonly apiBaseUrl: string;
    readonly apiKey: string;
    readonly userLabel: string;
  }
  | {
    readonly type: "providerAccountLoginStart";
    readonly requestId: string;
    readonly userLabel: string;
    readonly accountId: string | null;
    readonly mode: "browser" | "deviceCode";
  }
  | {
    readonly type: "providerAccountAuthTokensInstall";
    readonly requestId: string;
    readonly accessToken: string;
    readonly chatgptAccountId: string;
    readonly chatgptPlanType: string | null;
    readonly userLabel: string;
    readonly accountId: string | null;
  }
  | {
    readonly type: "providerAccountAuthTokenRead";
    readonly requestId: string;
    readonly accountId: string;
  }
  | {
    readonly type: "providerAccountRename";
    readonly requestId: string;
    readonly expectedRevision: number;
    readonly accountId: string;
    readonly userLabel: string;
  }
  | {
    readonly type: "providerAccountReplaceApiKey";
    readonly requestId: string;
    readonly expectedRevision: number;
    readonly accountId: string;
    readonly apiKey: string;
  }
  | {
    readonly type: "providerAccountRemovalPreview";
    readonly requestId: string;
    readonly accountId: string;
  }
  | {
    readonly type: "providerAccountRemove";
    readonly requestId: string;
    readonly expectedRevision: number;
    readonly accountId: string;
  }
  | {
    readonly type: "providerAccountLoginStatus";
    readonly requestId: string;
    readonly loginId: string;
  }
  | {
    readonly type: "providerAccountLoginCancel";
    readonly requestId: string;
    readonly loginId: string;
  }
  | { readonly type: "modelRouteRead"; readonly requestId: string }
  | {
    readonly type: "modelRouteCreate";
    readonly requestId: string;
    readonly expectedRevision: number;
    readonly modelTag: string;
    readonly displayName: string;
    readonly semanticModel: string;
    readonly providerId: string;
    readonly accountId: string;
    readonly upstreamModelId: string;
  }
  | {
    readonly type: "modelRouteRetire";
    readonly requestId: string;
    readonly expectedRevision: number;
    readonly modelTag: string;
  }
  | {
    readonly type: "modelRouteSetTargets";
    readonly requestId: string;
    readonly expectedRevision: number;
    readonly modelTag: string;
    readonly targets: readonly {
      readonly id?: string;
      readonly providerId: string;
      readonly accountId: string;
      readonly upstreamModelId: string;
    }[];
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
  | HeadlessProtocolErrorMessage
  | HeadlessProviderAccountListResultMessage
  | HeadlessProviderAccountImportResultMessage
  | HeadlessProviderAccountAuthTokenReadResultMessage
  | HeadlessProviderAccountRemovalPreviewResultMessage
  | HeadlessProviderAccountLoginResultMessage
  | HeadlessModelRouteReadResultMessage
  | HeadlessModelRouteMutationResultMessage
  | HeadlessOperationErrorMessage,
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

function readyMessage(
  instanceId: string,
  modelPlaneStore: ModelPlaneStore,
  dataPlanePort: number,
): HeadlessReadyMessage {
  const snapshot = modelPlaneStore.snapshot();
  return {
    type: "ready",
    protocolVersion: RICHCODEX_BACKEND_PROTOCOL_VERSION,
    instanceId,
    kernel: RICHCODEX_BACKEND_KERNEL,
    desiredStateRevision: snapshot.desiredStateRevision,
    catalogRevision: snapshot.catalogRevision,
    dataPlanePort,
    providers: snapshot.providers,
    models: snapshot.modelRoutes.filter(route => !route.retired),
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasExactlyKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = ownKeys(value).sort();
  const expected = [...keys].sort();
  return actual.length === expected.length && actual.every((key, index) => key === expected[index]);
}

function parseRequestId(value: unknown): string | null {
  return isBoundedOpaqueText(value, RICHCODEX_BACKEND_MAX_REQUEST_ID_BYTES) ? value : null;
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
  if (record.type === "shutdown") {
    const requestId = parseRequestId(record.requestId);
    if (!hasExactlyKeys(record, ["type", "requestId"]) || !requestId) {
      return { ok: false, error: protocolError("malformed_shutdown") };
    }
    return { ok: true, message: { type: "shutdown", requestId } };
  }
  if (record.type === "providerAccountList") {
    const requestId = parseRequestId(record.requestId);
    const cursor = record.cursor === undefined || record.cursor === null ? null : record.cursor;
    const limit = record.limit === undefined || record.limit === null ? null : record.limit;
    const keys = ownKeys(record);
    if (
      !requestId
      || keys.some(key => !["type", "requestId", "cursor", "limit"].includes(key))
      || typeof cursor !== "string" && cursor !== null
      || typeof cursor === "string" && !isBoundedOpaqueText(cursor, 64)
      || typeof limit !== "number" && limit !== null
      || (typeof limit === "number" && (!Number.isSafeInteger(limit) || limit < 1 || limit > 100))
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return { ok: true, message: { type: "providerAccountList", requestId, cursor, limit } };
  }
  if (record.type === "providerAccountImport") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "authJsonPath", "userLabel"])
      || !requestId
      || !isBoundedOpaqueText(record.authJsonPath, RICHCODEX_BACKEND_MAX_STATE_ROOT_BYTES)
      || !isAbsolute(record.authJsonPath)
      || !isBoundedOpaqueText(record.userLabel, 80)
      || record.userLabel.trim() !== record.userLabel
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return {
      ok: true,
      message: {
        type: "providerAccountImport",
        requestId,
        authJsonPath: record.authJsonPath,
        userLabel: record.userLabel,
      },
    };
  }
  if (record.type === "providerAccountAddApiKey") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, [
        "type",
        "requestId",
        "providerId",
        "providerDisplayName",
        "apiBaseUrl",
        "apiKey",
        "userLabel",
      ])
      || !requestId
      || !isProviderId(record.providerId)
      || !isBoundedOpaqueText(record.providerDisplayName, 80)
      || record.providerDisplayName.trim() !== record.providerDisplayName
      || !isApiBaseUrl(record.apiBaseUrl)
      || !isBoundedOpaqueText(record.apiKey, 64 * 1024)
      || record.apiKey.trim() !== record.apiKey
      || !isBoundedOpaqueText(record.userLabel, 80)
      || record.userLabel.trim() !== record.userLabel
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return {
      ok: true,
      message: {
        type: "providerAccountAddApiKey",
        requestId,
        providerId: record.providerId,
        providerDisplayName: record.providerDisplayName,
        apiBaseUrl: record.apiBaseUrl,
        apiKey: record.apiKey,
        userLabel: record.userLabel,
      },
    };
  }
  if (record.type === "providerAccountLoginStart") {
    const requestId = parseRequestId(record.requestId);
    const accountId = record.accountId === undefined || record.accountId === null
      ? null
      : record.accountId;
    if (
      ownKeys(record).some(key => !["type", "requestId", "userLabel", "accountId", "mode"].includes(key))
      || !requestId
      || !isBoundedOpaqueText(record.userLabel, 80)
      || record.userLabel.trim() !== record.userLabel
      || accountId !== null && !isBoundedOpaqueText(accountId, 80)
      || record.mode !== "browser" && record.mode !== "deviceCode"
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return {
      ok: true,
      message: {
        type: "providerAccountLoginStart",
        requestId,
        userLabel: record.userLabel,
        accountId,
        mode: record.mode,
      },
    };
  }
  if (record.type === "providerAccountAuthTokensInstall") {
    const requestId = parseRequestId(record.requestId);
    const accountId = record.accountId === undefined || record.accountId === null
      ? null
      : record.accountId;
    const planType = record.chatgptPlanType === undefined || record.chatgptPlanType === null
      ? null
      : record.chatgptPlanType;
    if (
      !hasExactlyKeys(record, [
        "type",
        "requestId",
        "accessToken",
        "chatgptAccountId",
        "chatgptPlanType",
        "userLabel",
        "accountId",
      ])
      || !requestId
      || !isBoundedOpaqueText(record.accessToken, 64 * 1024)
      || !isBoundedOpaqueText(record.chatgptAccountId, 512)
      || planType !== null && !isBoundedOpaqueText(planType, 128)
      || !isBoundedOpaqueText(record.userLabel, 80)
      || record.userLabel.trim() !== record.userLabel
      || accountId !== null && !isBoundedOpaqueText(accountId, 80)
    ) return { ok: false, error: protocolError("malformed_message") };
    return {
      ok: true,
      message: {
        type: "providerAccountAuthTokensInstall",
        requestId,
        accessToken: record.accessToken,
        chatgptAccountId: record.chatgptAccountId,
        chatgptPlanType: planType,
        userLabel: record.userLabel,
        accountId,
      },
    };
  }
  if (record.type === "providerAccountAuthTokenRead") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "accountId"])
      || !requestId
      || !isBoundedOpaqueText(record.accountId, 80)
    ) return { ok: false, error: protocolError("malformed_message") };
    return {
      ok: true,
      message: {
        type: "providerAccountAuthTokenRead",
        requestId,
        accountId: record.accountId,
      },
    };
  }
  if (record.type === "providerAccountReplaceApiKey") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "expectedRevision", "accountId", "apiKey"])
      || !requestId
      || !Number.isSafeInteger(record.expectedRevision)
      || (record.expectedRevision as number) < 0
      || !isBoundedOpaqueText(record.accountId, 80)
      || !isBoundedOpaqueText(record.apiKey, 64 * 1024)
      || record.apiKey.trim() !== record.apiKey
    ) return { ok: false, error: protocolError("malformed_message") };
    return {
      ok: true,
      message: {
        type: "providerAccountReplaceApiKey",
        requestId,
        expectedRevision: record.expectedRevision as number,
        accountId: record.accountId,
        apiKey: record.apiKey,
      },
    };
  }
  if (record.type === "providerAccountRename") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "expectedRevision", "accountId", "userLabel"])
      || !requestId
      || !Number.isSafeInteger(record.expectedRevision)
      || (record.expectedRevision as number) < 0
      || !isBoundedOpaqueText(record.accountId, 80)
      || !isBoundedOpaqueText(record.userLabel, 80)
      || record.userLabel.trim() !== record.userLabel
    ) return { ok: false, error: protocolError("malformed_message") };
    return {
      ok: true,
      message: {
        type: "providerAccountRename",
        requestId,
        expectedRevision: record.expectedRevision as number,
        accountId: record.accountId,
        userLabel: record.userLabel,
      },
    };
  }
  if (record.type === "providerAccountRemovalPreview") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "accountId"])
      || !requestId
      || !isBoundedOpaqueText(record.accountId, 80)
    ) return { ok: false, error: protocolError("malformed_message") };
    return {
      ok: true,
      message: { type: "providerAccountRemovalPreview", requestId, accountId: record.accountId },
    };
  }
  if (record.type === "providerAccountRemove") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "expectedRevision", "accountId"])
      || !requestId
      || !Number.isSafeInteger(record.expectedRevision)
      || (record.expectedRevision as number) < 0
      || !isBoundedOpaqueText(record.accountId, 80)
    ) return { ok: false, error: protocolError("malformed_message") };
    return {
      ok: true,
      message: {
        type: "providerAccountRemove",
        requestId,
        expectedRevision: record.expectedRevision as number,
        accountId: record.accountId,
      },
    };
  }
  if (
    record.type === "providerAccountLoginStatus"
    || record.type === "providerAccountLoginCancel"
  ) {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "loginId"])
      || !requestId
      || !isBoundedOpaqueText(record.loginId, 80)
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return {
      ok: true,
      message: { type: record.type, requestId, loginId: record.loginId },
    };
  }
  if (record.type === "modelRouteRead") {
    const requestId = parseRequestId(record.requestId);
    if (!hasExactlyKeys(record, ["type", "requestId"]) || !requestId) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return { ok: true, message: { type: "modelRouteRead", requestId } };
  }
  if (record.type === "modelRouteCreate") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, [
        "type",
        "requestId",
        "expectedRevision",
        "modelTag",
        "displayName",
        "semanticModel",
        "providerId",
        "accountId",
        "upstreamModelId",
      ])
      || !requestId
      || !Number.isSafeInteger(record.expectedRevision)
      || (record.expectedRevision as number) < 0
      || !isBoundedOpaqueText(record.modelTag, 80)
      || record.modelTag.trim() !== record.modelTag
      || !/^[a-z0-9][a-z0-9._/-]*$/.test(record.modelTag)
      || !isBoundedOpaqueText(record.displayName, 80)
      || record.displayName.trim() !== record.displayName
      || !isBoundedOpaqueText(record.semanticModel, 200)
      || record.semanticModel.trim() !== record.semanticModel
      || !isProviderId(record.providerId)
      || !isBoundedOpaqueText(record.accountId, 80)
      || !isBoundedOpaqueText(record.upstreamModelId, 512)
      || record.upstreamModelId.trim() !== record.upstreamModelId
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return {
      ok: true,
      message: {
        type: "modelRouteCreate",
        requestId,
        expectedRevision: record.expectedRevision as number,
        modelTag: record.modelTag,
        displayName: record.displayName,
        semanticModel: record.semanticModel,
        providerId: record.providerId,
        accountId: record.accountId,
        upstreamModelId: record.upstreamModelId,
      },
    };
  }
  if (record.type === "modelRouteSetTargets") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "expectedRevision", "modelTag", "targets"])
      || !requestId
      || !Number.isSafeInteger(record.expectedRevision)
      || (record.expectedRevision as number) < 0
      || !isBoundedOpaqueText(record.modelTag, 80)
      || record.modelTag.trim() !== record.modelTag
      || !Array.isArray(record.targets)
      || record.targets.length === 0
      || record.targets.length > 64
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    const targets = record.targets.flatMap(target => {
      if (
        !isRecord(target)
        || ownKeys(target).some(key => !["id", "providerId", "accountId", "upstreamModelId"].includes(key))
        || target.id !== undefined && target.id !== null && !isBoundedOpaqueText(target.id, 80)
        || !isProviderId(target.providerId)
        || !isBoundedOpaqueText(target.accountId, 80)
        || !isBoundedOpaqueText(target.upstreamModelId, 512)
        || target.upstreamModelId.trim() !== target.upstreamModelId
      ) return [];
      return [{
        ...(typeof target.id === "string" ? { id: target.id } : {}),
        providerId: target.providerId,
        accountId: target.accountId,
        upstreamModelId: target.upstreamModelId,
      }];
    });
    if (targets.length !== record.targets.length) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return {
      ok: true,
      message: {
        type: "modelRouteSetTargets",
        requestId,
        expectedRevision: record.expectedRevision as number,
        modelTag: record.modelTag,
        targets,
      },
    };
  }
  if (record.type === "modelRouteRetire") {
    const requestId = parseRequestId(record.requestId);
    if (
      !hasExactlyKeys(record, ["type", "requestId", "expectedRevision", "modelTag"])
      || !requestId
      || !Number.isSafeInteger(record.expectedRevision)
      || (record.expectedRevision as number) < 0
      || !isBoundedOpaqueText(record.modelTag, 80)
      || record.modelTag.trim() !== record.modelTag
    ) {
      return { ok: false, error: protocolError("malformed_message") };
    }
    return {
      ok: true,
      message: {
        type: "modelRouteRetire",
        requestId,
        expectedRevision: record.expectedRevision as number,
        modelTag: record.modelTag,
      },
    };
  }
  return { ok: false, error: protocolError("unknown_message_type") };
}

function isBoundedOpaqueText(value: unknown, maxBytes: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && Buffer.byteLength(value, "utf8") <= maxBytes
    && !CONTROL_CHARACTER.test(value);
}

function isProviderId(value: unknown): value is string {
  return isBoundedOpaqueText(value, 64)
    && value.trim() === value
    && /^[a-z0-9][a-z0-9._-]*$/.test(value);
}

function isApiBaseUrl(value: unknown): value is string {
  if (!isBoundedOpaqueText(value, 2048) || value.trim() !== value) return false;
  try {
    const url = new URL(value);
    const loopback = url.hostname === "localhost"
      || url.hostname === "127.0.0.1"
      || url.hostname === "[::1]";
    return (url.protocol === "https:" || url.protocol === "http:" && loopback)
      && url.username === ""
      && url.password === ""
      && url.search === ""
      && url.hash === ""
      && url.pathname !== "/"
      && !url.pathname.endsWith("/");
  } catch {
    return false;
  }
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

function operationErrorForCode(
  requestId: string,
  code: ProviderAccountImportCode | ModelRouteMutationCode,
): HeadlessOperationErrorMessage {
  const messages: Record<ProviderAccountImportCode | ModelRouteMutationCode, string> = {
    source_unavailable: "selected credential source is unavailable",
    source_too_large: "selected credential source exceeds its limit",
    invalid_auth_document: "selected credential source is not a supported Codex login",
    credential_expired: "selected Codex login has expired",
    account_already_exists: "this provider account is already configured",
    account_limit_reached: "provider account limit reached",
    account_not_found: "provider account does not exist",
    account_in_use: "provider account is still referenced by model targets",
    credential_kind_mismatch: "provider account credential kind does not match",
    account_identity_mismatch: "reauthentication returned a different upstream account",
    invalid_provider: "provider configuration is invalid",
    provider_conflict: "provider ID is already configured differently",
    invalid_api_key: "API key is invalid",
    login_unavailable: "provider login is unavailable",
    login_limit_reached: "provider login limit reached",
    login_not_found: "provider login does not exist",
    store_unavailable: "RichCodex model plane store is unavailable",
    invalid_request: "model plane request is invalid",
    revision_conflict: "model plane revision does not match",
    model_tag_exists: "model tag already exists",
    model_tag_not_found: "model tag does not exist",
    account_unavailable: "selected provider account is unavailable",
  };
  return { type: "operationError", requestId, code, message: messages[code] };
}

function operationError(requestId: string, error: unknown): HeadlessOperationErrorMessage {
  return operationErrorForCode(
    requestId,
    error instanceof ModelPlaneError ? error.code : "store_unavailable",
  );
}

function pageAccounts(
  accounts: readonly SafeProviderAccount[],
  cursor: string | null,
  requestedLimit: number | null,
): { readonly data: readonly SafeProviderAccount[]; readonly nextCursor: string | null } | null {
  const offset = cursor === null ? 0 : Number(cursor);
  if (!Number.isSafeInteger(offset) || offset < 0 || offset > accounts.length) return null;
  const limit = requestedLimit ?? 50;
  const data = accounts.slice(offset, offset + limit);
  const nextOffset = offset + data.length;
  return { data, nextCursor: nextOffset < accounts.length ? String(nextOffset) : null };
}

/** Create the RichCodex-owned composition root. */
export function createHeadlessBackend(options: HeadlessBackendOptions = {}): HeadlessBackend {
  const stateRoot = resolveBackendStateRoot(options);
  const modelPlaneStore = options.modelPlaneStore ?? createModelPlaneStore(stateRoot);
  const deviceOAuth = options.deviceOAuthCoordinator ?? createDeviceOAuthCoordinator({
    modelPlaneStore,
    env: options.env,
  });
  const browserOAuth = options.browserOAuthCoordinator ?? createBrowserOAuthCoordinator({
    modelPlaneStore,
    env: options.env,
  });
  const loginModes = new Map<string, "browser" | "deviceCode">();
  const dataPlaneCapability = options.dataPlaneCapability
    ?? options.env?.[RICHCODEX_BACKEND_DATA_PLANE_TOKEN_ENV];
  if (!dataPlaneCapability) {
    throw new HeadlessBackendConfigurationError("data_plane_capability_missing");
  }
  if (
    dataPlaneCapability.length < 32
    || dataPlaneCapability.length > 512
    || !/^[A-Za-z0-9._~-]+$/.test(dataPlaneCapability)
  ) {
    throw new HeadlessBackendConfigurationError("data_plane_capability_invalid");
  }
  const instanceId = options.createInstanceId?.() ?? defaultInstanceId();
  if (!isBoundedOpaqueText(instanceId, RICHCODEX_BACKEND_MAX_MESSAGE_BYTES)) {
    throw new HeadlessBackendConfigurationError("instance_id_invalid");
  }

  return {
    stateRoot,
    instanceId,
    async run(io): Promise<HeadlessBackendRunResult> {
      const dataPlane = createModelDataPlane({
        capability: dataPlaneCapability,
        modelPlaneStore,
        responsesWebSocketProxy: options.env?.HTTPS_PROXY
          ?? options.env?.https_proxy
          ?? options.env?.ALL_PROXY
          ?? options.env?.all_proxy,
      }).start();
      try {
        let readyWritten = false;
        const write = async (
          message: HeadlessReadyMessage
            | HeadlessShutdownCompleteMessage
            | HeadlessProtocolErrorMessage
            | HeadlessProviderAccountListResultMessage
            | HeadlessProviderAccountImportResultMessage
            | HeadlessProviderAccountAuthTokenReadResultMessage
            | HeadlessProviderAccountRemovalPreviewResultMessage
            | HeadlessProviderAccountLoginResultMessage
            | HeadlessModelRouteReadResultMessage
            | HeadlessModelRouteMutationResultMessage
            | HeadlessOperationErrorMessage,
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

        if (!await write(readyMessage(instanceId, modelPlaneStore, dataPlane.port))) {
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

            if (parsed.message.type === "shutdown") {
              if (!await write({ type: "shutdownComplete", requestId: parsed.message.requestId })) {
                return { exitCode: 1, reason: "output_error" };
              }
              return { exitCode: 0, reason: "shutdown" };
            }
            if (parsed.message.type === "providerAccountList") {
              const snapshot = modelPlaneStore.snapshot();
              const page = pageAccounts(snapshot.accounts, parsed.message.cursor, parsed.message.limit);
              const response = page === null
                ? operationErrorForCode(parsed.message.requestId, "invalid_request")
                : {
                    type: "providerAccountListResult" as const,
                    requestId: parsed.message.requestId,
                    desiredStateRevision: snapshot.desiredStateRevision,
                    catalogRevision: snapshot.catalogRevision,
                    providers: snapshot.providers,
                    data: page.data,
                    nextCursor: page.nextCursor,
                  };
              if (!await write(response)) return { exitCode: 1, reason: "output_error" };
              continue;
            }
            if (parsed.message.type === "modelRouteRead") {
              const snapshot = modelPlaneStore.snapshot();
              if (!await write({
                type: "modelRouteReadResult",
                requestId: parsed.message.requestId,
                desiredStateRevision: snapshot.desiredStateRevision,
                catalogRevision: snapshot.catalogRevision,
                data: snapshot.modelRoutes,
              })) return { exitCode: 1, reason: "output_error" };
              continue;
            }
            try {
              if (parsed.message.type === "providerAccountImport") {
                const account = modelPlaneStore.importCodexAuthJson(
                  parsed.message.authJsonPath,
                  parsed.message.userLabel,
                );
                const snapshot = modelPlaneStore.snapshot();
                if (!await write({
                  type: "providerAccountImportResult",
                  requestId: parsed.message.requestId,
                  desiredStateRevision: snapshot.desiredStateRevision,
                  catalogRevision: snapshot.catalogRevision,
                  account,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountAddApiKey") {
                const account = modelPlaneStore.addApiKeyAccount(parsed.message);
                const snapshot = modelPlaneStore.snapshot();
                if (!await write({
                  type: "providerAccountAddApiKeyResult",
                  requestId: parsed.message.requestId,
                  desiredStateRevision: snapshot.desiredStateRevision,
                  catalogRevision: snapshot.catalogRevision,
                  account,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountAuthTokensInstall") {
                const account = modelPlaneStore.installClientAuthTokens(parsed.message);
                const snapshot = modelPlaneStore.snapshot();
                if (!await write({
                  type: "providerAccountAuthTokensInstallResult",
                  requestId: parsed.message.requestId,
                  desiredStateRevision: snapshot.desiredStateRevision,
                  catalogRevision: snapshot.catalogRevision,
                  account,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountAuthTokenRead") {
                const accessToken = modelPlaneStore.readOAuthAccessToken(
                  parsed.message.accountId,
                );
                if (!await write({
                  type: "providerAccountAuthTokenReadResult",
                  requestId: parsed.message.requestId,
                  accessToken,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountLoginStart") {
                const coordinator = parsed.message.mode === "browser" ? browserOAuth : deviceOAuth;
                const login = await coordinator.start(
                  parsed.message.userLabel,
                  parsed.message.accountId ?? undefined,
                );
                loginModes.set(login.loginId, parsed.message.mode);
                if (!await write({
                  type: "providerAccountLoginStartResult",
                  requestId: parsed.message.requestId,
                  ...login,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountReplaceApiKey") {
                const account = modelPlaneStore.replaceApiKeyCredential(
                  parsed.message.accountId,
                  parsed.message.expectedRevision,
                  parsed.message.apiKey,
                );
                const snapshot = modelPlaneStore.snapshot();
                if (!await write({
                  type: "providerAccountReplaceApiKeyResult",
                  requestId: parsed.message.requestId,
                  desiredStateRevision: snapshot.desiredStateRevision,
                  catalogRevision: snapshot.catalogRevision,
                  account,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountRename") {
                const account = modelPlaneStore.renameAccount(
                  parsed.message.accountId,
                  parsed.message.expectedRevision,
                  parsed.message.userLabel,
                );
                const snapshot = modelPlaneStore.snapshot();
                if (!await write({
                  type: "providerAccountRenameResult",
                  requestId: parsed.message.requestId,
                  desiredStateRevision: snapshot.desiredStateRevision,
                  catalogRevision: snapshot.catalogRevision,
                  account,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountRemovalPreview") {
                const preview = modelPlaneStore.previewAccountRemoval(parsed.message.accountId);
                const snapshot = modelPlaneStore.snapshot();
                if (!await write({
                  type: "providerAccountRemovalPreviewResult",
                  requestId: parsed.message.requestId,
                  desiredStateRevision: snapshot.desiredStateRevision,
                  catalogRevision: snapshot.catalogRevision,
                  ...preview,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountRemove") {
                const account = modelPlaneStore.removeAccount(
                  parsed.message.accountId,
                  parsed.message.expectedRevision,
                );
                const snapshot = modelPlaneStore.snapshot();
                if (!await write({
                  type: "providerAccountRemoveResult",
                  requestId: parsed.message.requestId,
                  desiredStateRevision: snapshot.desiredStateRevision,
                  catalogRevision: snapshot.catalogRevision,
                  account,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountLoginStatus") {
                const mode = loginModes.get(parsed.message.loginId);
                if (!mode) throw new ModelPlaneError("login_not_found");
                const login = (mode === "browser" ? browserOAuth : deviceOAuth)
                  .status(parsed.message.loginId);
                if (!await write({
                  type: "providerAccountLoginStatusResult",
                  requestId: parsed.message.requestId,
                  ...login,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              if (parsed.message.type === "providerAccountLoginCancel") {
                const mode = loginModes.get(parsed.message.loginId);
                if (!mode) throw new ModelPlaneError("login_not_found");
                const login = (mode === "browser" ? browserOAuth : deviceOAuth)
                  .cancel(parsed.message.loginId);
                if (!await write({
                  type: "providerAccountLoginCancelResult",
                  requestId: parsed.message.requestId,
                  ...login,
                })) return { exitCode: 1, reason: "output_error" };
                continue;
              }
              const route = parsed.message.type === "modelRouteCreate"
                ? modelPlaneStore.createModelRoute(parsed.message)
                : parsed.message.type === "modelRouteSetTargets"
                  ? modelPlaneStore.setModelRouteTargets(parsed.message)
                  : modelPlaneStore.retireModelRoute(
                      parsed.message.modelTag,
                      parsed.message.expectedRevision,
                    );
              const snapshot = modelPlaneStore.snapshot();
              if (!await write({
                type: parsed.message.type === "modelRouteCreate"
                  ? "modelRouteCreateResult"
                  : parsed.message.type === "modelRouteSetTargets"
                    ? "modelRouteSetTargetsResult"
                    : "modelRouteRetireResult",
                requestId: parsed.message.requestId,
                desiredStateRevision: snapshot.desiredStateRevision,
                catalogRevision: snapshot.catalogRevision,
                route,
              })) return { exitCode: 1, reason: "output_error" };
            } catch (error) {
              if (!await write(operationError(parsed.message.requestId, error))) {
                return { exitCode: 1, reason: "output_error" };
              }
            }
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
      } finally {
        deviceOAuth.shutdown();
        browserOAuth.shutdown();
        await dataPlane.stop();
      }
    },
  };
}
