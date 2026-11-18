// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from "node:assert/strict";
import { once } from "node:events";
import { mkdir, readFile, readdir, unlink, writeFile } from "node:fs/promises";
import { get as httpGet } from "node:http";
import { createServer } from "node:net";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { chromium, expect } from "@playwright/test";
import { shutdown, spawnNativeFixture } from "./starter-process.mjs";

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
const native = process.env.WEBUI_INIT_NATIVE_CLI;
assert.ok(native, "WEBUI_INIT_NATIVE_CLI must point to the native WebUI CLI.");
const environment = { PORT: String(port), HOST: "127.0.0.1",
  PATH: process.env.WEBUI_INIT_APP_PATH ?? "" };
const drainBytes = 12 * 1024 * 1024;
const drainName = `acceptance-drain-${process.pid}.css`;
const drainPath = path.join(root, "dist/public", drainName);
await writeFile(drainPath, Buffer.alloc(drainBytes, 32), { flag: "wx" });
const server = spawnNativeFixture(root, native, ['app', 'start'], environment);
server.stdout.on("data", chunk => { output += chunk; });
server.stderr.on("data", chunk => { output += chunk; });
server.on("error", error => { output += error.message; });
let browser;
let report;
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
  const about = await (await fetch(`${origin}/about`)).text();
  assert.doesNotMatch(about, /<script[^>]+(?:src=|type=["']module)/i);
  for (const target of ["/missing", "/protocol.bin", "/dist/protocol.bin", "/server/main.mjs",
    "/assets/../protocol.bin", "/assets/%2e%2e%2fprotocol.bin"]) {
    const response = await fetch(origin + target);
    assert.ok(response.status >= 400, `${target} exposed a private or nonexistent resource`);
  }
  for (const target of ["/assets/index.js/missing.css", "/assets/index.js/missing/deep.css",
    `/assets/${"x".repeat(256)}.css`]) {
    for (const method of ["GET", "HEAD"]) {
      const response = await fetch(origin + target, { method });
      assert.equal(response.status, 404, `${method} ${target}`);
      if (method === "HEAD") assert.equal(await response.text(), "");
      else await response.arrayBuffer();
    }
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
  const plainCounter = plain.locator("app-counter");
  await expect(plainCounter.getByRole("button", { name: "Increment" })).toBeVisible();
  await plainCounter.getByRole("button", { name: "Increment" }).click();
  await expect(plainCounter.getByRole("status")).toHaveText("0");
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
  await browser.close();
  browser = undefined;
  await assertActiveRequestDrains(server, `${origin}/assets/${drainName}`, drainBytes);
  report = { app: root, javascriptBytes, screenshots, http: "passed", browser: "passed", draining: "passed" };
} finally {
  try { await browser?.close(); } finally {
    try { await shutdown(server); } finally { await unlink(drainPath); }
  }
}
console.log(JSON.stringify(report));

async function assertActiveRequestDrains(server, url, expected) {
  const response = await new Promise((resolve, reject) => {
    httpGet(url, { agent: false }, resolve).once("error", reject);
  });
  let stopping;
  try {
    assert.equal(response.statusCode, 200);
    if (response.headers["content-length"] !== undefined) {
      assert.equal(Number(response.headers["content-length"]), expected);
    }
    response.pause();
    await delay(100);
    let received = 0;
    const body = new Promise((resolve, reject) => {
      response.on("data", chunk => { received += chunk.length; });
      response.once("end", resolve);
      response.once("error", reject);
    });
    response.pause();
    stopping = shutdown(server);
    const completed = Promise.all([body, stopping]);
    await Promise.race([completed, delay(100)]);
    assert.equal(server.exitCode, null, "shutdown must wait for the backpressured response");
    response.resume();
    await completed;
    assert.equal(received, expected, "shutdown truncated an active response");
    assert.equal(response.complete, true);
  } finally {
    response.destroy();
    await stopping;
  }
}
