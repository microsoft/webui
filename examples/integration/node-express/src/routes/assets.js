// Route: GET /assets/* — serve static files from app assets directory.

import fs from "node:fs";
import path from "node:path";

const MIME_TYPES = {
  ".css": "text/css",
  ".js": "application/javascript",
  ".html": "text/html",
  ".json": "application/json",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
};

/**
 * @param {object} paths - App paths { assetsDir }
 * @returns {function} Express route handler
 */
export function assetsRoute(paths) {
  return (req, res) => {
    // Express 5 splat params are returned as an array
    const splatParam = req.params.splat;
    const relativePath = Array.isArray(splatParam)
      ? splatParam.join("/")
      : splatParam;
    const filePath = path.join(paths.assetsDir, relativePath);

    // Prevent directory traversal
    const resolved = path.resolve(filePath);
    const assetsResolved = path.resolve(paths.assetsDir);
    const relativeFromAssets = path.relative(assetsResolved, resolved);
    if (
      relativeFromAssets.startsWith("..") ||
      path.isAbsolute(relativeFromAssets)
    ) {
      return res.status(403).type("text/plain").send("Forbidden");
    }

    try {
      const contents = fs.readFileSync(filePath);
      const ext = path.extname(filePath).toLowerCase();
      const contentType = MIME_TYPES[ext] || "application/octet-stream";
      res.type(contentType).send(contents);
    } catch {
      res.status(404).type("text/plain").send("Not Found");
    }
  };
}
