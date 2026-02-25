// Route: GET /hmr — return latest file modification version for HMR polling.

import { hmrVersion } from "../hmr.js";

/**
 * @param {object} paths - App paths { template, data }
 * @returns {function} Express route handler
 */
export function hmrRoute(paths) {
  return (_req, res) => {
    const version = hmrVersion(paths.protocolBin, paths.data);
    res.type("text/plain").send(version);
  };
}
