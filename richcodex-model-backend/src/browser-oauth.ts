import { randomUUID } from "node:crypto";
import type { DeviceOAuthCoordinator, SafeProviderLogin } from "./device-oauth";
import {
  ModelPlaneError,
  oauthCredentialFromTokens,
  type ModelPlaneStore,
  type SafeProviderAccount,
} from "./model-plane";
import {
  createRouteAwareFetch,
  createSystemNetworkRouteResolver,
  type NetworkRouteResolver,
  type RouteAwareFetch,
} from "./network-route";

const OPENAI_AUTH_BASE_URL = "https://auth.openai.com";
const OPENAI_CODEX_CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_SCOPE = "openid profile email offline_access api.connectors.read api.connectors.invoke";
const CALLBACK_PATH = "/auth/callback";
const DEFAULT_CALLBACK_PORT = 1455;
const BROWSER_LOGIN_LIFETIME_MS = 5 * 60 * 1000;
const BROWSER_LOGIN_TERMINAL_RETENTION_MS = 15 * 60 * 1000;
const BROWSER_LOGIN_REQUEST_TIMEOUT_MS = 30 * 1000;
const BROWSER_LOGIN_MAX_FLOWS = 1;
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f]/;

type BackendEnvironment = Readonly<Record<string, string | undefined>>;
type FetchInit = RequestInit & { readonly proxy?: string };
type BunServer = ReturnType<typeof Bun.serve>;

interface BrowserLoginFlow {
  readonly loginId: string;
  readonly userLabel: string;
  readonly accountId: string | null;
  readonly expiresAt: number;
  readonly abort: AbortController;
  readonly state: string;
  readonly verifier: string;
  readonly servers: BunServer[];
  readonly timeout: ReturnType<typeof setTimeout>;
  status: SafeProviderLogin["status"];
  verificationUrl: string | null;
  failure: SafeProviderLogin["failure"];
  account: SafeProviderAccount | null;
  terminalAt: number | null;
}

interface BrowserOAuthCoordinatorOptions {
  readonly modelPlaneStore: ModelPlaneStore;
  readonly env?: BackendEnvironment;
  readonly fetch?: RouteAwareFetch;
  readonly networkRouteResolver?: NetworkRouteResolver;
  readonly now?: () => number;
  readonly createLoginId?: () => string;
  readonly callbackPort?: number;
}

function isBoundedText(value: unknown, maxBytes: number): value is string {
  return typeof value === "string"
    && value.length > 0
    && Buffer.byteLength(value, "utf8") <= maxBytes
    && !CONTROL_CHARACTER.test(value);
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function isAddressInUse(error: unknown): boolean {
  return error !== null
    && typeof error === "object"
    && "code" in error
    && (error as { readonly code?: unknown }).code === "EADDRINUSE";
}

function loginFailure(error: unknown): SafeProviderLogin["failure"] {
  if (error instanceof ModelPlaneError) {
    switch (error.code) {
      case "account_already_exists": return "accountAlreadyExists";
      case "account_limit_reached": return "accountLimitReached";
      case "account_not_found": return "accountNotFound";
      case "credential_kind_mismatch": return "credentialKindMismatch";
      case "account_identity_mismatch": return "accountIdentityMismatch";
      case "invalid_auth_document":
      case "credential_expired": return "invalidCredential";
      case "store_unavailable": return "storeUnavailable";
      default: return "unavailable";
    }
  }
  return "unavailable";
}

function randomBase64Url(bytes: number): string {
  const value = new Uint8Array(bytes);
  crypto.getRandomValues(value);
  return Buffer.from(value).toString("base64url");
}

async function pkce(): Promise<{ verifier: string; challenge: string }> {
  const verifier = randomBase64Url(96);
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
  return { verifier, challenge: Buffer.from(digest).toString("base64url") };
}

async function fetchWithTimeout(
  fetchImpl: RouteAwareFetch,
  input: string,
  init: FetchInit,
  parentSignal: AbortSignal,
): Promise<Response> {
  const abort = new AbortController();
  const abortFromParent = (): void => abort.abort();
  parentSignal.addEventListener("abort", abortFromParent, { once: true });
  const timer = setTimeout(() => abort.abort(), BROWSER_LOGIN_REQUEST_TIMEOUT_MS);
  try {
    return await fetchImpl(input, { ...init, signal: abort.signal });
  } finally {
    clearTimeout(timer);
    parentSignal.removeEventListener("abort", abortFromParent);
  }
}

function callbackPage(ok: boolean): Response {
  const title = ok ? "Authorization complete" : "Authorization failed";
  const body = ok
    ? "You can close this tab and return to VibeSeed to finish signing in."
    : "Return to VibeSeed and start a new sign-in attempt.";
  return new Response(
    `<!doctype html><html><head><meta charset="utf-8"><title>VibeSeed</title></head><body><h2>${title}</h2><p>${body}</p></body></html>`,
    { status: ok ? 200 : 400, headers: { "content-type": "text/html; charset=utf-8" } },
  );
}

/** Backend-owned browser OAuth lifecycle. PKCE and callback codes never cross this interface. */
export function createBrowserOAuthCoordinator(
  options: BrowserOAuthCoordinatorOptions,
): DeviceOAuthCoordinator {
  const modelPlaneStore = options.modelPlaneStore;
  const env = options.env ?? process.env;
  const fetchImpl = createRouteAwareFetch(
    options.networkRouteResolver ?? createSystemNetworkRouteResolver({ env }),
    options.fetch ?? fetch,
  );
  const now = options.now ?? Date.now;
  const createLoginId = options.createLoginId ?? (() => `browser-login-${randomUUID()}`);
  const callbackPort = options.callbackPort ?? DEFAULT_CALLBACK_PORT;
  const flows = new Map<string, BrowserLoginFlow>();

  const safe = (flow: BrowserLoginFlow): SafeProviderLogin => {
    const snapshot = modelPlaneStore.snapshot();
    return {
      loginId: flow.loginId,
      status: flow.status,
      verificationUrl: flow.verificationUrl,
      userCode: null,
      expiresAt: Math.floor(flow.expiresAt / 1000),
      failure: flow.failure,
      account: flow.account,
      desiredStateRevision: snapshot.desiredStateRevision,
      catalogRevision: snapshot.catalogRevision,
    };
  };

  const finish = (
    flow: BrowserLoginFlow,
    status: "completed" | "failed" | "cancelled",
    failure: SafeProviderLogin["failure"],
    account: SafeProviderAccount | null = null,
  ): void => {
    if (flow.terminalAt !== null) return;
    clearTimeout(flow.timeout);
    flow.status = status;
    flow.verificationUrl = null;
    flow.failure = failure;
    flow.account = account;
    flow.terminalAt = now();
    for (const server of flow.servers) server.stop();
  };

  const exchange = async (flow: BrowserLoginFlow, code: string): Promise<void> => {
    try {
      const response = await fetchWithTimeout(
        fetchImpl,
        `${OPENAI_AUTH_BASE_URL}/oauth/token`,
        {
          method: "POST",
          headers: { "content-type": "application/x-www-form-urlencoded" },
          body: new URLSearchParams({
            grant_type: "authorization_code",
            client_id: OPENAI_CODEX_CLIENT_ID,
            code,
            redirect_uri: `http://localhost:${flow.servers[0].port}${CALLBACK_PATH}`,
            code_verifier: flow.verifier,
          }).toString(),
        },
        flow.abort.signal,
      );
      if (!response.ok) throw new Error("browser token exchange failed");
      const tokens = record(await response.json());
      if (
        !tokens
        || !isBoundedText(tokens.id_token, 64 * 1024)
        || !isBoundedText(tokens.access_token, 64 * 1024)
        || !isBoundedText(tokens.refresh_token, 64 * 1024)
      ) throw new Error("browser token response is invalid");
      if (flow.abort.signal.aborted) return;
      const credential = oauthCredentialFromTokens({
        idToken: tokens.id_token,
        accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token,
      }, now());
      const account = flow.accountId === null
        ? modelPlaneStore.addOAuthAccount(credential, flow.userLabel)
        : modelPlaneStore.reauthenticateOAuthAccount(flow.accountId, credential);
      finish(flow, "completed", null, account);
    } catch (error) {
      if (flow.abort.signal.aborted) {
        finish(flow, "cancelled", null);
      } else {
        finish(flow, "failed", loginFailure(error));
      }
    }
  };

  const prune = (): void => {
    for (const [loginId, flow] of flows) {
      if (
        flow.terminalAt !== null
        && now() - flow.terminalAt >= BROWSER_LOGIN_TERMINAL_RETENTION_MS
      ) flows.delete(loginId);
    }
  };

  return {
    async start(userLabel: string, accountId?: string): Promise<SafeProviderLogin> {
      prune();
      if (
        !isBoundedText(userLabel, 80)
        || userLabel.trim() !== userLabel
        || accountId !== undefined && !isBoundedText(accountId, 80)
      ) throw new ModelPlaneError("login_unavailable");
      if (accountId !== undefined) {
        const account = modelPlaneStore.snapshot().accounts.find(candidate => candidate.id === accountId);
        if (!account) throw new ModelPlaneError("account_not_found");
        if (account.credentialKind !== "oauth") {
          throw new ModelPlaneError("credential_kind_mismatch");
        }
      }
      if ([...flows.values()].filter(flow => flow.terminalAt === null).length >= BROWSER_LOGIN_MAX_FLOWS) {
        throw new ModelPlaneError("login_limit_reached");
      }

      const loginId = createLoginId();
      if (!isBoundedText(loginId, 80) || flows.has(loginId)) {
        throw new ModelPlaneError("login_unavailable");
      }
      const state = randomBase64Url(24);
      const { verifier, challenge } = await pkce();
      let flow: BrowserLoginFlow;
      const handleCallback = (request: Request): Response => {
        const url = new URL(request.url);
        if (url.pathname !== CALLBACK_PATH || request.method !== "GET") {
          return new Response("Not Found", { status: 404 });
        }
        const returnedState = url.searchParams.get("state");
        const code = url.searchParams.get("code");
        const error = url.searchParams.get("error");
        if (returnedState !== flow.state) {
          return callbackPage(false);
        }
        if (error) {
          finish(flow, "failed", "invalidCredential");
          return callbackPage(false);
        }
        if (!code || !isBoundedText(code, 64 * 1024)) return callbackPage(false);
        flow.status = "exchanging";
        flow.verificationUrl = null;
        void exchange(flow, code);
        return callbackPage(true);
      };

      let primary: BunServer;
      try {
        primary = Bun.serve({
          hostname: "127.0.0.1",
          port: callbackPort,
          reusePort: false,
          fetch: handleCallback,
        });
      } catch {
        throw new ModelPlaneError("login_unavailable");
      }
      const servers = [primary];
      try {
        servers.push(Bun.serve({
          hostname: "::1",
          port: primary.port,
          reusePort: false,
          fetch: handleCallback,
        }));
      } catch (error) {
        if (isAddressInUse(error)) {
          primary.stop(true);
          throw new ModelPlaneError("login_unavailable");
        }
      }
      const redirectUri = `http://localhost:${primary.port}${CALLBACK_PATH}`;
      const params = new URLSearchParams({
        response_type: "code",
        client_id: OPENAI_CODEX_CLIENT_ID,
        redirect_uri: redirectUri,
        scope: OPENAI_SCOPE,
        code_challenge: challenge,
        code_challenge_method: "S256",
        state,
        codex_cli_simplified_flow: "true",
        originator: "richcodex",
        id_token_add_organizations: "true",
        prompt: "login",
      });
      const expiresAt = now() + BROWSER_LOGIN_LIFETIME_MS;
      const timeout = setTimeout(() => finish(flow, "failed", "expired"), BROWSER_LOGIN_LIFETIME_MS);
      flow = {
        loginId,
        userLabel,
        accountId: accountId ?? null,
        expiresAt,
        abort: new AbortController(),
        state,
        verifier,
        servers,
        timeout,
        status: "awaitingUser",
        verificationUrl: `${OPENAI_AUTH_BASE_URL}/oauth/authorize?${params}`,
        failure: null,
        account: null,
        terminalAt: null,
      };
      flows.set(loginId, flow);
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
