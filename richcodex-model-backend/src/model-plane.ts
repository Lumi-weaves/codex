import { randomBytes, randomUUID } from "node:crypto";
import {
  chmodSync,
  closeSync,
  constants,
  existsSync,
  fstatSync,
  fsyncSync,
  mkdirSync,
  openSync,
  readSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";

export const OPENAI_PROVIDER_ID = "openai" as const;
export const OPENAI_PROVIDER_DISPLAY_NAME = "OpenAI" as const;
export const PROVIDER_ACCOUNT_STORE_MAX_BYTES = 1024 * 1024;
export const PROVIDER_ACCOUNT_MAX_ROWS = 64;
export const MODEL_ROUTE_MAX_ROWS = 256;

const MODEL_PLANE_SCHEMA_VERSION = 1 as const;
const LEGACY_ACCOUNT_SCHEMA_VERSION = 1 as const;
const SAFE_TEXT_CONTROL = /[\u0000-\u001f\u007f]/;

export type ProviderAccountStatus = "ready" | "verificationRequired" | "reauthenticationRequired";

export interface SafeProviderAccount {
  readonly id: string;
  readonly providerId: typeof OPENAI_PROVIDER_ID;
  readonly userLabel: string;
  readonly credentialKind: "oauth" | "apiKey";
  readonly status: ProviderAccountStatus;
  readonly addedAt: number;
}

export interface SafeProviderSummary {
  readonly id: typeof OPENAI_PROVIDER_ID;
  readonly displayName: typeof OPENAI_PROVIDER_DISPLAY_NAME;
  readonly accountCount: number;
  readonly status: "ready" | "needsAccount";
}

export interface StoredOAuthCredential {
  readonly kind: "oauth";
  readonly accessToken: string;
  readonly refreshToken: string;
  readonly chatgptAccountId: string;
  readonly expiresAt: number;
}

export interface StoredApiKeyCredential {
  readonly kind: "apiKey";
  readonly apiKey: string;
}

export type StoredProviderCredential = StoredOAuthCredential | StoredApiKeyCredential;

interface StoredProviderAccount extends SafeProviderAccount {
  readonly credential: StoredProviderCredential;
}

interface StoredModelTarget {
  readonly id: string;
  readonly providerId: typeof OPENAI_PROVIDER_ID;
  readonly accountId: string;
  readonly upstreamModelId: string;
  readonly priority: number;
}

interface StoredModelTag {
  readonly id: string;
  readonly semanticModel: string;
  readonly targets: readonly StoredModelTarget[];
}

interface StoredDisplayEntry {
  readonly modelTag: string;
  readonly displayName: string;
  readonly retired: boolean;
  readonly addedAt: number;
}

interface ModelPlaneDocument {
  readonly schemaVersion: typeof MODEL_PLANE_SCHEMA_VERSION;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly accounts: readonly StoredProviderAccount[];
  readonly modelTags: readonly StoredModelTag[];
  readonly displayEntries: readonly StoredDisplayEntry[];
}

interface LegacyProviderAccountDocument {
  readonly schemaVersion: typeof LEGACY_ACCOUNT_SCHEMA_VERSION;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly accounts: readonly StoredProviderAccount[];
}

export type ProviderAccountImportCode =
  | "source_unavailable"
  | "source_too_large"
  | "invalid_auth_document"
  | "credential_expired"
  | "account_already_exists"
  | "account_limit_reached"
  | "invalid_api_key"
  | "login_unavailable"
  | "login_limit_reached"
  | "login_not_found"
  | "store_unavailable";

export type ModelRouteMutationCode =
  | "invalid_request"
  | "revision_conflict"
  | "model_tag_exists"
  | "model_tag_not_found"
  | "account_unavailable"
  | "store_unavailable";

export class ModelPlaneError extends Error {
  readonly code: ProviderAccountImportCode | ModelRouteMutationCode;

  constructor(code: ProviderAccountImportCode | ModelRouteMutationCode) {
    super(code);
    this.name = "ModelPlaneError";
    this.code = code;
  }
}

export interface ProviderAccountSnapshot {
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly providers: readonly SafeProviderSummary[];
  readonly accounts: readonly SafeProviderAccount[];
  readonly modelRoutes: readonly SafeModelRoute[];
}

export type SafeModelTargetStatus = "unverified" | "reauthenticationRequired";

export interface SafeModelTarget {
  readonly id: string;
  readonly providerId: typeof OPENAI_PROVIDER_ID;
  readonly accountId: string;
  readonly upstreamModelId: string;
  readonly priority: number;
  readonly status: SafeModelTargetStatus;
}

export interface SafeModelRoute {
  readonly modelTag: string;
  readonly displayName: string;
  readonly retired: boolean;
  readonly semanticModel: string;
  readonly targets: readonly SafeModelTarget[];
}

export interface CreateModelRouteInput {
  readonly expectedRevision: number;
  readonly modelTag: string;
  readonly displayName: string;
  readonly semanticModel: string;
  readonly providerId: typeof OPENAI_PROVIDER_ID;
  readonly accountId: string;
  readonly upstreamModelId: string;
}

export interface SetModelRouteTargetInput {
  readonly id?: string;
  readonly providerId: typeof OPENAI_PROVIDER_ID;
  readonly accountId: string;
  readonly upstreamModelId: string;
}

export interface SetModelRouteTargetsInput {
  readonly expectedRevision: number;
  readonly modelTag: string;
  readonly targets: readonly SetModelRouteTargetInput[];
}

export interface ModelPlaneStore {
  snapshot(): ProviderAccountSnapshot;
  importCodexAuthJson(authJsonPath: string, userLabel: string): SafeProviderAccount;
  addOAuthAccount(credential: StoredOAuthCredential, userLabel: string): SafeProviderAccount;
  addApiKeyAccount(apiKey: string, userLabel: string): SafeProviderAccount;
  createModelRoute(input: CreateModelRouteInput): SafeModelRoute;
  setModelRouteTargets(input: SetModelRouteTargetsInput): SafeModelRoute;
  retireModelRoute(modelTag: string, expectedRevision: number): SafeModelRoute;
  resolveExecutionCandidates(modelTag: string): readonly ModelExecutionCandidate[];
  replaceOAuthCredential(
    accountId: string,
    expectedRefreshToken: string,
    credential: StoredOAuthCredential,
  ): boolean;
  markAccountStatus(accountId: string, status: ProviderAccountStatus): void;
}

/** Private execution material returned only inside the bundled backend process. */
export interface ModelExecutionCandidate {
  readonly modelTag: string;
  readonly semanticModel: string;
  readonly targetId: string;
  readonly providerId: typeof OPENAI_PROVIDER_ID;
  readonly accountId: string;
  readonly upstreamModelId: string;
  readonly priority: number;
  readonly credential: StoredProviderCredential;
}

function emptyDocument(): ModelPlaneDocument {
  return {
    schemaVersion: MODEL_PLANE_SCHEMA_VERSION,
    desiredStateRevision: 0,
    catalogRevision: 0,
    accounts: [],
    modelTags: [],
    displayEntries: [],
  };
}

function storePath(stateRoot: string): string { return join(stateRoot, "model-plane.json"); }

function legacyStorePath(stateRoot: string): string {
  return join(stateRoot, "providers", OPENAI_PROVIDER_ID, "accounts.json");
}

function isRecord(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === "object" && !Array.isArray(value); }

function isSafeText(value: unknown, maxBytes: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && Buffer.byteLength(value, "utf8") <= maxBytes
    && !SAFE_TEXT_CONTROL.test(value);
}

function isSafeAccountStatus(value: unknown): value is ProviderAccountStatus {
  return value === "ready"
    || value === "verificationRequired"
    || value === "reauthenticationRequired";
}

function parseStoredAccount(value: unknown): StoredProviderAccount | null {
  if (!isRecord(value) || !isRecord(value.credential)) return null;
  const credential = value.credential;
  const commonIsValid = isSafeText(value.id, 80)
    && value.providerId === OPENAI_PROVIDER_ID
    && isSafeText(value.userLabel, 80)
    && isSafeAccountStatus(value.status)
    && Number.isSafeInteger(value.addedAt)
    && (value.addedAt as number) >= 0;
  if (!commonIsValid) return null;
  if (credential.kind === "apiKey") {
    if (!isSafeText(credential.apiKey, 64 * 1024)) return null;
    return {
      id: value.id as string,
      providerId: OPENAI_PROVIDER_ID,
      userLabel: value.userLabel as string,
      credentialKind: "apiKey",
      status: value.status as ProviderAccountStatus,
      addedAt: value.addedAt as number,
      credential: { kind: "apiKey", apiKey: credential.apiKey },
    };
  }
  if (
    credential.kind !== undefined && credential.kind !== "oauth"
    || !isSafeText(credential.accessToken, 64 * 1024)
    || !isSafeText(credential.refreshToken, 64 * 1024)
    || !isSafeText(credential.chatgptAccountId, 512)
    || !Number.isSafeInteger(credential.expiresAt)
  ) {
    return null;
  }
  return {
    id: value.id as string,
    providerId: OPENAI_PROVIDER_ID,
    userLabel: value.userLabel as string,
    credentialKind: "oauth",
    status: value.status as ProviderAccountStatus,
    addedAt: value.addedAt as number,
    credential: {
      kind: "oauth",
      accessToken: credential.accessToken as string,
      refreshToken: credential.refreshToken as string,
      chatgptAccountId: credential.chatgptAccountId as string,
      expiresAt: credential.expiresAt as number,
    },
  };
}

function parseStoredTarget(value: unknown): StoredModelTarget | null {
  if (
    !isRecord(value)
    || !isSafeText(value.id, 80)
    || value.providerId !== OPENAI_PROVIDER_ID
    || !isSafeText(value.accountId, 80)
    || !isSafeText(value.upstreamModelId, 512)
    || !Number.isSafeInteger(value.priority)
    || (value.priority as number) < 0
  ) {
    return null;
  }
  return {
    id: value.id as string,
    providerId: OPENAI_PROVIDER_ID,
    accountId: value.accountId as string,
    upstreamModelId: value.upstreamModelId as string,
    priority: value.priority as number,
  };
}

function parseStoredTag(value: unknown): StoredModelTag | null {
  if (
    !isRecord(value)
    || !isSafeText(value.id, 80)
    || !isSafeText(value.semanticModel, 200)
    || !Array.isArray(value.targets)
    || value.targets.length === 0
    || value.targets.length > MODEL_ROUTE_MAX_ROWS
  ) {
    return null;
  }
  const targets = value.targets.map(parseStoredTarget);
  if (targets.some(target => target === null)) return null;
  const typedTargets = targets as StoredModelTarget[];
  if (
    new Set(typedTargets.map(target => target.id)).size !== typedTargets.length
    || typedTargets.some((target, index) => target.priority !== index)
  ) {
    return null;
  }
  return { id: value.id, semanticModel: value.semanticModel, targets: typedTargets };
}

function parseStoredDisplayEntry(value: unknown): StoredDisplayEntry | null {
  if (
    !isRecord(value)
    || !isSafeText(value.modelTag, 80)
    || !isSafeText(value.displayName, 80)
    || typeof value.retired !== "boolean"
    || !Number.isSafeInteger(value.addedAt)
    || (value.addedAt as number) < 0
  ) {
    return null;
  }
  return {
    modelTag: value.modelTag,
    displayName: value.displayName,
    retired: value.retired,
    addedAt: value.addedAt as number,
  };
}

function decodeDocument(bytes: Uint8Array): Record<string, unknown> {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    throw new ModelPlaneError("store_unavailable");
  }
  if (!isRecord(value)) throw new ModelPlaneError("store_unavailable");
  return value;
}

function parseRevisionedAccounts(value: Record<string, unknown>): readonly StoredProviderAccount[] {
  if (
    !Number.isSafeInteger(value.desiredStateRevision)
    || (value.desiredStateRevision as number) < 0
    || !Number.isSafeInteger(value.catalogRevision)
    || (value.catalogRevision as number) < 0
    || !Array.isArray(value.accounts)
    || value.accounts.length > PROVIDER_ACCOUNT_MAX_ROWS
  ) {
    throw new ModelPlaneError("store_unavailable");
  }
  const accounts = value.accounts.map(parseStoredAccount);
  if (accounts.some(account => account === null)) {
    throw new ModelPlaneError("store_unavailable");
  }
  const ids = new Set(accounts.map(account => account!.id));
  if (ids.size !== accounts.length) {
    throw new ModelPlaneError("store_unavailable");
  }
  return accounts as StoredProviderAccount[];
}

function parseDocument(bytes: Uint8Array): ModelPlaneDocument {
  const value = decodeDocument(bytes);
  if (
    value.schemaVersion !== MODEL_PLANE_SCHEMA_VERSION
    || !Array.isArray(value.modelTags)
    || value.modelTags.length > MODEL_ROUTE_MAX_ROWS
    || !Array.isArray(value.displayEntries)
    || value.displayEntries.length > MODEL_ROUTE_MAX_ROWS
  ) {
    throw new ModelPlaneError("store_unavailable");
  }
  const accounts = parseRevisionedAccounts(value);
  const modelTags = value.modelTags.map(parseStoredTag);
  const displayEntries = value.displayEntries.map(parseStoredDisplayEntry);
  if (modelTags.some(tag => tag === null) || displayEntries.some(entry => entry === null)) {
    throw new ModelPlaneError("store_unavailable");
  }
  const typedTags = modelTags as StoredModelTag[];
  const typedEntries = displayEntries as StoredDisplayEntry[];
  const accountIds = new Set(accounts.map(account => account.id));
  const tagIds = new Set(typedTags.map(tag => tag.id));
  const targetIds = typedTags.flatMap(tag => tag.targets.map(target => target.id));
  if (
    tagIds.size !== typedTags.length
    || new Set(typedEntries.map(entry => entry.modelTag)).size !== typedEntries.length
    || new Set(targetIds).size !== targetIds.length
    || typedEntries.some(entry => !tagIds.has(entry.modelTag))
    || typedTags.some(tag => tag.targets.some(target => !accountIds.has(target.accountId)))
  ) {
    throw new ModelPlaneError("store_unavailable");
  }
  return {
    schemaVersion: MODEL_PLANE_SCHEMA_VERSION,
    desiredStateRevision: value.desiredStateRevision as number,
    catalogRevision: value.catalogRevision as number,
    accounts,
    modelTags: typedTags,
    displayEntries: typedEntries,
  };
}

function parseLegacyDocument(bytes: Uint8Array): LegacyProviderAccountDocument {
  const value = decodeDocument(bytes);
  if (value.schemaVersion !== LEGACY_ACCOUNT_SCHEMA_VERSION) {
    throw new ModelPlaneError("store_unavailable");
  }
  return {
    schemaVersion: LEGACY_ACCOUNT_SCHEMA_VERSION,
    desiredStateRevision: value.desiredStateRevision as number,
    catalogRevision: value.catalogRevision as number,
    accounts: parseRevisionedAccounts(value),
  };
}

function readBoundedFile(path: string, errorCode: "source_unavailable" | "store_unavailable"): Uint8Array {
  let fd: number | undefined;
  try {
    fd = openSync(path, constants.O_RDONLY);
    const stat = fstatSync(fd);
    if (!stat.isFile()) throw new ModelPlaneError(errorCode);
    if (stat.size <= 0 || stat.size > PROVIDER_ACCOUNT_STORE_MAX_BYTES) {
      throw new ModelPlaneError(
        errorCode === "source_unavailable" ? "source_too_large" : errorCode,
      );
    }
    const bytes = Buffer.alloc(stat.size);
    let offset = 0;
    while (offset < bytes.length) {
      const read = readSync(fd, bytes, offset, bytes.length - offset, null);
      if (read === 0) break;
      offset += read;
    }
    if (offset !== bytes.length) throw new ModelPlaneError(errorCode);
    return bytes;
  } catch (error) {
    if (error instanceof ModelPlaneError) throw error;
    throw new ModelPlaneError(errorCode);
  } finally {
    if (fd !== undefined) {
      try { closeSync(fd); } catch { /* best effort after a classified failure */ }
    }
  }
}

function hardenDirectory(path: string): void {
  mkdirSync(path, { recursive: true, mode: 0o700 });
  try { chmodSync(path, 0o700); } catch { /* Windows may not implement POSIX modes */ }
}

function persistDocument(stateRoot: string, path: string, document: ModelPlaneDocument): void {
  const directory = dirname(path);
  try {
    hardenDirectory(stateRoot);
    hardenDirectory(directory);
    const temporary = join(directory, `.model-plane.${process.pid}.${randomBytes(8).toString("hex")}.tmp`);
    let fd: number | undefined;
    try {
      fd = openSync(temporary, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY, 0o600);
      writeFileSync(fd, `${JSON.stringify(document, null, 2)}\n`, "utf8");
      fsyncSync(fd);
      closeSync(fd);
      fd = undefined;
      try { chmodSync(temporary, 0o600); } catch { /* Windows may not implement POSIX modes */ }
      renameSync(temporary, path);
    } catch (error) {
      if (fd !== undefined) {
        try { closeSync(fd); } catch { /* keep original error */ }
      }
      try { unlinkSync(temporary); } catch { /* absent or already renamed */ }
      throw error;
    }
  } catch {
    throw new ModelPlaneError("store_unavailable");
  }
}

function loadDocument(stateRoot: string): ModelPlaneDocument {
  const path = storePath(stateRoot);
  if (existsSync(path)) {
    try { chmodSync(path, 0o600); } catch { /* Windows may not implement POSIX modes */ }
    return parseDocument(readBoundedFile(path, "store_unavailable"));
  }
  const legacyPath = legacyStorePath(stateRoot);
  if (!existsSync(legacyPath)) return emptyDocument();
  try { chmodSync(legacyPath, 0o600); } catch { /* Windows may not implement POSIX modes */ }
  const legacy = parseLegacyDocument(readBoundedFile(legacyPath, "store_unavailable"));
  const migrated: ModelPlaneDocument = {
    schemaVersion: MODEL_PLANE_SCHEMA_VERSION,
    desiredStateRevision: legacy.desiredStateRevision,
    catalogRevision: legacy.catalogRevision,
    accounts: legacy.accounts,
    modelTags: [],
    displayEntries: [],
  };
  persistDocument(stateRoot, path, migrated);
  try {
    unlinkSync(legacyPath);
  } catch {
    throw new ModelPlaneError("store_unavailable");
  }
  return migrated;
}

function decodeJwtPayload(token: string): Record<string, unknown> | null {
  const parts = token.split(".");
  if (parts.length !== 3 || !parts[1]) return null;
  try {
    const parsed = JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8"));
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function accountIdFromPayload(payload: Record<string, unknown> | null): string | null {
  if (!payload) return null;
  if (isSafeText(payload.chatgpt_account_id, 512)) return payload.chatgpt_account_id;
  const namespace = payload["https://api.openai.com/auth"];
  if (isRecord(namespace) && isSafeText(namespace.chatgpt_account_id, 512)) {
    return namespace.chatgpt_account_id;
  }
  const organizations = payload.organizations;
  if (Array.isArray(organizations) && isRecord(organizations[0])
    && isSafeText(organizations[0].id, 512)) {
    return organizations[0].id;
  }
  return null;
}

function expiryFromPayload(payload: Record<string, unknown> | null): number | null {
  const seconds = payload?.exp;
  if (typeof seconds !== "number" || !Number.isSafeInteger(seconds) || seconds <= 0) return null;
  const milliseconds = seconds * 1000;
  return Number.isSafeInteger(milliseconds) ? milliseconds : null;
}

function parseCodexAuthJson(bytes: Uint8Array, now: number): StoredOAuthCredential {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    throw new ModelPlaneError("invalid_auth_document");
  }
  if (!isRecord(value) || !isRecord(value.tokens)) {
    throw new ModelPlaneError("invalid_auth_document");
  }
  const tokens = value.tokens;
  if (
    !isSafeText(tokens.access_token, 64 * 1024)
    || !isSafeText(tokens.refresh_token, 64 * 1024)
  ) {
    throw new ModelPlaneError("invalid_auth_document");
  }
  const idToken = isSafeText(tokens.id_token, 64 * 1024) ? tokens.id_token : undefined;
  return oauthCredentialFromTokens({
    accessToken: tokens.access_token,
    refreshToken: tokens.refresh_token,
    idToken,
    explicitAccountId: isSafeText(tokens.account_id, 512) ? tokens.account_id : undefined,
  }, now);
}

/** Convert freshly exchanged OAuth tokens into backend-private execution material. */
export function oauthCredentialFromTokens(
  tokens: {
    readonly accessToken: string;
    readonly refreshToken: string;
    readonly idToken?: string;
    readonly explicitAccountId?: string;
  },
  now: number,
): StoredOAuthCredential {
  if (
    !isSafeText(tokens.accessToken, 64 * 1024)
    || !isSafeText(tokens.refreshToken, 64 * 1024)
    || tokens.idToken !== undefined && !isSafeText(tokens.idToken, 64 * 1024)
    || tokens.explicitAccountId !== undefined && !isSafeText(tokens.explicitAccountId, 512)
  ) {
    throw new ModelPlaneError("invalid_auth_document");
  }
  const accessPayload = decodeJwtPayload(tokens.accessToken);
  const idPayload = tokens.idToken ? decodeJwtPayload(tokens.idToken) : null;
  // Preserve the frozen kernel's extraction order. Codex may retain selected
  // workspace metadata beside tokens whose claims describe a different
  // default workspace; disagreement is not proof that the login is malformed.
  const chatgptAccountId = accountIdFromPayload(idPayload)
    ?? accountIdFromPayload(accessPayload)
    ?? tokens.explicitAccountId;
  const expiresAt = expiryFromPayload(accessPayload) ?? expiryFromPayload(idPayload);
  if (!chatgptAccountId || expiresAt === null) {
    throw new ModelPlaneError("invalid_auth_document");
  }
  if (expiresAt <= now) throw new ModelPlaneError("credential_expired");
  return {
    kind: "oauth",
    accessToken: tokens.accessToken,
    refreshToken: tokens.refreshToken,
    chatgptAccountId,
    expiresAt,
  };
}

function safeAccount(account: StoredProviderAccount, now: number): SafeProviderAccount {
  return {
    id: account.id,
    providerId: OPENAI_PROVIDER_ID,
    userLabel: account.userLabel,
    credentialKind: account.credential.kind,
    status: account.credential.kind === "oauth" && account.credential.expiresAt <= now
      ? "reauthenticationRequired"
      : account.status,
    addedAt: account.addedAt,
  };
}

function safeModelRoute(
  document: ModelPlaneDocument,
  tag: StoredModelTag,
  entry: StoredDisplayEntry,
  now: number,
): SafeModelRoute {
  const accounts = new Map(document.accounts.map(account => [account.id, account]));
  return {
    modelTag: tag.id,
    displayName: entry.displayName,
    retired: entry.retired,
    semanticModel: tag.semanticModel,
    targets: tag.targets.map(target => {
      const account = accounts.get(target.accountId);
      return {
        id: target.id,
        providerId: target.providerId,
        accountId: target.accountId,
        upstreamModelId: target.upstreamModelId,
        priority: target.priority,
        status: account
          && (account.credential.kind === "apiKey" || account.credential.expiresAt > now)
          && account.status !== "reauthenticationRequired"
          ? "unverified" as const
          : "reauthenticationRequired" as const,
      };
    }),
  };
}

function safeSnapshot(document: ModelPlaneDocument, now: number): ProviderAccountSnapshot {
  const accounts = document.accounts.map(account => safeAccount(account, now));
  const tags = new Map(document.modelTags.map(tag => [tag.id, tag]));
  return {
    desiredStateRevision: document.desiredStateRevision,
    catalogRevision: document.catalogRevision,
    providers: [{
      id: OPENAI_PROVIDER_ID,
      displayName: OPENAI_PROVIDER_DISPLAY_NAME,
      accountCount: accounts.length,
      status: accounts.length === 0 ? "needsAccount" : "ready",
    }],
    accounts,
    modelRoutes: document.displayEntries.map(entry => {
      const tag = tags.get(entry.modelTag);
      if (!tag) throw new ModelPlaneError("store_unavailable");
      return safeModelRoute(document, tag, entry, now);
    }),
  };
}

function nextDocument(
  document: ModelPlaneDocument,
  changes: Pick<ModelPlaneDocument, "accounts" | "modelTags" | "displayEntries">,
): ModelPlaneDocument {
  if (
    document.desiredStateRevision >= Number.MAX_SAFE_INTEGER
    || document.catalogRevision >= Number.MAX_SAFE_INTEGER
  ) {
    throw new ModelPlaneError("store_unavailable");
  }
  return {
    schemaVersion: MODEL_PLANE_SCHEMA_VERSION,
    desiredStateRevision: document.desiredStateRevision + 1,
    catalogRevision: document.catalogRevision + 1,
    ...changes,
  };
}

function validateCreateModelRouteInput(input: CreateModelRouteInput): void {
  if (
    !Number.isSafeInteger(input.expectedRevision)
    || input.expectedRevision < 0
    || !isSafeText(input.modelTag, 80)
    || input.modelTag.trim() !== input.modelTag
    || !/^[a-z0-9][a-z0-9._/-]*$/.test(input.modelTag)
    || !isSafeText(input.displayName, 80)
    || input.displayName.trim() !== input.displayName
    || !isSafeText(input.semanticModel, 200)
    || input.semanticModel.trim() !== input.semanticModel
    || input.providerId !== OPENAI_PROVIDER_ID
    || !isSafeText(input.accountId, 80)
    || !isSafeText(input.upstreamModelId, 512)
    || input.upstreamModelId.trim() !== input.upstreamModelId
  ) {
    throw new ModelPlaneError("invalid_request");
  }
}

function validateSetModelRouteTargetsInput(input: SetModelRouteTargetsInput): void {
  if (
    !Number.isSafeInteger(input.expectedRevision)
    || input.expectedRevision < 0
    || !isSafeText(input.modelTag, 80)
    || input.modelTag.trim() !== input.modelTag
    || input.targets.length === 0
    || input.targets.length > PROVIDER_ACCOUNT_MAX_ROWS
    || input.targets.some(target =>
      target.id !== undefined && !isSafeText(target.id, 80)
      || target.providerId !== OPENAI_PROVIDER_ID
      || !isSafeText(target.accountId, 80)
      || !isSafeText(target.upstreamModelId, 512)
      || target.upstreamModelId.trim() !== target.upstreamModelId
    )
  ) {
    throw new ModelPlaneError("invalid_request");
  }
}

/** RichCodex-owned model-plane store; it never inspects native product homes. */
export function createModelPlaneStore(
  stateRoot: string,
  options: {
    readonly now?: () => number;
    readonly createAccountId?: () => string;
    readonly createTargetId?: () => string;
  } = {},
): ModelPlaneStore {
  const path = storePath(stateRoot);
  const now = options.now ?? Date.now;
  const createAccountId = options.createAccountId ?? (() => `account-${randomUUID()}`);
  const createTargetId = options.createTargetId ?? (() => `target-${randomUUID()}`);
  let document = loadDocument(stateRoot);

  const addOAuthAccount = (
    credential: StoredOAuthCredential,
    userLabel: string,
  ): SafeProviderAccount => {
    if (
      credential.kind !== "oauth"
      || !isSafeText(credential.accessToken, 64 * 1024)
      || !isSafeText(credential.refreshToken, 64 * 1024)
      || !isSafeText(credential.chatgptAccountId, 512)
      || !Number.isSafeInteger(credential.expiresAt)
      || credential.expiresAt <= now()
      || !isSafeText(userLabel, 80)
      || userLabel.trim() !== userLabel
    ) {
      throw new ModelPlaneError("invalid_auth_document");
    }
    if (document.accounts.length >= PROVIDER_ACCOUNT_MAX_ROWS) {
      throw new ModelPlaneError("account_limit_reached");
    }
    if (document.accounts.some(account =>
      account.credential.kind === "oauth"
      && account.credential.chatgptAccountId === credential.chatgptAccountId
    )) {
      throw new ModelPlaneError("account_already_exists");
    }
    const id = createAccountId();
    if (!isSafeText(id, 80) || document.accounts.some(account => account.id === id)) {
      throw new ModelPlaneError("store_unavailable");
    }
    const account: StoredProviderAccount = {
      id,
      providerId: OPENAI_PROVIDER_ID,
      userLabel,
      credentialKind: "oauth",
      status: "verificationRequired",
      addedAt: Math.floor(now() / 1000),
      credential,
    };
    const next = nextDocument(document, {
      accounts: [...document.accounts, account],
      modelTags: document.modelTags,
      displayEntries: document.displayEntries,
    });
    persistDocument(stateRoot, path, next);
    document = next;
    return safeAccount(account, now());
  };

  return {
    snapshot(): ProviderAccountSnapshot {
      return safeSnapshot(document, now());
    },
    importCodexAuthJson(authJsonPath: string, userLabel: string): SafeProviderAccount {
      const credential = parseCodexAuthJson(readBoundedFile(authJsonPath, "source_unavailable"), now());
      return addOAuthAccount(credential, userLabel);
    },
    addOAuthAccount(credential: StoredOAuthCredential, userLabel: string): SafeProviderAccount {
      return addOAuthAccount(credential, userLabel);
    },
    addApiKeyAccount(apiKey: string, userLabel: string): SafeProviderAccount {
      if (
        !isSafeText(apiKey, 64 * 1024)
        || apiKey.trim() !== apiKey
        || !isSafeText(userLabel, 80)
        || userLabel.trim() !== userLabel
      ) {
        throw new ModelPlaneError("invalid_api_key");
      }
      if (document.accounts.length >= PROVIDER_ACCOUNT_MAX_ROWS) {
        throw new ModelPlaneError("account_limit_reached");
      }
      if (document.accounts.some(account =>
        account.credential.kind === "apiKey" && account.credential.apiKey === apiKey
      )) {
        throw new ModelPlaneError("account_already_exists");
      }
      const id = createAccountId();
      if (!isSafeText(id, 80) || document.accounts.some(account => account.id === id)) {
        throw new ModelPlaneError("store_unavailable");
      }
      const account: StoredProviderAccount = {
        id,
        providerId: OPENAI_PROVIDER_ID,
        userLabel,
        credentialKind: "apiKey",
        status: "verificationRequired",
        addedAt: Math.floor(now() / 1000),
        credential: { kind: "apiKey", apiKey },
      };
      const next = nextDocument(document, {
        accounts: [...document.accounts, account],
        modelTags: document.modelTags,
        displayEntries: document.displayEntries,
      });
      persistDocument(stateRoot, path, next);
      document = next;
      return safeAccount(account, now());
    },
    createModelRoute(input: CreateModelRouteInput): SafeModelRoute {
      validateCreateModelRouteInput(input);
      if (input.expectedRevision !== document.desiredStateRevision) {
        throw new ModelPlaneError("revision_conflict");
      }
      if (document.modelTags.some(tag => tag.id === input.modelTag)) {
        throw new ModelPlaneError("model_tag_exists");
      }
      if (document.modelTags.length >= MODEL_ROUTE_MAX_ROWS) {
        throw new ModelPlaneError("store_unavailable");
      }
      const account = document.accounts.find(candidate => candidate.id === input.accountId);
      if (!account || account.providerId !== input.providerId) {
        throw new ModelPlaneError("account_unavailable");
      }
      const targetId = createTargetId();
      const existingTargetIds = document.modelTags.flatMap(tag => tag.targets.map(target => target.id));
      if (!isSafeText(targetId, 80) || existingTargetIds.includes(targetId)) {
        throw new ModelPlaneError("store_unavailable");
      }
      const tag: StoredModelTag = {
        id: input.modelTag,
        semanticModel: input.semanticModel,
        targets: [{
          id: targetId,
          providerId: input.providerId,
          accountId: input.accountId,
          upstreamModelId: input.upstreamModelId,
          priority: 0,
        }],
      };
      const entry: StoredDisplayEntry = {
        modelTag: input.modelTag,
        displayName: input.displayName,
        retired: false,
        addedAt: Math.floor(now() / 1000),
      };
      const next = nextDocument(document, {
        accounts: document.accounts,
        modelTags: [...document.modelTags, tag],
        displayEntries: [...document.displayEntries, entry],
      });
      persistDocument(stateRoot, path, next);
      document = next;
      return safeModelRoute(document, tag, entry, now());
    },
    setModelRouteTargets(input: SetModelRouteTargetsInput): SafeModelRoute {
      validateSetModelRouteTargetsInput(input);
      if (input.expectedRevision !== document.desiredStateRevision) {
        throw new ModelPlaneError("revision_conflict");
      }
      const tag = document.modelTags.find(candidate => candidate.id === input.modelTag);
      const entry = document.displayEntries.find(candidate => candidate.modelTag === input.modelTag);
      if (!tag || !entry) throw new ModelPlaneError("model_tag_not_found");
      const accounts = new Map(document.accounts.map(account => [account.id, account]));
      const currentTargets = new Map(tag.targets.map(target => [target.id, target]));
      const occupiedIds = new Set(
        document.modelTags.flatMap(candidate => candidate.targets.map(target => target.id)),
      );
      const selectedIds = new Set<string>();
      const selectedBindings = new Set<string>();
      const targets: StoredModelTarget[] = input.targets.map((target, priority) => {
        const account = accounts.get(target.accountId);
        if (!account || account.providerId !== target.providerId) {
          throw new ModelPlaneError("account_unavailable");
        }
        const binding = `${target.providerId}\u0000${target.accountId}\u0000${target.upstreamModelId}`;
        if (selectedBindings.has(binding)) throw new ModelPlaneError("invalid_request");
        selectedBindings.add(binding);

        let id = target.id;
        if (id !== undefined) {
          if (!currentTargets.has(id) || selectedIds.has(id)) {
            throw new ModelPlaneError("invalid_request");
          }
        } else {
          id = createTargetId();
          if (!isSafeText(id, 80) || occupiedIds.has(id) || selectedIds.has(id)) {
            throw new ModelPlaneError("store_unavailable");
          }
        }
        selectedIds.add(id);
        occupiedIds.add(id);
        return {
          id,
          providerId: target.providerId,
          accountId: target.accountId,
          upstreamModelId: target.upstreamModelId,
          priority,
        };
      });
      const nextTag: StoredModelTag = { ...tag, targets };
      const next = nextDocument(document, {
        accounts: document.accounts,
        modelTags: document.modelTags.map(candidate => candidate.id === tag.id ? nextTag : candidate),
        displayEntries: document.displayEntries,
      });
      persistDocument(stateRoot, path, next);
      document = next;
      return safeModelRoute(document, nextTag, entry, now());
    },
    retireModelRoute(modelTag: string, expectedRevision: number): SafeModelRoute {
      if (
        !isSafeText(modelTag, 80)
        || modelTag.trim() !== modelTag
        || !Number.isSafeInteger(expectedRevision)
        || expectedRevision < 0
      ) {
        throw new ModelPlaneError("invalid_request");
      }
      if (expectedRevision !== document.desiredStateRevision) {
        throw new ModelPlaneError("revision_conflict");
      }
      const tag = document.modelTags.find(candidate => candidate.id === modelTag);
      const entry = document.displayEntries.find(candidate => candidate.modelTag === modelTag);
      if (!tag || !entry) throw new ModelPlaneError("model_tag_not_found");
      if (entry.retired) return safeModelRoute(document, tag, entry, now());
      const nextEntry: StoredDisplayEntry = { ...entry, retired: true };
      const next = nextDocument(document, {
        accounts: document.accounts,
        modelTags: document.modelTags,
        displayEntries: document.displayEntries.map(candidate =>
          candidate.modelTag === modelTag ? nextEntry : candidate
        ),
      });
      persistDocument(stateRoot, path, next);
      document = next;
      return safeModelRoute(document, tag, nextEntry, now());
    },
    resolveExecutionCandidates(modelTag: string): readonly ModelExecutionCandidate[] {
      const tag = document.modelTags.find(candidate => candidate.id === modelTag);
      const display = document.displayEntries.find(candidate => candidate.modelTag === modelTag);
      if (!tag || !display || display.retired) return [];
      const accounts = new Map(document.accounts.map(account => [account.id, account]));
      return tag.targets.flatMap(target => {
        const account = accounts.get(target.accountId);
        if (!account || account.status === "reauthenticationRequired") return [];
        return [{
          modelTag: tag.id,
          semanticModel: tag.semanticModel,
          targetId: target.id,
          providerId: target.providerId,
          accountId: target.accountId,
          upstreamModelId: target.upstreamModelId,
          priority: target.priority,
          credential: { ...account.credential },
        }];
      }).sort((left, right) => left.priority - right.priority || left.targetId.localeCompare(right.targetId));
    },
    replaceOAuthCredential(
      accountId: string,
      expectedRefreshToken: string,
      credential: StoredOAuthCredential,
    ): boolean {
      const account = document.accounts.find(candidate => candidate.id === accountId);
      if (
        !account
        || account.credential.kind !== "oauth"
        || account.credential.refreshToken !== expectedRefreshToken
      ) return false;
      if (
        credential.kind !== "oauth"
        || !isSafeText(credential.accessToken, 64 * 1024)
        || !isSafeText(credential.refreshToken, 64 * 1024)
        || !isSafeText(credential.chatgptAccountId, 512)
        || !Number.isSafeInteger(credential.expiresAt)
        || credential.expiresAt <= now()
      ) {
        throw new ModelPlaneError("store_unavailable");
      }
      const nextAccount: StoredProviderAccount = {
        ...account,
        status: "ready",
        credential,
      };
      const next: ModelPlaneDocument = {
        ...document,
        accounts: document.accounts.map(candidate =>
          candidate.id === accountId ? nextAccount : candidate
        ),
      };
      persistDocument(stateRoot, path, next);
      document = next;
      return true;
    },
    markAccountStatus(accountId: string, status: ProviderAccountStatus): void {
      const account = document.accounts.find(candidate => candidate.id === accountId);
      if (!account || account.status === status) return;
      const next: ModelPlaneDocument = {
        ...document,
        accounts: document.accounts.map(candidate =>
          candidate.id === accountId ? { ...candidate, status } : candidate
        ),
      };
      persistDocument(stateRoot, path, next);
      document = next;
    },
  };
}
