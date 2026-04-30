// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Electron integration for WebUI — renders any pre-built WebUI app in a
// frameless desktop window using the @microsoft/webui package.
//
// Prerequisites:
//   1. Build the native addon: cargo build -p microsoft-webui-node --release
//   2. Build the @microsoft/webui package: pnpm --filter @microsoft/webui build
//   3. Install workspace dependencies: pnpm install
//   4. Build a WebUI app:
//      cargo run -p microsoft-webui-cli -- build <app>/src --out <app>/dist --css link --plugin=fast
//      esbuild <app>/src/index.ts --bundle --outfile=<app>/dist/index.js --format=esm
//
// Usage:
//   electron dist/main.js [dist-dir] [state.json] [--plugin=fast]

import { app, BrowserWindow, Menu, net, protocol } from 'electron';
import { existsSync, readFileSync } from 'fs';
import { resolve, join, basename } from 'path';
import { renderStream } from '@microsoft/webui';

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

// Filter out Electron's own args (everything before --)
const args = process.argv.slice(2).filter(a => !a.startsWith('--inspect'));

const positional = args.filter(a => !a.startsWith('--'));
const flags = args.filter(a => a.startsWith('--'));

if (positional.length < 2) {
  console.error('Usage: electron dist/main.js <dist-dir> <state.json> [--plugin=fast]');
  console.error('  dist-dir    Path to the app dist/ directory containing protocol.bin');
  console.error('  state.json  Path to the JSON state file');
  process.exit(1);
}

const distDir = resolve(positional[0]);
const statePath = resolve(positional[1]);

const pluginArg = flags.find(a => a.startsWith('--plugin='));
const pluginName = pluginArg ? pluginArg.split('=')[1] : undefined;

// ---------------------------------------------------------------------------
// Custom protocol
// ---------------------------------------------------------------------------

protocol.registerSchemesAsPrivileged([
  {
    scheme: 'webui',
    privileges: { standard: true, secure: true, supportFetchAPI: true },
  },
]);

// ---------------------------------------------------------------------------
// App lifecycle
// ---------------------------------------------------------------------------

app.whenReady().then(() => {
  Menu.setApplicationMenu(null);

  if (!existsSync(join(distDir, 'protocol.bin'))) {
    console.error(`protocol.bin not found in ${distDir}`);
    console.error('Build the app first: cargo run -p microsoft-webui-cli -- build <app>/src --out <app>/dist');
    app.quit();
    return;
  }

  const protocolBin = readFileSync(join(distDir, 'protocol.bin'));
  const stateJson = existsSync(statePath) ? readFileSync(statePath, 'utf-8') : '{}';

  // Render SSR HTML
  const chunks: string[] = [];
  renderStream(protocolBin, stateJson, (chunk: string) => {
    chunks.push(chunk);
  }, { plugin: pluginName });
  const html = chunks.join('');

  // Protocol handler — serves rendered HTML + static assets
  protocol.handle('webui', (request) => {
    const url = new URL(request.url);

    if (url.pathname === '/' || url.pathname === '') {
      return new Response(html, {
        headers: { 'Content-Type': 'text/html; charset=utf-8' },
      });
    }

    // Serve static assets (CSS, JS, maps) from dist dir
    const filePath = join(distDir, basename(url.pathname));
    if (existsSync(filePath)) {
      return net.fetch(`file://${filePath}`);
    }

    return new Response('Not Found', { status: 404 });
  });

  // Main window
  const win = new BrowserWindow({
    width: 1200,
    height: 800,
    titleBarStyle: 'hidden',
    titleBarOverlay: {
      color: '#ffffff',
      symbolColor: '#374151',
      height: 40,
    },
    webPreferences: {
      preload: join(import.meta.dirname, 'preload.js'),
      contextIsolation: true,
    },
  });

  win.loadURL('webui://app/');
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
