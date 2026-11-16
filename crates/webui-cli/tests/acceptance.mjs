// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, readFile, readdir } from "node:fs/promises";
import { createServer } from "node:net";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { chromium, expect } from "@playwright/test";

const root = path.resolve(process.argv[2] ?? "");
assert.ok(process.argv[2], "Pass an independently installed and built generated app directory.");
const screenshots = path.resolve(process.env.WEBUI_INIT_SCREENSHOTS ?? "target/init-acceptance");
const reservation = createServer();
reservation.listen(0, "127.0.0.1");
await once(reservation, "listening");
const port = reservation.address().port;
await new Promise((resolve, reject) => reservation.close(error => error ? reject(error) : resolve()));
const origin = `http://127.0.0.1:${port}`;
let output = "";
const server = spawn(process.execPath, ["scripts/app.mjs", "start"], {
  cwd: root,
  env: { ...process.env, PORT: String(port), HOST: "127.0.0.1" },
  stdio: ["ignore", "pipe", "pipe"],
});
server.stdout.on("data", chunk => { output += chunk; });
server.stderr.on("data", chunk => { output += chunk; });
server.on("error", error => { output += error.message; });
const stopped = once(server, "exit");
let browser;
try {
  let ready = false;
  for (let attempt = 0; attempt < 300; attempt += 1) {
    if (server.exitCode !== null) throw new Error(`Server exited:\n${output}`);
    try {
      const response = await fetch(origin, { signal: AbortSignal.timeout(200) });
      if (response.status === 200) { ready = true; break; }
    } catch (error) {
      if (!(error instanceof TypeError) && error.name !== "TimeoutError") throw error;
    }
    await delay(100);
  }
  assert.ok(ready, `Server did not become ready:\n${output}`);
  const home = await (await fetch(origin)).text();
  assert.match(home, /Hello|WebUI/);
  assert.match(home, /app-counter/);
  assert.match(home, /disabled/);
  const about = await (await fetch(`${origin}/about`)).text();
  assert.doesNotMatch(about, /<script[^>]+(?:src=|type=["']module)/i);
  for (const target of ["/missing", "/protocol.bin", "/dist/protocol.bin", "/server/main.mjs",
    "/assets/../protocol.bin", "/assets/%2e%2e%2fprotocol.bin"]) {
    const response = await fetch(origin + target);
    assert.ok(response.status >= 400, `${target} exposed a private or nonexistent resource`);
  }
  const head = await fetch(origin, { method: "HEAD" });
  assert.equal(head.status, 200);
  assert.equal((await head.arrayBuffer()).byteLength, 0);
  const post = await fetch(origin, { method: "POST" });
  assert.equal(post.status, 405);
  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const scripts = [];
  page.on("request", request => {
    if (request.resourceType() === "script") scripts.push(request.url());
  });
  await page.goto(origin);
  const counter = page.locator("app-counter");
  const increment = counter.getByRole("button", { name: /increment|increase|add/i });
  await increment.waitFor();
  await expect(increment).toBeEnabled();
  for (let step = 0; step < 10 && !await increment.evaluate(button => button === button.getRootNode().activeElement); step++) {
    await page.keyboard.press("Tab");
  }
  await expect(increment).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(increment).not.toBeFocused();
  await page.keyboard.press("Tab");
  await expect(increment).toBeFocused();
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.querySelector("app-counter")?.count === 1);
  assert.ok(scripts.length > 0, "Home should load its interactive island");
  await mkdir(screenshots, { recursive: true });
  await page.screenshot({ path: path.join(screenshots, "home-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: path.join(screenshots, "home-mobile.png"), fullPage: true });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  scripts.length = 0;
  await page.getByRole("link", { name: /^about$/i }).first().click();
  await page.waitForURL(`${origin}/about`);
  await page.waitForLoadState("networkidle");
  assert.equal(scripts.length, 0, "About must make zero JavaScript requests");
  await page.screenshot({ path: path.join(screenshots, "about-mobile.png"), fullPage: true });
  await page.goBack();
  await increment.waitFor();
  await expect(increment).toBeEnabled();
  assert.deepEqual(errors, [], "Browser console or unhandled errors");

  const noJs = await browser.newContext({ javaScriptEnabled: false });
  const plain = await noJs.newPage();
  await plain.goto(origin);
  assert.ok(await plain.getByRole("heading", { level: 1 }).isVisible());
  assert.equal(await plain.locator("app-counter button").first().isDisabled(), true);
  await plain.getByRole("link", { name: /^about$/i }).first().click();
  await plain.waitForURL(`${origin}/about`);
  assert.ok(await plain.getByRole("heading", { level: 1 }).isVisible());
  await noJs.close();
  const files = await readdir(path.join(root, "dist/public"), { recursive: true });
  const js = files.filter(file => file.endsWith(".js"));
  let javascriptBytes = 0;
  for (const file of js) {
    const source = await readFile(path.join(root, "dist/public", file), "utf8");
    javascriptBytes += Buffer.byteLength(source);
    assert.doesNotMatch(source, /\[WebUI\] Hydration mismatch/);
  }
  console.log(JSON.stringify({ app: root, javascriptBytes, screenshots, http: "passed", browser: "passed" }));
} finally {
  await browser?.close();
  if (server.exitCode === null) {
    server.kill("SIGTERM");
    const timer = setTimeout(() => server.kill("SIGKILL"), 10_000);
    try { await stopped; } finally { clearTimeout(timer); }
  }
}
