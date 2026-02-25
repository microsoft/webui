// WebUI Node.js + Express integration example.
//
// Serves a WebUI app using the napi-rs native addon for rendering.
// Follows the same patterns as the hyper and tiny_http Rust integrations.
//
// Usage:
//   node src/index.js --app hello-world

import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import express from "express";
import minimist from "minimist";

import { renderToResponse } from "./render.js";
import { startFileWatcher } from "./watcher.js";
import { indexRoute } from "./routes/index.js";
import { assetsRoute } from "./routes/assets.js";
import { hmrRoute } from "./routes/hmr.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function main() {
  const args = minimist(process.argv.slice(2), {
    string: ["app"],
    default: { app: "hello-world" },
  });

  const appDir = path.resolve(__dirname, "..", "..", "..", "app", args.app);
  if (!fs.existsSync(appDir)) {
    process.stderr.write(
      `  ✘ App directory not found: ${appDir}\n` +
        `  hint: Check that the app name matches a folder under examples/app/\n`,
    );
    process.exit(1);
  }

  // Protocol must be pre-built by `webui build`
  const protocolBin = path.join(appDir, "dist", "protocol.bin");
  if (!fs.existsSync(protocolBin)) {
    process.stderr.write(
      `  ✘ Protocol not found: ${protocolBin}\n` +
        `  hint: Run 'cargo run -p webui-cli -- build ${path.join(appDir, "templates")} --out ${path.join(appDir, "dist")}' first\n`,
    );
    process.exit(1);
  }

  const paths = {
    protocolBin,
    data: path.join(appDir, "data", "state.json"),
    assetsDir: path.join(appDir, "assets"),
  };

  // Styled console output (mirrors Rust Printer)
  process.stderr.write(`\n  ⚡ WebUI Express Server\n`);
  process.stderr.write(`  ▸ App       ${args.app}\n`);
  process.stderr.write(`  ▸ Directory ${appDir}\n`);

  // Start file watcher for HMR
  startFileWatcher(paths);
  process.stderr.write(`  ✔ File watcher started\n`);

  // Express server
  const app = express();
  const port = 8080;

  app.get("/", indexRoute(paths));
  app.get("/index.html", indexRoute(paths));
  app.get("/assets/*splat", assetsRoute(paths));
  app.get("/hmr", hmrRoute(paths));

  app.use((_req, res) => {
    res.status(404).type("text/plain").send("Not Found");
  });

  app.listen(port, "127.0.0.1", () => {
    process.stderr.write(`  ▸ URL       http://127.0.0.1:${port}/\n`);
    process.stderr.write(`  ✨ Server is running — press Ctrl+C to stop\n\n`);
  });
}

main();
