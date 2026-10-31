// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Static server for the NTP fixtures, and the generated background image.

import { once } from 'node:events';
import { createServer } from 'node:http';
import { brotli } from './build.mjs';

const IMMUTABLE = 'public, max-age=31536000, immutable';
const PIXEL = Buffer.from('R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7', 'base64');
const MAX_HOLD_MS = 5000;

/**
 * A deterministic 1600x1000 landscape JPEG (about 170 KB), drawn by the
 * browser itself so no image library is needed. It is the page's LCP element.
 */
export async function backgroundImage(browser) {
  const page = await browser.newPage();
  try {
    const url = await page.evaluate(({ width, height }) => {
      const canvas = document.createElement('canvas');
      canvas.width = width;
      canvas.height = height;
      const g = canvas.getContext('2d');
      let seed = 2463534242;
      const random = () => {
        seed ^= seed << 13; seed >>>= 0;
        seed ^= seed >>> 17;
        seed ^= seed << 5; seed >>>= 0;
        return seed / 4294967296;
      };
      const sky = g.createLinearGradient(0, 0, 0, height);
      sky.addColorStop(0, '#f6a96b');
      sky.addColorStop(0.45, '#c7778a');
      sky.addColorStop(1, '#3b4f7a');
      g.fillStyle = sky;
      g.fillRect(0, 0, width, height);
      const sun = g.createRadialGradient(width * 0.68, height * 0.4, 8, width * 0.68, height * 0.4, height * 0.5);
      sun.addColorStop(0, 'rgba(255, 236, 200, 0.95)');
      sun.addColorStop(1, 'rgba(255, 236, 200, 0)');
      g.fillStyle = sun;
      g.fillRect(0, 0, width, height);
      ['#5a5f86', '#454c73', '#333a5c', '#252b45', '#1a1f33'].forEach((color, layer) => {
        g.beginPath();
        g.moveTo(0, height);
        const base = height * (0.5 + layer * 0.1);
        for (let x = 0; x <= width; x += 6) {
          const peak = layer === 0 ? Math.max(0, 260 - Math.abs(x - width * 0.6) * 0.8) : 0;
          g.lineTo(x, base - Math.sin(x / (90 + layer * 40) + layer) * 40 - Math.sin(x / 23 + layer * 3) * 10 - random() * 8 - peak);
        }
        g.lineTo(width, height);
        g.closePath();
        g.fillStyle = color;
        g.fill();
      });
      // Film grain gives the JPEG a photo's entropy, and so a photo's size.
      const pixels = g.getImageData(0, 0, width, height);
      for (let i = 0; i < pixels.data.length; i += 4) {
        const noise = (random() - 0.5) * 22;
        pixels.data[i] += noise;
        pixels.data[i + 1] += noise;
        pixels.data[i + 2] += noise;
      }
      g.putImageData(pixels, 0, 0);
      return canvas.toDataURL('image/jpeg', 0.8);
    }, { width: 1600, height: 1000 });
    return Buffer.from(url.slice(url.indexOf(',') + 1), 'base64');
  } finally {
    await page.close();
  }
}

/**
 * Serve fixtures at `/<key>/`. HTML is rendered and Brotli-5 compressed per
 * request with `no-store`; scripts, CSS, packs, and the image are immutable
 * static files (Brotli 11). `?hold=<ms>` adds an image that holds the window
 * `load` event for that long. Holds must be whole milliseconds from 0 to 5000.
 */
export async function serve(fixtures, state, image) {
  const server = createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://127.0.0.1');
    const [key, ...rest] = url.pathname.split('/').filter(Boolean);
    const fixture = fixtures.get(key);
    if (!fixture) return response.writeHead(404).end();
    const path = rest.join('/');
    const hold = path === '' || path === '__hold.gif'
      ? Number(url.searchParams.get(path === '' ? 'hold' : 'ms') ?? 0)
      : 0;
    if (!Number.isInteger(hold) || hold < 0 || hold > MAX_HOLD_MS) {
      return response.writeHead(400, {
        'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store',
      }).end('Load hold must be whole milliseconds between 0 and 5000.');
    }
    const acceptsBrotli = /\bbr\b/.test(String(request.headers['accept-encoding'] ?? ''));
    const send = (type, cache, body) => {
      const bytes = acceptsBrotli && body.br ? body.br : body.raw;
      response.writeHead(200, {
        'content-type': type, 'cache-control': cache, vary: 'accept-encoding', 'content-length': bytes.length,
        ...(acceptsBrotli && body.br ? { 'content-encoding': 'br' } : {}),
      });
      response.end(bytes);
    };
    if (path === '') {
      let html = fixture.protocol.render(state).toString();
      if (hold > 0) {
        html = html.replace('</body>', `<img src="__hold.gif?ms=${hold}" alt="" width="1" height="1" style="position:absolute;opacity:0"></body>`);
      }
      const raw = Buffer.from(html);
      return send('text/html; charset=utf-8', 'no-store', { raw, br: brotli(raw, 5) });
    }
    if (path === '__hold.gif') {
      const timer = setTimeout(() => send('image/gif', 'no-store', { raw: PIXEL }), hold);
      response.once('close', () => clearTimeout(timer));
      return undefined;
    }
    if (path === 'bg.jpg') return send('image/jpeg', IMMUTABLE, { raw: image });
    const file = fixture.files.get(path);
    if (file) return send(file.type, IMMUTABLE, file.body);
    return response.writeHead(404).end();
  });
  server.keepAliveTimeout = 30_000;
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close() {
      server.close();
      server.closeAllConnections();
    },
  };
}
