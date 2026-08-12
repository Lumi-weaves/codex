import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// @testing-library/react + React 19 act integration.
(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
});
