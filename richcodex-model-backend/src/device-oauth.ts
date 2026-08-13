import { randomUUID } from "node:crypto";
import {
  ModelPlaneError,
  oauthCredentialFromTokens,
  type ModelPlaneStore,
  type SafeProviderAccount,
} from "./model-plane";

const OPENAI_AUTH_BASE_URL = "https://auth.openai.com";
const OPENAI_CODEX_CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_LOGIN_LIFETIME_MS = 15 * 60 * 1000;
const DEVICE_LOGIN_TERMINAL_RETENTION_MS = 15 * 60 * 1000;
const DEVICE_LOGIN_REQUEST_TIMEOUT_MS = 30 * 1000;
const DEVICE_LOGIN_MAX_FLOWS = 16;
const DEVICE_LOGIN_MAX_RETAINED_FLOWS = 32;
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f]/;

type BackendEnvironment = Readonly<Record<string, string | undefined>>;
type FetchInit = RequestInit & { readonly proxy?: string };
type FetchImplementation = (input: string | URL | Request, init?: FetchInit) => Promise<Response>;

export type ProviderLoginStatus =
  | "awaitingUser"
  | "exchanging"
  | "completed"
  | "failed"
  | "cancelled";

export type ProviderLoginFailure =
  | "expired"
  | "unavailable"
  | "invalidCredential"
  | "accountAlreadyExists"
  | "accountLimitReached"
  | "storeUnavailable";

export interface SafeProviderLogin {
  readonly loginId: string;
  readonly status: ProviderLoginStatus;
  readonly verificationUrl: string | null;
  readonly userCode: string | null;
  readonly expiresAt: number;
  readonly failure: ProviderLoginFailure | null;
  readonly account: SafeProviderAccount | null;
  readonly desiredStateRevision: number;
  readonly catalogRevision: number;
}

export interface DeviceOAuthCoordinator {
  start(userLabel: string): Promise<SafeProviderLogin>;
  status(loginId: string): SafeProviderLogin;
  cancel(loginId: string): SafeProviderLogin;
  shutdown(): void;
}

interface LoginFlow {
  readonly loginId: string;
  readonly userLabel: string;
  readonly expiresAt: number;
  readonly abort: AbortController;
  status: ProviderLoginStatus;
  verificationUrl: string | null;
  userCode: string | null;
  failure: ProviderLoginFailure | null;
  account: SafeProviderAccount | null;
  terminalAt: number | null;
  deviceAuthId: string | null;
  intervalMs: number;
}

interface DeviceOAuthCoordinatorOptions {
  readonly modelPlaneStore: ModelPlaneStore;
  readonly env?: BackendEnvironment;
  readonly fetch?: FetchImplementation;
  readonly now?: () => number;
  readonly createLoginId?: () => string;
  readonly sleep?: (milliseconds: number, signal: AbortSignal) => Promise<void>;
}

function isBoundedText(value: unknown, maxBytes: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && Buffer.byteLength(value, "utf8") <= maxBytes
    && !CONTROL_CHARACTER.test(value);
}

function proxyFromEnvironment(env: BackendEnvironment): string | undefined {
  return env.HTTPS_PROXY
    ?? env.https_proxy
    ?? env.ALL_PROXY
    ?? env.all_proxy
    ?? env.HTTP_PROXY
    ?? env.http_proxy;
}

function defaultSleep(milliseconds: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException("Aborted", "AbortError"));
      return;
    }
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(new DOMException("Aborted", "AbortError"));
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, milliseconds);
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

async function fetchWithTimeout(
  fetchImpl: FetchImplementation,
  input: string,
  init: FetchInit,
  parentSignal: AbortSignal | undefined,
): Promise<Response> {
  const abort = new AbortController();
  const abortFromParent = (): void => abort.abort();
  parentSignal?.addEventListener("abort", abortFromParent, { once: true });
  const timer = setTimeout(() => abort.abort(), DEVICE_LOGIN_REQUEST_TIMEOUT_MS);
  try {
    return await fetchImpl(input, { ...init, signal: abort.signal });
  } finally {
    clearTimeout(timer);
    parentSignal?.removeEventListener("abort", abortFromParent);
  }
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function loginFailure(error: unknown): ProviderLoginFailure {
  if (error instanceof ModelPlaneError) {
    switch (error.code) {
      case "account_already_exists": return "accountAlreadyExists";
      case "account_limit_reached": return "accountLimitReached";
      case "invalid_auth_document":
      case "credential_expired": return "invalidCredential";
      case "store_unavailable": return "storeUnavailable";
      default: return "unavailable";
    }
  }
  return "unavailable";
}

/** Backend-owned device OAuth lifecycle. No upstream credential crosses this interface. */
export function createDeviceOAuthCoordinator(
  options: DeviceOAuthCoordinatorOptions,
): DeviceOAuthCoordinator {
  const modelPlaneStore = options.modelPlaneStore;
  const env = options.env ?? process.env;
  const fetchImpl = options.fetch ?? (fetch as FetchImplementation);
  const now = options.now ?? Date.now;
  const createLoginId = options.createLoginId ?? (() => `login-${randomUUID()}`);
  const sleep = options.sleep ?? defaultSleep;
  const proxy = proxyFromEnvironment(env);
  const flows = new Map<string, LoginFlow>();

  const safe = (flow: LoginFlow): SafeProviderLogin => {
    const snapshot = modelPlaneStore.snapshot();
    return {
      loginId: flow.loginId,
      status: flow.status,
      verificationUrl: flow.verificationUrl,
      userCode: flow.userCode,
      expiresAt: Math.floor(flow.expiresAt / 1000),
      failure: flow.failure,
      account: flow.account,
      desiredStateRevision: snapshot.desiredStateRevision,
      catalogRevision: snapshot.catalogRevision,
    };
  };

  const finish = (
    flow: LoginFlow,
    status: "completed" | "failed" | "cancelled",
    failure: ProviderLoginFailure | null,
    account: SafeProviderAccount | null = null,
  ): void => {
    flow.status = status;
    flow.failure = failure;
    flow.account = account;
    flow.terminalAt = now();
    flow.deviceAuthId = null;
    flow.verificationUrl = null;
    flow.userCode = null;
  };

  const poll = async (flow: LoginFlow): Promise<void> => {
    try {
      while (!flow.abort.signal.aborted) {
        if (now() >= flow.expiresAt) {
          finish(flow, "failed", "expired");
          return;
        }
        const response = await fetchWithTimeout(
          fetchImpl,
          `${OPENAI_AUTH_BASE_URL}/api/accounts/deviceauth/token`,
          {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
              device_auth_id: flow.deviceAuthId,
              user_code: flow.userCode,
            }),
            ...(proxy ? { proxy } : {}),
          },
          flow.abort.signal,
        );
        if (response.status === 403 || response.status === 404) {
          await sleep(Math.min(flow.intervalMs, flow.expiresAt - now()), flow.abort.signal);
          continue;
        }
        if (!response.ok) throw new Error("device token poll failed");
        const code = record(await response.json());
        if (
          !code
          || !isBoundedText(code.authorization_code, 64 * 1024)
          || !isBoundedText(code.code_verifier, 64 * 1024)
        ) {
          throw new Error("device token response is invalid");
        }
        flow.status = "exchanging";
        const tokenBody = new URLSearchParams({
          grant_type: "authorization_code",
          code: code.authorization_code,
          redirect_uri: `${OPENAI_AUTH_BASE_URL}/deviceauth/callback`,
          client_id: OPENAI_CODEX_CLIENT_ID,
          code_verifier: code.code_verifier,
        });
        const tokenResponse = await fetchWithTimeout(
          fetchImpl,
          `${OPENAI_AUTH_BASE_URL}/oauth/token`,
          {
            method: "POST",
            headers: { "content-type": "application/x-www-form-urlencoded" },
            body: tokenBody.toString(),
            ...(proxy ? { proxy } : {}),
          },
          flow.abort.signal,
        );
        if (!tokenResponse.ok) throw new Error("device token exchange failed");
        const tokens = record(await tokenResponse.json());
        if (
          !tokens
          || !isBoundedText(tokens.id_token, 64 * 1024)
          || !isBoundedText(tokens.access_token, 64 * 1024)
          || !isBoundedText(tokens.refresh_token, 64 * 1024)
        ) {
          throw new Error("device token exchange response is invalid");
        }
        if (flow.abort.signal.aborted) return;
        const credential = oauthCredentialFromTokens({
          idToken: tokens.id_token,
          accessToken: tokens.access_token,
          refreshToken: tokens.refresh_token,
        }, now());
        const account = modelPlaneStore.addOAuthAccount(credential, flow.userLabel);
        finish(flow, "completed", null, account);
        return;
      }
    } catch (error) {
      if (flow.abort.signal.aborted) {
        if (flow.status !== "cancelled") finish(flow, "cancelled", null);
        return;
      }
      finish(flow, "failed", loginFailure(error));
    }
  };

  const prune = (): void => {
    for (const [loginId, flow] of flows) {
      if (
        flow.terminalAt !== null
        && now() - flow.terminalAt >= DEVICE_LOGIN_TERMINAL_RETENTION_MS
      ) {
        flows.delete(loginId);
      }
    }
    const terminal = [...flows.values()]
      .filter(flow => flow.terminalAt !== null)
      .sort((left, right) => left.terminalAt! - right.terminalAt!);
    while (flows.size >= DEVICE_LOGIN_MAX_RETAINED_FLOWS && terminal.length > 0) {
      flows.delete(terminal.shift()!.loginId);
    }
  };

  return {
    async start(userLabel: string): Promise<SafeProviderLogin> {
      prune();
      if (
        !isBoundedText(userLabel, 80)
        || userLabel.trim() !== userLabel
      ) {
        throw new ModelPlaneError("login_unavailable");
      }
      const activeCount = [...flows.values()].filter(flow => flow.terminalAt === null).length;
      if (activeCount >= DEVICE_LOGIN_MAX_FLOWS) {
        throw new ModelPlaneError("login_limit_reached");
      }
      let response: Response;
      try {
        response = await fetchWithTimeout(
          fetchImpl,
          `${OPENAI_AUTH_BASE_URL}/api/accounts/deviceauth/usercode`,
          {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ client_id: OPENAI_CODEX_CLIENT_ID }),
            ...(proxy ? { proxy } : {}),
          },
          undefined,
        );
      } catch {
        throw new ModelPlaneError("login_unavailable");
      }
      if (!response.ok) throw new ModelPlaneError("login_unavailable");
      let payload: Record<string, unknown> | null;
      try {
        payload = record(await response.json());
      } catch {
        throw new ModelPlaneError("login_unavailable");
      }
      const rawInterval = payload?.interval;
      const intervalSeconds = typeof rawInterval === "string"
        ? Number(rawInterval.trim())
        : rawInterval;
      if (
        !payload
        || !isBoundedText(payload.device_auth_id, 512)
        || !isBoundedText(payload.user_code ?? payload.usercode, 128)
        || typeof intervalSeconds !== "number"
        || !Number.isSafeInteger(intervalSeconds)
        || intervalSeconds < 1
        || intervalSeconds > 60
      ) {
        throw new ModelPlaneError("login_unavailable");
      }
      const loginId = createLoginId();
      if (!isBoundedText(loginId, 80) || flows.has(loginId)) {
        throw new ModelPlaneError("login_unavailable");
      }
      const flow: LoginFlow = {
        loginId,
        userLabel,
        expiresAt: now() + DEVICE_LOGIN_LIFETIME_MS,
        abort: new AbortController(),
        status: "awaitingUser",
        verificationUrl: `${OPENAI_AUTH_BASE_URL}/codex/device`,
        userCode: (payload.user_code ?? payload.usercode) as string,
        failure: null,
        account: null,
        terminalAt: null,
        deviceAuthId: payload.device_auth_id,
        intervalMs: intervalSeconds * 1000,
      };
      flows.set(loginId, flow);
      void poll(flow);
      return safe(flow);
    },
    status(loginId: string): SafeProviderLogin {
      prune();
      const flow = flows.get(loginId);
      if (!flow) throw new ModelPlaneError("login_not_found");
      return safe(flow);
    },
    cancel(loginId: string): SafeProviderLogin {
      prune();
      const flow = flows.get(loginId);
      if (!flow) throw new ModelPlaneError("login_not_found");
      if (flow.terminalAt === null) {
        flow.abort.abort();
        finish(flow, "cancelled", null);
      }
      return safe(flow);
    },
    shutdown(): void {
      for (const flow of flows.values()) {
        if (flow.terminalAt === null) {
          flow.abort.abort();
          finish(flow, "cancelled", null);
        }
      }
    },
  };
}
