import { render, screen } from "@testing-library/react";
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";

import { createAppRouter } from "./router";

const UPCOMING_LABELS = [
  "Prompt Studio",
  "Providers",
  "Profiles",
  "Releases",
  "Hosts",
];

function renderAt(path: string) {
  const router = createAppRouter(
    createMemoryHistory({ initialEntries: [path] }),
  );
  render(<RouterProvider router={router} />);
  return router;
}

describe("shell navigation contract", () => {
  it("redirects the root path to Agent Operations", async () => {
    const router = renderAt("/");
    await screen.findByRole("heading", { name: "Agent Operations" });
    expect(router.state.location.pathname).toBe("/agent-operations");
  });

  it("marks Agent Operations active and upcoming modules honestly disabled", async () => {
    renderAt("/agent-operations");
    await screen.findByRole("heading", { name: "Agent Operations" });

    const activeLink = screen.getByRole("link", { name: "Agent Operations" });
    expect(activeLink.getAttribute("aria-current")).toBe("page");

    for (const label of UPCOMING_LABELS) {
      // Upcoming modules must not pretend to exist: no link, no route.
      expect(screen.queryByRole("link", { name: label })).toBeNull();
      const entry = screen.getByText(label);
      expect(entry.getAttribute("aria-disabled")).toBe("true");
    }
  });

  it("renders a not-found state for unknown routes", async () => {
    renderAt("/does-not-exist");
    await screen.findByRole("heading", { name: "Page not found" });
  });
});
