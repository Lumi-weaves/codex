/// <reference types="vitest/config" />
import process from "node:process";

import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import type { Plugin } from "vite";

import { AGENT_OPERATIONS_ENDPOINT } from "./src/api/paths";

// The shell never talks to the app server directly. In development `/api` is
// proxied to an optional, separately launched BFF; without one the frontend
// falls back to deterministic fixtures.
const bffOrigin = process.env.LUMI_WEB_BFF_ORIGIN;

function readOnlyApiGuard(): Plugin {
  return {
    name: "lumi-read-only-api-guard",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const pathname = new URL(request.url ?? "/", "http://localhost")
          .pathname;
        if (!pathname.startsWith("/api")) return next();
        if (
          request.method === "GET" &&
          pathname === AGENT_OPERATIONS_ENDPOINT
        ) {
          return next();
        }
        response.statusCode = request.method === "GET" ? 404 : 405;
        response.setHeader("content-type", "text/plain; charset=utf-8");
        response.end("Unsupported read-only Web API route");
      });
    },
  };
}

export default defineConfig({
  plugins: [readOnlyApiGuard(), react()],
  define: {
    "import.meta.env.VITE_LUMI_WEB_BFF": JSON.stringify(
      bffOrigin ? "true" : "false",
    ),
  },
  server: {
    proxy: bffOrigin
      ? {
          [AGENT_OPERATIONS_ENDPOINT]: {
            target: bffOrigin,
            changeOrigin: true,
          },
        }
      : undefined,
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          graph: ["@dagrejs/dagre", "@xyflow/react"],
          router: ["@tanstack/react-router"],
        },
      },
    },
  },
  test: {
    environment: "happy-dom",
    setupFiles: ["./src/test/setup.ts"],
  },
});
