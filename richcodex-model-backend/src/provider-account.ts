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

const STORE_SCHEMA_VERSION = 1 as const;
const SAFE_TEXT_CONTROL = /[\u0000-\u001f\u007f]/;

export type ProviderAccountStatus = "verificationRequired" | "reauthenticationRequired";

export interface SafeProviderAccount {
  readonly id: string;
  readonly providerId: typeof OPENAI_PROVIDER_ID;
  readonly userLabel: string;
  readonly status: ProviderAccountStatus;
  readonly addedAt: number;
}

export interface SafeProviderSummary {
  readonly id: typeof OPENAI_PROVIDER_ID;
  readonly displayName: typeof OPENAI_PROVIDER_DISPLAY_NAME;
  readonly accountCount: number;
  readonly status: "ready" | "needsAccount";
}

interface StoredCredential {
  readonly accessToken: string;
  readonly refreshToken: string;
  readonly chatgptAccountId: string;
  readonly expiresAt: number;
}

interface StoredProviderAccount extends SafeProviderAccount {
  readonly credential: StoredCredential;
}

interface ProviderAccountDocument {
  readonly schemaVersion: typeof STORE_SCHEMA_VERSION;
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
  | "store_unavailable";

export class ProviderAccountImportError extends Error {
  readonly code: ProviderAccountImportCode;

  constructor(code: ProviderAccountImportCode) {
    super(code);
    this.name = "ProviderAccountImportError";
    this.code = code;
  }
}

export interface ProviderAccountSnapshot {
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
  readonly providers: readonly SafeProviderSummary[];
  readonly accounts: readonly SafeProviderAccount[];
}

export interface ProviderAccountStore {
  snapshot(): ProviderAccountSnapshot;
  importCodexAuthJson(authJsonPath: string, userLabel: string): SafeProviderAccount;
}

function emptyDocument(): ProviderAccountDocument {
  return { schemaVersion: STORE_SCHEMA_VERSION, desiredStateRevision: 0, catalogRevision: 0, accounts: [] };
}

function storePath(stateRoot: string): string { return join(stateRoot, "providers", OPENAI_PROVIDER_ID, "accounts.json"); }

function isRecord(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === "object" && !Array.isArray(value); }

function isSafeText(value: unknown, maxBytes: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && Buffer.byteLength(value, "utf8") <= maxBytes
    && !SAFE_TEXT_CONTROL.test(value);
}

function isSafeAccountStatus(value: unknown): value is ProviderAccountStatus {
  return value === "verificationRequired" || value === "reauthenticationRequired";
}

function parseStoredAccount(value: unknown): StoredProviderAccount | null {
  if (!isRecord(value) || !isRecord(value.credential)) return null;
  const credential = value.credential;
  if (
    !isSafeText(value.id, 80)
    || value.providerId !== OPENAI_PROVIDER_ID
    || !isSafeText(value.userLabel, 80)
    || !isSafeAccountStatus(value.status)
    || !Number.isSafeInteger(value.addedAt)
    || (value.addedAt as number) < 0
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
    status: value.status as ProviderAccountStatus,
    addedAt: value.addedAt as number,
    credential: {
      accessToken: credential.accessToken as string,
      refreshToken: credential.refreshToken as string,
      chatgptAccountId: credential.chatgptAccountId as string,
      expiresAt: credential.expiresAt as number,
    },
  };
}

function parseDocument(bytes: Uint8Array): ProviderAccountDocument {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    throw new ProviderAccountImportError("store_unavailable");
  }
  if (
    !isRecord(value)
    || value.schemaVersion !== STORE_SCHEMA_VERSION
    || !Number.isSafeInteger(value.desiredStateRevision)
    || (value.desiredStateRevision as number) < 0
    || !Number.isSafeInteger(value.catalogRevision)
    || (value.catalogRevision as number) < 0
    || !Array.isArray(value.accounts)
    || value.accounts.length > PROVIDER_ACCOUNT_MAX_ROWS
  ) {
    throw new ProviderAccountImportError("store_unavailable");
  }
  const accounts = value.accounts.map(parseStoredAccount);
  if (accounts.some(account => account === null)) {
    throw new ProviderAccountImportError("store_unavailable");
  }
  const ids = new Set(accounts.map(account => account!.id));
  if (ids.size !== accounts.length) {
    throw new ProviderAccountImportError("store_unavailable");
  }
  return {
    schemaVersion: STORE_SCHEMA_VERSION,
    desiredStateRevision: value.desiredStateRevision as number,
    catalogRevision: value.catalogRevision as number,
    accounts: accounts as StoredProviderAccount[],
  };
}

function readBoundedFile(path: string, errorCode: "source_unavailable" | "store_unavailable"): Uint8Array {
  let fd: number | undefined;
  try {
    fd = openSync(path, constants.O_RDONLY);
    const stat = fstatSync(fd);
    if (!stat.isFile()) throw new ProviderAccountImportError(errorCode);
    if (stat.size <= 0 || stat.size > PROVIDER_ACCOUNT_STORE_MAX_BYTES) {
      throw new ProviderAccountImportError(
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
    if (offset !== bytes.length) throw new ProviderAccountImportError(errorCode);
    return bytes;
  } catch (error) {
    if (error instanceof ProviderAccountImportError) throw error;
    throw new ProviderAccountImportError(errorCode);
  } finally {
    if (fd !== undefined) {
      try { closeSync(fd); } catch { /* best effort after a classified failure */ }
    }
  }
}

function loadDocument(path: string): ProviderAccountDocument {
  if (!existsSync(path)) return emptyDocument();
  try { chmodSync(path, 0o600); } catch { /* Windows may not implement POSIX modes */ }
  return parseDocument(readBoundedFile(path, "store_unavailable"));
}

function hardenDirectory(path: string): void {
  mkdirSync(path, { recursive: true, mode: 0o700 });
  try { chmodSync(path, 0o700); } catch { /* Windows may not implement POSIX modes */ }
}

function persistDocument(stateRoot: string, path: string, document: ProviderAccountDocument): void {
  const directory = dirname(path);
  try {
    hardenDirectory(stateRoot);
    hardenDirectory(join(stateRoot, "providers"));
    hardenDirectory(directory);
    const temporary = join(directory, `.accounts.${process.pid}.${randomBytes(8).toString("hex")}.tmp`);
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
    throw new ProviderAccountImportError("store_unavailable");
  }
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

function parseCodexAuthJson(bytes: Uint8Array, now: number): StoredCredential {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    throw new ProviderAccountImportError("invalid_auth_document");
  }
  if (!isRecord(value) || !isRecord(value.tokens)) {
    throw new ProviderAccountImportError("invalid_auth_document");
  }
  const tokens = value.tokens;
  if (
    !isSafeText(tokens.access_token, 64 * 1024)
    || !isSafeText(tokens.refresh_token, 64 * 1024)
  ) {
    throw new ProviderAccountImportError("invalid_auth_document");
  }
  const idToken = isSafeText(tokens.id_token, 64 * 1024) ? tokens.id_token : undefined;
  const accessPayload = decodeJwtPayload(tokens.access_token);
  const idPayload = idToken ? decodeJwtPayload(idToken) : null;
  const explicitAccountId = isSafeText(tokens.account_id, 512) ? tokens.account_id : null;
  const identityCandidates = [
    accountIdFromPayload(idPayload),
    accountIdFromPayload(accessPayload),
    explicitAccountId,
  ].filter((candidate): candidate is string => candidate !== null);
  if (new Set(identityCandidates).size > 1) {
    throw new ProviderAccountImportError("invalid_auth_document");
  }
  const chatgptAccountId = identityCandidates[0] ?? null;
  const expiresAt = expiryFromPayload(accessPayload) ?? expiryFromPayload(idPayload);
  if (!chatgptAccountId || expiresAt === null) {
    throw new ProviderAccountImportError("invalid_auth_document");
  }
  if (expiresAt <= now) throw new ProviderAccountImportError("credential_expired");
  return {
    accessToken: tokens.access_token,
    refreshToken: tokens.refresh_token,
    chatgptAccountId,
    expiresAt,
  };
}

function safeAccount(account: StoredProviderAccount, now: number): SafeProviderAccount {
  return {
    id: account.id,
    providerId: OPENAI_PROVIDER_ID,
    userLabel: account.userLabel,
    status: account.credential.expiresAt <= now
      ? "reauthenticationRequired"
      : "verificationRequired",
    addedAt: account.addedAt,
  };
}

function safeSnapshot(document: ProviderAccountDocument, now: number): ProviderAccountSnapshot {
  const accounts = document.accounts.map(account => safeAccount(account, now));
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
  };
}

/** RichCodex-owned account store; it never inspects native Codex or OpenCodex homes. */
export function createProviderAccountStore(
  stateRoot: string,
  options: { readonly now?: () => number; readonly createAccountId?: () => string } = {},
): ProviderAccountStore {
  const path = storePath(stateRoot);
  const now = options.now ?? Date.now;
  const createAccountId = options.createAccountId ?? (() => `account-${randomUUID()}`);
  let document = loadDocument(path);

  return {
    snapshot(): ProviderAccountSnapshot {
      return safeSnapshot(document, now());
    },
    importCodexAuthJson(authJsonPath: string, userLabel: string): SafeProviderAccount {
      const credential = parseCodexAuthJson(readBoundedFile(authJsonPath, "source_unavailable"), now());
      if (document.accounts.length >= PROVIDER_ACCOUNT_MAX_ROWS) {
        throw new ProviderAccountImportError("account_limit_reached");
      }
      if (
        document.desiredStateRevision >= Number.MAX_SAFE_INTEGER
        || document.catalogRevision >= Number.MAX_SAFE_INTEGER
      ) {
        throw new ProviderAccountImportError("store_unavailable");
      }
      if (document.accounts.some(account => account.credential.chatgptAccountId === credential.chatgptAccountId)) {
        throw new ProviderAccountImportError("account_already_exists");
      }
      const id = createAccountId();
      if (!isSafeText(id, 80) || document.accounts.some(account => account.id === id)) {
        throw new ProviderAccountImportError("store_unavailable");
      }
      const account: StoredProviderAccount = {
        id,
        providerId: OPENAI_PROVIDER_ID,
        userLabel,
        status: "verificationRequired",
        addedAt: Math.floor(now() / 1000),
        credential,
      };
      const next: ProviderAccountDocument = {
        schemaVersion: STORE_SCHEMA_VERSION,
        desiredStateRevision: document.desiredStateRevision + 1,
        catalogRevision: document.catalogRevision + 1,
        accounts: [...document.accounts, account],
      };
      persistDocument(stateRoot, path, next);
      document = next;
      return safeAccount(account, now());
    },
  };
}
