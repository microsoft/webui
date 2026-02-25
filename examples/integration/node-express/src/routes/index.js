// Route: GET / or GET /index.html — serve rendered HTML from dist/

import fs from "node:fs";
import path from "node:path";

/**
 * @param {object} paths - App paths { distDir }
 * @returns {function} Express route handler
 */
export function indexRoute(paths) {
  return (_req, res) => {
    const filePath = path.join(paths.distDir, "index.html");
    try {
      const contents = fs.readFileSync(filePath, "utf-8");
      res.type("text/html; charset=utf-8").send(contents);
    } catch (err) {
      res
        .status(500)
        .type("text/plain")
        .send(`Failed to read dist/index.html: ${err.message}`);
    }
  };
}
