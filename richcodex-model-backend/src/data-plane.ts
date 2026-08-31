import type {
  ModelExecutionCandidate,
  ModelPlaneStore,
  StoredProviderCredential,
  StoredOAuthCredential,
} from "./model-plane";
import {
  ResponsesWebSocketPool,
  type ResponsesWebSocketFactory,
} from "./responses-websocket";

const OPENAI_CODEX_RESPONSES_URL = "https://chatgpt.com/backend-api/codex/responses";
const OPENAI_TOKEN_URL = "https://auth.openai.com/oauth/token";
const OPENAI_CODEX_CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
const MAX_REQUEST_BYTES = 32 * 1024 * 1024;
const DATA_PLANE_TOKEN_HEADER = "x-richcodex-data-plane-token";
const CLIENT_ATTEMPT_ID_REQUEST_HEADER = "x-codex-client-attempt-id";
const CLIENT_ATTEMPT_ID_RESPONSE_HEADER = "x-richcodex-client-attempt-id";
const TOKEN_REFRESH_SKEW_MS = 5 * 60_000;
const QUOTA_EVIDENCE_TTL_MS = 15 * 60_000;
const DEFAULT_COOLDOWN_MS = 60_000;
const MAX_COOLDOWN_MS = 24 * 60 * 60_000;

const FORWARDED_REQUEST_HEADERS = [
  "accept",
  "content-type",
  "openai-beta",
  "originator",
  "session_id",
  "session-id",
  "thread-id",
  "x-client-request-id",
  "x-codex-beta-features",
  "x-codex-installation-id",
  "x-codex-parent-thread-id",
  "x-codex-turn-metadata",
  "x-codex-turn-state",
  "x-codex-window-id",
  "x-openai-subagent",
  "x-responsesapi-include-timing-metrics",
] as const;

const STRIPPED_RESPONSE_HEADERS = new Set([
  "connection",
  "content-encoding",
  "content-length",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "set-cookie",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
]);

interface AccountRuntimeState {
  cooldownUntil?: number;
  quotaResetAt?: number;
  quotaObservedAt?: number;
}

type FetchFunction = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

class CredentialRefreshError extends Error {
  readonly accountStatus: "verificationRequired" | "reauthenticationRequired";

  constructor(accountStatus: "verificationRequired" | "reauthenticationRequired") {
    super("credential_refresh_failed");
    this.name = "CredentialRefreshError";
    this.accountStatus = accountStatus;
  }
}

export interface ModelDataPlaneOptions {
  readonly capability: string;
  readonly modelPlaneStore: ModelPlaneStore;
  readonly fetch?: FetchFunction;
  readonly now?: () => number;
  readonly responsesWebSocketFactory?: ResponsesWebSocketFactory;
  readonly responsesWebSocketProxy?: string;
}

export interface StartedModelDataPlane {
  readonly port: number;
  stop(): Promise<void>;
}

export interface ModelDataPlane {
  handle(request: Request): Promise<Response>;
  start(): StartedModelDataPlane;
}

function staticError(status: number, code: string): Response {
  return Response.json({ error: { code } }, { status });
}

function boundedRetryAfter(headers: Headers, now: number): number {
  const value = headers.get("retry-after")?.trim();
  if (value) {
    const seconds = Number(value);
    if (Number.isFinite(seconds) && seconds >= 0) {
      return Math.min(Math.max(Math.ceil(seconds * 1000), 1), MAX_COOLDOWN_MS);
    }
    const timestamp = Date.parse(value);
    if (Number.isFinite(timestamp)) {
      return Math.min(Math.max(timestamp - now, 1), MAX_COOLDOWN_MS);
    }
  }
  return DEFAULT_COOLDOWN_MS;
}

function finiteHeaderNumber(headers: Headers, name: string): number | undefined {
  const raw = headers.get(name);
  if (raw === null || raw.trim() === "") return undefined;
  const value = Number(raw);
  return Number.isFinite(value) && value >= 0 ? value : undefined;
}

function quotaResetAt(headers: Headers): number | undefined {
  const windows = ["primary", "secondary", "tertiary"] as const;
  const values = windows.flatMap(window => {
    const usedPercent = finiteHeaderNumber(headers, `x-codex-${window}-used-percent`);
    const resetAt = finiteHeaderNumber(headers, `x-codex-${window}-reset-at`);
    return usedPercent !== undefined && usedPercent <= 100 && resetAt !== undefined
      ? [resetAt]
      : [];
  });
  if (values.length === 0) return undefined;
  const earliest = Math.min(...values);
  return earliest < 10_000_000_000 ? earliest * 1000 : earliest;
}

function sortedCandidates(
  candidates: readonly ModelExecutionCandidate[],
  runtime: ReadonlyMap<string, AccountRuntimeState>,
  now: number,
): ModelExecutionCandidate[] {
  const eligible = candidates.filter(candidate => {
    const cooldownUntil = runtime.get(candidate.accountId)?.cooldownUntil;
    return cooldownUntil === undefined || cooldownUntil <= now;
  });
  const byPriority = new Map<number, ModelExecutionCandidate[]>();
  for (const candidate of eligible) {
    const group = byPriority.get(candidate.priority) ?? [];
    group.push(candidate);
    byPriority.set(candidate.priority, group);
  }
  return [...byPriority.entries()]
    .sort(([left], [right]) => left - right)
    .flatMap(([, group]) => {
      const allHaveFreshReset = group.every(candidate => {
        const state = runtime.get(candidate.accountId);
        return state?.quotaResetAt !== undefined
          && state.quotaObservedAt !== undefined
          && now - state.quotaObservedAt <= QUOTA_EVIDENCE_TTL_MS;
      });
      if (!allHaveFreshReset) return group;
      return [...group].sort((left, right) => {
        const leftReset = runtime.get(left.accountId)!.quotaResetAt!;
        const rightReset = runtime.get(right.accountId)!.quotaResetAt!;
        return leftReset - rightReset || left.targetId.localeCompare(right.targetId);
      });
    });
}

function forwardedRequestHeaders(request: Request, credential: StoredProviderCredential): Headers {
  const headers = new Headers();
  for (const name of FORWARDED_REQUEST_HEADERS) {
    const value = request.headers.get(name);
    if (value !== null) headers.set(name, value);
  }
  const bearer = credential.kind === "oauth" ? credential.accessToken : credential.apiKey;
  headers.set("authorization", `Bearer ${bearer}`);
  if (credential.kind === "oauth") {
    headers.set("chatgpt-account-id", credential.chatgptAccountId);
  }
  headers.set("content-type", "application/json");
  return headers;
}

function responseHeaders(
  upstream: Response,
  candidate: ModelExecutionCandidate,
  attempt: number,
  clientAttemptId: string | undefined,
): Headers {
  const headers = new Headers();
  upstream.headers.forEach((value, name) => {
    if (!STRIPPED_RESPONSE_HEADERS.has(name.toLowerCase())) headers.append(name, value);
  });
  addRouteHeaders(headers, candidate, attempt, clientAttemptId);
  return headers;
}

function addRouteHeaders(
  headers: Headers,
  candidate: ModelExecutionCandidate,
  attempt: number,
  clientAttemptId: string | undefined,
): void {
  headers.delete(CLIENT_ATTEMPT_ID_RESPONSE_HEADER);
  headers.set("x-richcodex-model-tag", candidate.modelTag);
  headers.set("x-richcodex-resolved-model", candidate.upstreamModelId);
  headers.set("x-richcodex-provider-id", candidate.providerId);
  headers.set("x-richcodex-account-id", candidate.accountId);
  headers.set("x-richcodex-target-id", candidate.targetId);
  headers.set("x-richcodex-route-attempt", String(attempt));
  if (clientAttemptId !== undefined) {
    headers.set(CLIENT_ATTEMPT_ID_RESPONSE_HEADER, clientAttemptId);
  }
}

function safeReceipt(candidate: ModelExecutionCandidate, attempt: number): string {
  return Buffer.from(JSON.stringify({
    modelTag: candidate.modelTag,
    resolvedModel: candidate.upstreamModelId,
    providerId: candidate.providerId,
    accountId: candidate.accountId,
    targetId: candidate.targetId,
    attempt,
  }), "utf8").toString("base64url");
}

function validCapability(value: string): boolean {
  return value.length >= 32 && value.length <= 512 && /^[A-Za-z0-9._~-]+$/.test(value);
}

function validClientAttemptId(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
}

function validContinuationKey(value: unknown): value is string {
  return typeof value === "string"
    && value.length >= 8
    && value.length <= 512
    && /^[A-Za-z0-9._:-]+$/.test(value);
}

function responsesWebSocketUrl(candidate: ModelExecutionCandidate): string {
  const url = new URL(
    candidate.credential.kind === "oauth"
      ? OPENAI_CODEX_RESPONSES_URL
      : `${candidate.apiBaseUrl}/responses`,
  );
  if (url.protocol === "https:") url.protocol = "wss:";
  else if (url.protocol === "http:") url.protocol = "ws:";
  return url.toString();
}

function retryableUpstreamStatus(status: number): boolean {
  return status === 401
    || status === 402
    || status === 408
    || status === 429
    || status >= 500;
}

/** Build the loopback-only, capability-authenticated model execution plane. */
export function createModelDataPlane(options: ModelDataPlaneOptions): ModelDataPlane {
  if (!validCapability(options.capability)) throw new Error("data_plane_capability_invalid");
  const fetchImpl = options.fetch ?? globalThis.fetch;
  const now = options.now ?? Date.now;
  const runtime = new Map<string, AccountRuntimeState>();
  const refreshFlights = new Map<string, Promise<StoredOAuthCredential>>();
  const responsesWebSockets = new ResponsesWebSocketPool(
    options.responsesWebSocketFactory,
  );

  const refreshCredential = async (
    candidate: ModelExecutionCandidate,
    force: boolean,
  ): Promise<StoredProviderCredential> => {
    if (candidate.credential.kind === "apiKey") return candidate.credential;
    const oauthCredential = candidate.credential;
    const ownedRefreshToken = oauthCredential.refreshToken;
    const ownedExpiresAt = oauthCredential.expiresAt;
    if (ownedRefreshToken === null || ownedExpiresAt === null) {
      return oauthCredential;
    }
    if (!force && ownedExpiresAt > now() + TOKEN_REFRESH_SKEW_MS) {
      return oauthCredential;
    }
    const existing = refreshFlights.get(candidate.accountId);
    if (existing) return existing;
    let flight!: Promise<StoredOAuthCredential>;
    flight = (async () => {
      const response = await fetchImpl(OPENAI_TOKEN_URL, {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "refresh_token",
          client_id: OPENAI_CODEX_CLIENT_ID,
          refresh_token: ownedRefreshToken,
        }),
        redirect: "manual",
      });
      if (!response.ok) {
        throw new CredentialRefreshError(
          response.status === 400 || response.status === 401
            ? "reauthenticationRequired"
            : "verificationRequired",
        );
      }
      const payload = await response.json() as Record<string, unknown>;
      if (
        typeof payload.access_token !== "string"
        || payload.access_token.length === 0
        || payload.access_token.length > 64 * 1024
      ) {
        throw new CredentialRefreshError("verificationRequired");
      }
      const expiresIn = typeof payload.expires_in === "number"
        && Number.isFinite(payload.expires_in)
        && payload.expires_in >= 0
        ? payload.expires_in
        : 3600;
      const refreshToken = typeof payload.refresh_token === "string" && payload.refresh_token.length > 0
        ? payload.refresh_token
        : ownedRefreshToken;
      const credential: StoredOAuthCredential = {
        kind: "oauth",
        accessToken: payload.access_token,
        refreshToken,
        chatgptAccountId: oauthCredential.chatgptAccountId,
        expiresAt: now() + expiresIn * 1000,
      };
      if (!options.modelPlaneStore.replaceOAuthCredential(
        candidate.accountId,
        ownedRefreshToken,
        credential,
      )) {
        const current = options.modelPlaneStore
          .resolveExecutionCandidates(candidate.modelTag)
          .find(value => value.targetId === candidate.targetId);
        if (!current || current.credential.kind !== "oauth") {
          throw new CredentialRefreshError("verificationRequired");
        }
        return current.credential;
      }
      return credential;
    })().finally(() => {
      if (refreshFlights.get(candidate.accountId) === flight) {
        refreshFlights.delete(candidate.accountId);
      }
    });
    refreshFlights.set(candidate.accountId, flight);
    return flight;
  };

  const send = async (
    request: Request,
    body: Record<string, unknown>,
    candidate: ModelExecutionCandidate,
    attempt: number,
    forceRefresh: boolean,
    clientAttemptId: string | undefined,
  ): Promise<Response> => {
    const credential = await refreshCredential(candidate, forceRefresh);
    const upstream = await fetchImpl(
      credential.kind === "oauth"
        ? OPENAI_CODEX_RESPONSES_URL
        : `${candidate.apiBaseUrl}/responses`,
      {
        method: "POST",
        headers: forwardedRequestHeaders(request, credential),
        body: JSON.stringify({ ...body, model: candidate.upstreamModelId }),
        redirect: "manual",
        signal: request.signal,
      },
    );
    const state = runtime.get(candidate.accountId) ?? {};
    const resetAt = quotaResetAt(upstream.headers);
    if (resetAt !== undefined) {
      state.quotaResetAt = resetAt;
      state.quotaObservedAt = now();
    }
    if (
      upstream.status === 402
      || upstream.status === 408
      || upstream.status === 429
      || upstream.status >= 500
    ) {
      state.cooldownUntil = now() + boundedRetryAfter(upstream.headers, now());
    } else if (upstream.ok) {
      delete state.cooldownUntil;
      options.modelPlaneStore.markAccountStatus(candidate.accountId, "ready");
    }
    runtime.set(candidate.accountId, state);
    const headers = responseHeaders(upstream, candidate, attempt, clientAttemptId);
    headers.set("x-richcodex-execution-receipt", safeReceipt(candidate, attempt));
    return new Response(upstream.body, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers,
    });
  };

  const handle = async (request: Request): Promise<Response> => {
    if (new URL(request.url).pathname !== "/v1/responses" || request.method !== "POST") {
      return staticError(404, "not_found");
    }
    if (request.headers.get(DATA_PLANE_TOKEN_HEADER) !== options.capability) {
      return staticError(401, "unauthorized");
    }
    const clientAttemptId = request.headers.get(CLIENT_ATTEMPT_ID_REQUEST_HEADER) ?? undefined;
    if (clientAttemptId !== undefined && !validClientAttemptId(clientAttemptId)) {
      return staticError(400, "invalid_client_attempt_id");
    }
    const declaredLength = Number(request.headers.get("content-length"));
    if (Number.isFinite(declaredLength) && declaredLength > MAX_REQUEST_BYTES) {
      return staticError(413, "request_too_large");
    }
    let body: unknown;
    try {
      const bytes = new Uint8Array(await request.arrayBuffer());
      if (bytes.byteLength === 0 || bytes.byteLength > MAX_REQUEST_BYTES) {
        return staticError(413, "request_too_large");
      }
      body = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
    } catch {
      return staticError(400, "invalid_request");
    }
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return staticError(400, "invalid_request");
    }
    const modelTag = (body as Record<string, unknown>).model;
    if (typeof modelTag !== "string" || modelTag.length === 0 || modelTag.length > 80) {
      return staticError(400, "invalid_model_tag");
    }
    const candidates = sortedCandidates(
      options.modelPlaneStore.resolveExecutionCandidates(modelTag),
      runtime,
      now(),
    );
    if (candidates.length === 0) return staticError(503, "no_eligible_target");

    const continuationKey = (body as Record<string, unknown>).prompt_cache_key;
    const websocketCandidate = candidates[0];
    if (
      websocketCandidate !== undefined
      && websocketCandidate.providerId === "openai"
      && validContinuationKey(continuationKey)
    ) {
      try {
        const credential = await refreshCredential(websocketCandidate, false);
        const headers = forwardedRequestHeaders(request, credential);
        const responseHeaders = new Headers({ "content-type": "text/event-stream" });
        addRouteHeaders(responseHeaders, websocketCandidate, 1, clientAttemptId);
        responseHeaders.set(
          "x-richcodex-execution-receipt",
          safeReceipt(websocketCandidate, 1),
        );
        return await responsesWebSockets.stream({
          continuationKey,
          routeKey: websocketCandidate.targetId,
          url: responsesWebSocketUrl(websocketCandidate),
          headers,
          body: { ...(body as Record<string, unknown>), model: websocketCandidate.upstreamModelId },
          responseHeaders,
          proxy: options.responsesWebSocketProxy,
        });
      } catch {
        // A provider continuation is only an acceleration resource. The complete
        // Kernel-owned request remains sufficient for the ordinary HTTP path.
      }
    }

    let lastResponse: Response | undefined;
    for (const [index, candidate] of candidates.entries()) {
      try {
        let response = await send(
          request,
          body as Record<string, unknown>,
          candidate,
          index + 1,
          false,
          clientAttemptId,
        );
        if (
          response.status === 401
          && candidate.credential.kind === "oauth"
          && candidate.credential.refreshToken !== null
        ) {
          await response.body?.cancel().catch(() => undefined);
          response = await send(
            request,
            body as Record<string, unknown>,
            candidate,
            index + 1,
            true,
            clientAttemptId,
          );
        }
        if (!retryableUpstreamStatus(response.status)) {
          await lastResponse?.body?.cancel().catch(() => undefined);
          return response;
        }
        if (response.status === 401) {
          options.modelPlaneStore.markAccountStatus(candidate.accountId, "reauthenticationRequired");
        }
        await lastResponse?.body?.cancel().catch(() => undefined);
        lastResponse = response;
      } catch (error) {
        options.modelPlaneStore.markAccountStatus(
          candidate.accountId,
          error instanceof CredentialRefreshError
            ? error.accountStatus
            : "verificationRequired",
        );
      }
    }
    return lastResponse ?? staticError(502, "upstream_unavailable");
  };

  return {
    handle,
    start(): StartedModelDataPlane {
      const server = Bun.serve({
        hostname: "127.0.0.1",
        port: 0,
        fetch: handle,
      });
      if (server.port === undefined) {
        server.stop(true);
        throw new Error("data_plane_port_unavailable");
      }
      return {
        port: server.port,
        async stop(): Promise<void> {
          responsesWebSockets.closeAll();
          await server.stop(true);
        },
      };
    },
  };
}
