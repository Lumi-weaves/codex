const PERFETTO_ORIGIN = "https://ui.perfetto.dev";
const HANDSHAKE_TIMEOUT_MS = 20_000;
const PING_INTERVAL_MS = 100;

export interface PerfettoTraceRequest {
  traceUrl: string;
  title: string;
  fileName: string;
}

/** Open Perfetto from a user gesture and deliver private trace bytes locally. */
export async function openTraceInPerfetto({
  traceUrl,
  title,
  fileName,
}: PerfettoTraceRequest): Promise<void> {
  const perfettoWindow = window.open(PERFETTO_ORIGIN, "_blank");
  if (perfettoWindow === null) throw new Error("Perfetto pop-up was blocked");

  const response = await fetch(traceUrl);
  if (!response.ok) throw new Error(`Trace fetch failed (${response.status})`);
  const buffer = await response.arrayBuffer();

  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      window.clearInterval(pingTimer);
      window.clearTimeout(timeoutTimer);
      window.removeEventListener("message", onMessage);
      if (error === undefined) resolve();
      else reject(error);
    };
    const onMessage = (event: MessageEvent) => {
      if (
        event.source !== perfettoWindow ||
        event.origin !== PERFETTO_ORIGIN ||
        event.data !== "PONG"
      )
        return;
      perfettoWindow.postMessage(
        {
          perfetto: {
            buffer,
            title,
            fileName,
            localOnly: true,
          },
        },
        PERFETTO_ORIGIN,
        [buffer],
      );
      finish();
    };
    window.addEventListener("message", onMessage);
    const ping = () => perfettoWindow.postMessage("PING", PERFETTO_ORIGIN);
    const pingTimer = window.setInterval(ping, PING_INTERVAL_MS);
    const timeoutTimer = window.setTimeout(
      () => finish(new Error("Perfetto handshake timed out")),
      HANDSHAKE_TIMEOUT_MS,
    );
    ping();
  });
}
