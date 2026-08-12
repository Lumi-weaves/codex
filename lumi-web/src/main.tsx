import { StrictMode } from "react";

import { RouterProvider } from "@tanstack/react-router";
import { createRoot } from "react-dom/client";

import "@xyflow/react/dist/style.css";
import { router } from "./shell/router";
import "./styles.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("Root container #root is missing");
}

createRoot(container).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>,
);
