// HMR version: returns the latest modification time of template or data files.

import fs from "node:fs";

/**
 * Return the HMR version string (latest mtime in ms since epoch).
 *
 * @param {string} templatePath - Path to the template file
 * @param {string} dataPath - Path to the data file
 * @returns {string} Milliseconds since epoch, or "0"
 */
export function hmrVersion(templatePath, dataPath) {
  let latest = 0;

  try {
    const tStat = fs.statSync(templatePath);
    latest = Math.max(latest, tStat.mtimeMs);
  } catch {
    // Template file may not exist
  }

  try {
    const dStat = fs.statSync(dataPath);
    latest = Math.max(latest, dStat.mtimeMs);
  } catch {
    // Data file may not exist
  }

  return latest > 0 ? String(Math.floor(latest)) : "0";
}
