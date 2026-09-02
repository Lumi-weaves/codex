const SYSTEM_PROXY_SUCCESS_TTL_MS = 60_000;
const SYSTEM_PROXY_UNAVAILABLE_TTL_MS = 5_000;

type BackendEnvironment = Readonly<Record<string, string | undefined>>;
type FetchInit = RequestInit & { readonly proxy?: string };
export type RouteAwareFetch = (
  input: string | URL | Request,
  init?: FetchInit,
) => Promise<Response>;

export type NetworkRoute =
  | { readonly kind: "direct" }
  | { readonly kind: "proxy"; readonly url: string };

export interface NetworkRouteResolver {
  resolve(url: string): Promise<NetworkRoute>;
}

interface SystemProxyCommandResult {
  readonly exitCode: number;
  readonly stdout: string;
}

interface SystemNetworkRouteResolverOptions {
  readonly env?: BackendEnvironment;
  readonly platform?: NodeJS.Platform;
  readonly now?: () => number;
  readonly readMacOSSystemProxy?: () => Promise<SystemProxyCommandResult>;
}

interface CachedRoute {
  readonly route: NetworkRoute;
  readonly expiresAt: number;
}

function proxyFromEnvironment(
  env: BackendEnvironment,
  protocol: string,
): string | undefined {
  if (protocol === "https:" || protocol === "wss:") {
    return env.HTTPS_PROXY
      ?? env.https_proxy
      ?? env.HTTP_PROXY
      ?? env.http_proxy
      ?? env.ALL_PROXY
      ?? env.all_proxy;
  }
  if (protocol === "http:" || protocol === "ws:") {
    return env.HTTP_PROXY
      ?? env.http_proxy
      ?? env.ALL_PROXY
      ?? env.all_proxy;
  }
  return env.ALL_PROXY ?? env.all_proxy;
}

function ipv4(value: string): number | undefined {
  const parts = value.split(".");
  if (parts.length !== 4) return undefined;
  const octets = parts.map(Number);
  if (octets.some(octet => !Number.isInteger(octet) || octet < 0 || octet > 255)) {
    return undefined;
  }
  return octets.reduce((result, octet) => (result << 8) | octet, 0) >>> 0;
}

function cidrMatches(host: string, candidate: string): boolean {
  const [networkText, prefixText, extra] = candidate.split("/");
  if (extra !== undefined || networkText === undefined || prefixText === undefined) return false;
  const address = ipv4(host);
  const network = ipv4(networkText);
  const prefix = Number(prefixText);
  if (address === undefined || network === undefined || !Number.isInteger(prefix)) return false;
  if (prefix < 0 || prefix > 32) return false;
  const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
  return (address & mask) === (network & mask);
}

function hostMatchesBypass(host: string, rawCandidate: string): boolean {
  const candidate = rawCandidate.trim().toLowerCase();
  const normalizedHost = host.toLowerCase();
  if (candidate === "<local>") return !normalizedHost.includes(".");
  if (candidate === "*") return true;
  if (cidrMatches(normalizedHost, candidate)) return true;
  const suffix = candidate.startsWith("*.")
    ? candidate.slice(1)
    : candidate.startsWith(".") ? candidate : undefined;
  if (suffix !== undefined) {
    return normalizedHost === suffix.slice(1) || normalizedHost.endsWith(suffix);
  }
  return normalizedHost === candidate;
}

function bypassesProxy(host: string, candidates: readonly string[]): boolean {
  return candidates.some(candidate => hostMatchesBypass(host, candidate));
}

function macOSProxyExceptions(output: string): string[] {
  const lines = output.split(/\r?\n/);
  const start = lines.findIndex(line => /^\s*ExceptionsList\s*:\s*<array>\s*\{\s*$/.test(line));
  if (start < 0) return [];
  const values: string[] = [];
  for (const line of lines.slice(start + 1)) {
    if (/^\s*}\s*$/.test(line)) break;
    const match = line.match(/^\s*\d+\s*:\s*(.*?)\s*$/);
    if (match?.[1]) values.push(match[1]);
  }
  return values;
}

function boundedProxyUrl(host: string, port: string): string | undefined {
  const normalizedHost = host.trim();
  const normalizedPort = Number(port);
  if (
    normalizedHost.length === 0
    || normalizedHost.length > 255
    || /[\u0000-\u0020\u007f/@]/.test(normalizedHost)
    || !Number.isSafeInteger(normalizedPort)
    || normalizedPort <= 0
    || normalizedPort > 65_535
  ) return undefined;
  const renderedHost = normalizedHost.includes(":") && !normalizedHost.startsWith("[")
    ? `[${normalizedHost}]`
    : normalizedHost;
  return `http://${renderedHost}:${normalizedPort}`;
}

function field(output: string, name: string): string | undefined {
  const match = output.match(new RegExp(`^\\s*${name}\\s*:\\s*(.*?)\\s*$`, "m"));
  return match?.[1];
}

export function parseMacOSSystemProxy(
  output: string,
  requestUrl: string,
): NetworkRoute | null {
  let url: URL;
  try {
    url = new URL(requestUrl);
  } catch {
    return { kind: "direct" };
  }
  if (
    bypassesProxy(url.hostname, macOSProxyExceptions(output))
    || field(output, "ExcludeSimpleHostnames") === "1" && !url.hostname.includes(".")
  ) return { kind: "direct" };
  const protocol = url.protocol;
  const secure = protocol === "https:" || protocol === "wss:";
  const candidates = secure ? ["HTTPS", "HTTP"] : ["HTTP"];
  for (const candidate of candidates) {
    if (field(output, `${candidate}Enable`) !== "1") continue;
    const host = field(output, `${candidate}Proxy`);
    const port = field(output, `${candidate}Port`);
    if (host === undefined || port === undefined) return null;
    const url = boundedProxyUrl(host, port);
    return url === undefined ? null : { kind: "proxy", url };
  }
  if (
    field(output, "ProxyAutoConfigEnable") === "1"
    || field(output, "ProxyAutoDiscoveryEnable") === "1"
  ) return null;
  return { kind: "direct" };
}

async function readMacOSSystemProxy(): Promise<SystemProxyCommandResult> {
  const process = Bun.spawn(["/usr/sbin/scutil", "--proxy"], {
    stdin: "ignore",
    stdout: "pipe",
    stderr: "ignore",
  });
  const stdout = await new Response(process.stdout).text();
  return { exitCode: await process.exited, stdout };
}

/** Resolve the live outbound route at physical connection boundaries. */
export function createSystemNetworkRouteResolver(
  options: SystemNetworkRouteResolverOptions = {},
): NetworkRouteResolver {
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  const now = options.now ?? Date.now;
  const readSystemProxy = options.readMacOSSystemProxy ?? readMacOSSystemProxy;
  const cache = new Map<string, CachedRoute>();

  return {
    async resolve(requestUrl): Promise<NetworkRoute> {
      let url: URL;
      try {
        url = new URL(requestUrl);
      } catch {
        return { kind: "direct" };
      }
      const key = `${url.protocol}//${url.host}`;
      const cached = cache.get(key);
      const currentTime = now();
      if (cached !== undefined && cached.expiresAt > currentTime) return cached.route;
      cache.delete(key);

      let route: NetworkRoute | null = null;
      let ttl = SYSTEM_PROXY_UNAVAILABLE_TTL_MS;
      if (platform === "darwin") {
        try {
          const result = await readSystemProxy();
          if (result.exitCode === 0) route = parseMacOSSystemProxy(result.stdout, requestUrl);
        } catch {
          route = null;
        }
      }
      if (route !== null) {
        ttl = SYSTEM_PROXY_SUCCESS_TTL_MS;
      } else {
        const proxy = proxyFromEnvironment(env, url.protocol);
        const noProxy = (env.NO_PROXY ?? env.no_proxy ?? "").split(",");
        route = proxy === undefined || bypassesProxy(url.hostname, noProxy)
          ? { kind: "direct" }
          : { kind: "proxy", url: proxy };
      }
      cache.set(key, { route, expiresAt: currentTime + ttl });
      return route;
    },
  };
}

/** Apply the freshly resolved route to every outbound fetch attempt. */
export function createRouteAwareFetch(
  resolver: NetworkRouteResolver,
  fetchImpl: RouteAwareFetch = fetch as RouteAwareFetch,
): RouteAwareFetch {
  return async (input, init) => {
    const requestUrl = input instanceof Request ? input.url : String(input);
    const route = await resolver.resolve(requestUrl);
    return fetchImpl(
      input,
      { ...init, proxy: route.kind === "proxy" ? route.url : "" },
    );
  };
}
