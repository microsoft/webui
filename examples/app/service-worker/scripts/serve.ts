// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createReadStream } from "fs";
import { stat } from "fs/promises";
import { createServer, type ServerResponse } from "http";
import { extname, isAbsolute, relative, resolve, sep } from "path";
import { fileURLToPath } from "url";

const here = fileURLToPath(new URL(".", import.meta.url));
const publicDir = resolve(here, "../public");
const port = parsePort(process.argv);

const server = createServer(async (request, response) => {
  try {
    const requestUrl = new URL(request.url ?? "/", `http://${request.headers.host ?? "localhost"}`);
    const filePath = resolvePublicPath(requestUrl.pathname);
    const info = await stat(filePath);
    if (!info.isFile()) {
      respondText(response, 404, "not found");
      return;
    }

    response.writeHead(200, {
      "Content-Type": contentType(filePath),
      "Cache-Control": "no-store",
    });
    createReadStream(filePath).pipe(response);
  } catch (error) {
    respondText(response, statusForError(error), "not found");
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`service-worker example serving http://127.0.0.1:${port}/`);
});

function parsePort(args: string[]): number {
  const index = args.indexOf("--port");
  const raw = index >= 0 ? args[index + 1] : "4175";
  const parsed = Number(raw);
  if (!Number.isInteger(parsed) || parsed <= 0 || parsed > 65_535) {
    throw new Error(`Invalid --port value: ${raw ?? "(missing)"}`);
  }
  return parsed;
}

function resolvePublicPath(pathname: string): string {
  const decoded = decodeURIComponent(pathname);
  const withIndex = decoded.endsWith("/") ? `${decoded}index.html` : decoded;
  const filePath = resolve(publicDir, `.${withIndex}`);
  const rel = relative(publicDir, filePath);
  if (rel === ".." || rel.startsWith(`..${sep}`) || isAbsolute(rel)) {
    throw Object.assign(new Error("forbidden"), { statusCode: 403 });
  }
  return filePath;
}

function contentType(filePath: string): string {
  switch (extname(filePath)) {
    case ".css":
      return "text/css; charset=utf-8";
    case ".html":
      return "text/html; charset=utf-8";
    case ".js":
      return "text/javascript; charset=utf-8";
    case ".json":
      return "application/json; charset=utf-8";
    case ".wasm":
      return "application/wasm";
    default:
      return "application/octet-stream";
  }
}

function statusForError(error: unknown): number {
  if (isStatusError(error)) {
    return error.statusCode;
  }
  if (isNodeError(error) && error.code === "ENOENT") {
    return 404;
  }
  return 500;
}

function respondText(response: ServerResponse, statusCode: number, text: string): void {
  response.writeHead(statusCode, {
    "Content-Type": "text/plain; charset=utf-8",
    "Cache-Control": "no-store",
  });
  response.end(text);
}

function isStatusError(error: unknown): error is { statusCode: number } {
  return (
    typeof error === "object" &&
    error !== null &&
    "statusCode" in error &&
    typeof error.statusCode === "number"
  );
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return typeof error === "object" && error !== null && "code" in error;
}
