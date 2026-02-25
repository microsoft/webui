// Route: GET / or GET /index.html — stream rendered HTML directly.

import fs from "node:fs";
import { renderToResponse } from "../render.js";

/**
 * @param {object} paths - App paths { protocolBin, data }
 * @returns {function} Express route handler
 */
export function indexRoute(paths) {
  return (_req, res) => {
    try {
      res.type("text/html; charset=utf-8");
      renderToResponse(paths, res);
      res.end();
    } catch (err) {
      if (!res.headersSent) {
        res.status(500).type("text/plain");
      }
      res.end(`Render failed: ${err.message}`);
    }
  };
}
