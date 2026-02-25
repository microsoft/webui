// HMR version: returns the latest modification time of template or data files.

import fs from "node:fs";

/**
 * Return the HMR version string (latest mtime in ms since epoch).
 *
 * @param {string} protocolBinPath - Path to the protocol.bin file
 * @param {string} dataPath - Path to the data file
 * @returns {string} Milliseconds since epoch, or "0"
 */
export function hmrVersion(protocolBinPath, dataPath) {
  let latest = 0;

  try {
    const tStat = fs.statSync(protocolBinPath);
    latest = Math.max(latest, tStat.mtimeMs);
  } catch {
    // Protocol file may not exist
  }

  try {
    const dStat = fs.statSync(dataPath);
    latest = Math.max(latest, dStat.mtimeMs);
  } catch {
    // Data file may not exist
  }

  return latest > 0 ? String(Math.floor(latest)) : "0";
}
