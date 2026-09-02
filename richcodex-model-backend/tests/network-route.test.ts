import { describe, expect, test } from "bun:test";

import {
  createRouteAwareFetch,
  createSystemNetworkRouteResolver,
  parseMacOSSystemProxy,
} from "../src/network-route";

const DIRECT = `
<dictionary> {
  HTTPEnable : 0
  HTTPSEnable : 0
  SOCKSEnable : 0
}
`;

const PROXY = `
<dictionary> {
  ExceptionsList : <array> {
    0 : 127.0.0.1
    1 : 192.168.0.0/16
    2 : localhost
    3 : *.local
  }
  HTTPEnable : 1
  HTTPPort : 7897
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7897
  HTTPSProxy : 127.0.0.1
}
`;

describe("provider network routes", () => {
  test("parses concrete macOS HTTP and HTTPS system proxies", () => {
    expect(parseMacOSSystemProxy(PROXY, "https://chatgpt.com/backend-api/codex/responses"))
      .toEqual({ kind: "proxy", url: "http://127.0.0.1:7897" });
    expect(parseMacOSSystemProxy(PROXY, "wss://api.openai.com/v1/responses"))
      .toEqual({ kind: "proxy", url: "http://127.0.0.1:7897" });
    expect(parseMacOSSystemProxy(PROXY, "https://192.168.10.4/v1/responses"))
      .toEqual({ kind: "direct" });
    expect(parseMacOSSystemProxy(PROXY, "http://host.local/v1/responses"))
      .toEqual({ kind: "direct" });
    expect(parseMacOSSystemProxy(DIRECT, "https://chatgpt.com/"))
      .toEqual({ kind: "direct" });
  });

  test("re-resolves a changed macOS route after the bounded success cache", async () => {
    let now = 0;
    let output = DIRECT;
    let reads = 0;
    const resolver = createSystemNetworkRouteResolver({
      env: {},
      platform: "darwin",
      now: () => now,
      readMacOSSystemProxy: async () => {
        reads += 1;
        return { exitCode: 0, stdout: output };
      },
    });

    expect(await resolver.resolve("https://chatgpt.com/backend-api/codex/responses"))
      .toEqual({ kind: "direct" });
    output = PROXY;
    now = 59_999;
    expect(await resolver.resolve("https://chatgpt.com/backend-api/codex/responses"))
      .toEqual({ kind: "direct" });
    now = 60_000;
    expect(await resolver.resolve("https://chatgpt.com/backend-api/codex/responses"))
      .toEqual({ kind: "proxy", url: "http://127.0.0.1:7897" });
    expect(reads).toBe(2);
  });

  test("falls back to explicit environment proxy when system discovery is unavailable", async () => {
    const resolver = createSystemNetworkRouteResolver({
      env: { HTTPS_PROXY: "http://127.0.0.1:8118" },
      platform: "darwin",
      readMacOSSystemProxy: async () => ({ exitCode: 1, stdout: "" }),
    });

    expect(await resolver.resolve("https://auth.openai.com/oauth/token"))
      .toEqual({ kind: "proxy", url: "http://127.0.0.1:8118" });
  });

  test("honors NO_PROXY when environment routing is the fallback", async () => {
    const resolver = createSystemNetworkRouteResolver({
      env: {
        HTTPS_PROXY: "http://127.0.0.1:8118",
        NO_PROXY: "localhost,.internal.example",
      },
      platform: "linux",
    });

    expect(await resolver.resolve("https://service.internal.example/v1/responses"))
      .toEqual({ kind: "direct" });
  });

  test("applies the current route independently to consecutive fetch attempts", async () => {
    let proxy: string | undefined;
    const seen: Array<string | undefined> = [];
    const routeAwareFetch = createRouteAwareFetch(
      {
        resolve: async () => proxy === undefined
          ? { kind: "direct" }
          : { kind: "proxy", url: proxy },
      },
      async (_input, init) => {
        seen.push(init?.proxy);
        return new Response(null, { status: 200 });
      },
    );

    await routeAwareFetch("https://chatgpt.com/one");
    proxy = "http://127.0.0.1:7897";
    await routeAwareFetch("https://chatgpt.com/two");

    expect(seen).toEqual(["", "http://127.0.0.1:7897"]);
  });
});
