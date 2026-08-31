const RESPONSES_WEBSOCKET_BETA = "responses_websockets=2026-02-06";
const PREVIOUS_RESPONSE_NOT_FOUND = "previous_response_not_found";
const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;
const DEFAULT_MAX_CONNECTIONS = 64;

type JsonObject = Record<string, unknown>;

export type ResponsesWebSocketFactory = (
  url: string,
  options: { readonly headers: Headers; readonly proxy?: string },
) => WebSocket;

export interface ResponsesWebSocketStreamRequest {
  readonly continuationKey: string;
  readonly routeKey: string;
  readonly url: string;
  readonly headers: Headers;
  readonly body: JsonObject;
  readonly responseHeaders: Headers;
  readonly proxy?: string;
}

interface LastResponse {
  readonly responseId: string;
  readonly output: readonly unknown[];
}

function defaultWebSocketFactory(
  url: string,
  options: { readonly headers: Headers; readonly proxy?: string },
): WebSocket {
  // Bun extends the WebSocket constructor with request headers, while the
  // ambient DOM declaration selected by TypeScript exposes only protocols.
  return Reflect.construct(WebSocket, [
    url,
    {
      headers: Object.fromEntries(options.headers),
      ...(options.proxy === undefined ? {} : { proxy: options.proxy }),
    },
  ]) as WebSocket;
}

function ownedObject(value: unknown): JsonObject | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonObject
    : undefined;
}

function eventType(value: unknown): string | undefined {
  const type = ownedObject(value)?.type;
  return typeof type === "string" ? type : undefined;
}

function responseOf(value: unknown): JsonObject | undefined {
  return ownedObject(ownedObject(value)?.response);
}

function responseIdOf(value: unknown): string | undefined {
  const id = responseOf(value)?.id;
  return typeof id === "string" && id.length > 0 ? id : undefined;
}

function errorCodeOf(value: unknown): string | undefined {
  const error = ownedObject(ownedObject(value)?.error);
  const code = error?.code;
  return typeof code === "string" ? code : undefined;
}

function sameJson(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function requestProperties(body: JsonObject): JsonObject {
  const properties = { ...body };
  delete properties.input;
  delete properties.client_metadata;
  delete properties.stream_options;
  return properties;
}

function incrementalInput(
  previousRequest: JsonObject,
  lastResponse: LastResponse,
  currentRequest: JsonObject,
): unknown[] | undefined {
  if (!sameJson(requestProperties(previousRequest), requestProperties(currentRequest))) {
    return undefined;
  }
  const previousInput = previousRequest.input;
  const currentInput = currentRequest.input;
  if (!Array.isArray(previousInput) || !Array.isArray(currentInput)) return undefined;
  const prefix = [...previousInput, ...lastResponse.output];
  if (prefix.length > currentInput.length) return undefined;
  for (const [index, item] of prefix.entries()) {
    if (!sameJson(item, currentInput[index])) return undefined;
  }
  return currentInput.slice(prefix.length);
}

function createPayload(
  body: JsonObject,
  previousRequest: JsonObject | undefined,
  lastResponse: LastResponse | undefined,
): { readonly payload: JsonObject; readonly incremental: boolean } {
  if (previousRequest !== undefined && lastResponse !== undefined) {
    const input = incrementalInput(previousRequest, lastResponse, body);
    if (input !== undefined) {
      return {
        payload: {
          type: "response.create",
          ...body,
          previous_response_id: lastResponse.responseId,
          input,
        },
        incremental: true,
      };
    }
  }
  return { payload: { type: "response.create", ...body }, incremental: false };
}

function messageText(event: MessageEvent): string | undefined {
  return typeof event.data === "string" ? event.data : undefined;
}

function terminalEvent(type: string | undefined): boolean {
  return type === "response.completed"
    || type === "response.failed"
    || type === "response.incomplete"
    || type === "error";
}

class ResponsesWebSocketConnection {
  private active = false;
  private closed = false;
  private lastRequest: JsonObject | undefined;
  private lastResponse: LastResponse | undefined;
  private lastUsed = Date.now();

  constructor(
    readonly routeKey: string,
    private readonly socket: WebSocket,
    private readonly onClosed: () => void,
  ) {
    socket.addEventListener("close", () => {
      this.closed = true;
      this.onClosed();
    });
  }

  get idleSince(): number {
    return this.active ? Number.POSITIVE_INFINITY : this.lastUsed;
  }

  get usable(): boolean {
    return !this.closed && this.socket.readyState === WebSocket.OPEN;
  }

  stream(body: JsonObject, responseHeaders: Headers): Response {
    if (!this.usable || this.active) throw new Error("responses_websocket_unavailable");
    this.active = true;
    this.lastUsed = Date.now();
    const initial = createPayload(body, this.lastRequest, this.lastResponse);
    let retriedFull = false;
    let responseId: string | undefined;
    let pendingOutput: unknown[] = [];
    let settled = false;
    let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
    const encoder = new TextEncoder();

    const cleanup = (): void => {
      this.socket.removeEventListener("message", onMessage);
      this.socket.removeEventListener("error", onError);
      this.socket.removeEventListener("close", onClose);
      this.active = false;
      this.lastUsed = Date.now();
    };
    const fail = (error: Error): void => {
      if (settled) return;
      settled = true;
      cleanup();
      controller?.error(error);
      this.close();
    };
    const finish = (): void => {
      if (settled) return;
      settled = true;
      cleanup();
      controller?.enqueue(encoder.encode("data: [DONE]\n\n"));
      controller?.close();
    };
    const send = (payload: JsonObject): void => {
      this.socket.send(JSON.stringify(payload));
    };
    const onMessage = (event: MessageEvent): void => {
      const text = messageText(event);
      if (text === undefined) {
        fail(new Error("responses_websocket_non_text_message"));
        return;
      }
      let value: unknown;
      try {
        value = JSON.parse(text);
      } catch {
        fail(new Error("responses_websocket_invalid_event"));
        return;
      }
      const type = eventType(value);
      if (
        initial.incremental
        && !retriedFull
        && type === "error"
        && errorCodeOf(value) === PREVIOUS_RESPONSE_NOT_FOUND
      ) {
        retriedFull = true;
        responseId = undefined;
        pendingOutput = [];
        this.lastRequest = undefined;
        this.lastResponse = undefined;
        send({ type: "response.create", ...body });
        return;
      }
      responseId = responseIdOf(value) ?? responseId;
      if (type === "response.output_item.done") {
        const item = ownedObject(value)?.item;
        if (item !== undefined) pendingOutput.push(item);
      }
      controller?.enqueue(encoder.encode(`data: ${text}\n\n`));
      if (!terminalEvent(type)) return;
      if (type === "response.completed") {
        const completedOutput = responseOf(value)?.output;
        const output = Array.isArray(completedOutput) ? completedOutput : pendingOutput;
        if (responseId !== undefined) {
          this.lastRequest = structuredClone(body);
          this.lastResponse = {
            responseId,
            output: structuredClone(output),
          };
        } else {
          this.lastRequest = undefined;
          this.lastResponse = undefined;
        }
        finish();
      } else {
        finish();
        this.close();
      }
    };
    const onError = (): void => fail(new Error("responses_websocket_stream_failed"));
    const onClose = (): void => fail(new Error("responses_websocket_closed"));

    const stream = new ReadableStream<Uint8Array>({
      start: streamController => {
        controller = streamController;
        this.socket.addEventListener("message", onMessage);
        this.socket.addEventListener("error", onError);
        this.socket.addEventListener("close", onClose);
        try {
          send(initial.payload);
        } catch {
          fail(new Error("responses_websocket_send_failed"));
        }
      },
      cancel: () => this.close(),
    });
    return new Response(stream, { status: 200, headers: responseHeaders });
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    try {
      this.socket.close(1000, "continuation closed");
    } finally {
      this.onClosed();
    }
  }
}

async function openSocket(
  factory: ResponsesWebSocketFactory,
  url: string,
  headers: Headers,
  proxy: string | undefined,
  timeoutMs: number,
): Promise<WebSocket> {
  const socket = factory(url, { headers, proxy });
  return await new Promise<WebSocket>((resolve, reject) => {
    let settled = false;
    const settle = (result: "open" | "error"): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.removeEventListener("open", onOpen);
      socket.removeEventListener("error", onError);
      socket.removeEventListener("close", onError);
      if (result === "open") resolve(socket);
      else reject(new Error("responses_websocket_connect_failed"));
    };
    const onOpen = (): void => settle("open");
    const onError = (): void => settle("error");
    const timer = setTimeout(() => {
      try {
        socket.close(1000, "connect timeout");
      } finally {
        settle("error");
      }
    }, timeoutMs);
    socket.addEventListener("open", onOpen);
    socket.addEventListener("error", onError);
    socket.addEventListener("close", onError);
  });
}

/** Ephemeral, bounded provider continuation pool; callers retain full replay authority. */
export class ResponsesWebSocketPool {
  private readonly connections = new Map<string, ResponsesWebSocketConnection>();

  constructor(
    private readonly factory: ResponsesWebSocketFactory = defaultWebSocketFactory,
    private readonly connectTimeoutMs = DEFAULT_CONNECT_TIMEOUT_MS,
    private readonly maxConnections = DEFAULT_MAX_CONNECTIONS,
  ) {}

  async stream(request: ResponsesWebSocketStreamRequest): Promise<Response> {
    let connection = this.connections.get(request.continuationKey);
    if (
      connection !== undefined
      && (connection.routeKey !== request.routeKey || !connection.usable)
    ) {
      connection.close();
      connection = undefined;
    }
    if (connection === undefined) {
      this.makeRoom();
      const headers = new Headers(request.headers);
      headers.set("openai-beta", RESPONSES_WEBSOCKET_BETA);
      const socket = await openSocket(
        this.factory,
        request.url,
        headers,
        request.proxy,
        this.connectTimeoutMs,
      );
      const created = new ResponsesWebSocketConnection(
        request.routeKey,
        socket,
        () => {
          if (this.connections.get(request.continuationKey) === created) {
            this.connections.delete(request.continuationKey);
          }
        },
      );
      this.connections.set(request.continuationKey, created);
      connection = created;
    }
    return connection.stream(request.body, request.responseHeaders);
  }

  closeAll(): void {
    for (const connection of [...this.connections.values()]) connection.close();
    this.connections.clear();
  }

  private makeRoom(): void {
    if (this.connections.size < this.maxConnections) return;
    const candidate = [...this.connections.entries()]
      .filter(([, connection]) => Number.isFinite(connection.idleSince))
      .sort(([, left], [, right]) => left.idleSince - right.idleSince)[0];
    if (candidate === undefined) throw new Error("responses_websocket_pool_busy");
    candidate[1].close();
  }
}
