import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// @testing-library/react + React 19 act integration.
(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
});

// --- Localized shims: @xyflow/react under happy-dom ----------------------
// happy-dom does not implement ResizeObserver (used by React Flow viewport
// tracking) or DOMMatrix. Geometry is irrelevant to these tests, so minimal
// no-op shims keep rendering deterministic without asserting on layout.

if (typeof globalThis.ResizeObserver === "undefined") {
  class ResizeObserverShim {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  Object.defineProperty(globalThis, "ResizeObserver", {
    value: ResizeObserverShim,
    writable: true,
    configurable: true,
  });
}

if (typeof globalThis.DOMMatrix === "undefined") {
  class DOMMatrixShim {
    a = 1;
    b = 0;
    c = 0;
    d = 1;
    e = 0;
    f = 0;

    invertSelf(): this {
      return this;
    }

    multiplySelf(): this {
      return this;
    }

    translateSelf(): this {
      return this;
    }

    scaleSelf(): this {
      return this;
    }
  }
  Object.defineProperty(globalThis, "DOMMatrix", {
    value: DOMMatrixShim,
    writable: true,
    configurable: true,
  });
}
