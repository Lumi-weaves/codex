import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  redirect,
} from "@tanstack/react-router";
import type { RouterHistory } from "@tanstack/react-router";

import { AgentOperationsPage } from "../features/AgentOperationsPage";
import { ShellLayout } from "./ShellLayout";

function NotFound() {
  return (
    <div className="state-panel">
      <h1 className="page__title">Page not found</h1>
      <p>This area of the console does not exist yet.</p>
      <Link className="button" to="/agent-operations">
        Go to Agent Operations
      </Link>
    </div>
  );
}

const rootRoute = createRootRoute({
  component: ShellLayout,
  notFoundComponent: NotFound,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/agent-operations", replace: true });
  },
});

const agentOperationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agent-operations",
  component: AgentOperationsPage,
});

const routeTree = rootRoute.addChildren([indexRoute, agentOperationsRoute]);

export function createAppRouter(history?: RouterHistory) {
  return createRouter({ routeTree, history });
}

export const router = createAppRouter();

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
