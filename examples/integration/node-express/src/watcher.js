// File watcher: monitors protocol and data files for changes.
// The HMR client polls /hmr and reloads when the version changes.
// Since rendering happens per-request, no re-render step is needed.

import fs from "node:fs";
import path from "node:path";

/**
 * Start watching template and data directories for changes.
 * Re-renders dist/index.html on any modification.
 *
 * @param {object} paths - App paths { template, data, assetsDir, distDir }
 */
export function startFileWatcher(paths) {
  let lastFileTimes = collectFileTimes(paths);

  setInterval(() => {
    const currentFileTimes = collectFileTimes(paths);

    if (hasChanges(lastFileTimes, currentFileTimes)) {
      try {
        renderToIndexHtml(paths);
        process.stderr.write("  ✔ Re-rendered dist/index.html\n");
      } catch (err) {
        process.stderr.write(`  ✘ Re-render failed: ${err.message}\n`);
      }
      lastFileTimes = currentFileTimes;
    }
  }, 500);
}

/**
 * Collect file modification times for watched paths.
 * @returns {Map<string, number>}
 */
function collectFileTimes(paths) {
  const times = new Map();
  const watchDirs = [
    path.dirname(paths.protocolBin),
    path.dirname(paths.data),
    paths.assetsDir,
  ];

  for (const dir of watchDirs) {
    if (!fs.existsSync(dir)) continue;
    collectDirTimes(dir, times);
  }

  return times;
}

function collectDirTimes(dir, times) {
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }

  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      collectDirTimes(fullPath, times);
    } else if (entry.isFile()) {
      try {
        const stat = fs.statSync(fullPath);
        times.set(fullPath, stat.mtimeMs);
      } catch {
        // File may have been deleted between readdir and stat
      }
    }
  }
}

/**
 * Check if any files have changed between two snapshots.
 */
function hasChanges(previous, current) {
  if (previous.size !== current.size) return true;

  for (const [filePath, mtime] of current) {
    if (!previous.has(filePath) || previous.get(filePath) !== mtime) {
      return true;
    }
  }

  for (const filePath of previous.keys()) {
    if (!current.has(filePath)) return true;
  }

  return false;
}
