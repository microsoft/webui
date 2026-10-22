// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium } from '@playwright/test';
import type { Page } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const publicRoot = path.resolve(import.meta.dirname, '../../../docs/.webui-press/public');

async function rasterGeometry(page: Page, svg: string, size: number) {
  return page.evaluate(async ({ svg, size }) => {
    const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
    try {
      const image = new Image();
      image.src = url;
      await image.decode();
      const canvas = document.createElement('canvas');
      canvas.width = size;
      canvas.height = size;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('Canvas unavailable');
      context.drawImage(image, 0, 0, size, size);
      const pixels = context.getImageData(0, 0, size, size).data;
      const seen = new Uint8Array(size * size);
      const stack = new Uint32Array(size * size);
      const components: number[] = [];
      let touchesEdge = false;
      for (let index = 0; index < size * size; index++) {
        if (pixels[index * 4 + 3] < 128 || seen[index]) continue;
        let length = 1;
        stack[0] = index;
        seen[index] = 1;
        let count = 0;
        while (length) {
          const current = stack[--length];
          const x = current % size;
          const y = Math.floor(current / size);
          touchesEdge ||= x === 0 || y === 0 || x === size - 1 || y === size - 1;
          count++;
          for (let dy = -1; dy <= 1; dy++) {
            for (let dx = -1; dx <= 1; dx++) {
              const nx = x + dx;
              const ny = y + dy;
              if (nx < 0 || ny < 0 || nx >= size || ny >= size) continue;
              const next = ny * size + nx;
              if (!seen[next] && pixels[next * 4 + 3] >= 128) {
                seen[next] = 1;
                stack[length++] = next;
              }
            }
          }
        }
        components.push(count);
      }
      return {
        components: components.sort((left, right) => right - left),
        apertureOpen: pixels[(Math.floor(size / 2) * size + Math.floor(size / 2)) * 4 + 3] === 0,
        touchesEdge,
      };
    } finally {
      URL.revokeObjectURL(url);
    }
  }, { svg, size });
}

test('the WebUI mark retains its aperture and live cell at favicon sizes and in monochrome', async t => {
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  for (const size of [16, 32, 128]) {
    const color = await rasterGeometry(page, fs.readFileSync(path.join(publicRoot, 'logo.svg'), 'utf8'), size);
    assert.equal(color.components.length, 2, `${size}px: keep the prepared form and live cell separate.`);
    assert.equal(color.components[1], (size / 8) ** 2, `${size}px: preserve the fitted cell's physical size.`);
    assert.equal(color.apertureOpen, true);
    assert.equal(color.touchesEdge, false);
    for (const variant of ['monochrome.svg', 'inverse.svg']) {
      const monochrome = await rasterGeometry(page, fs.readFileSync(path.join(publicRoot, 'brand', variant), 'utf8'), size);
      assert.deepEqual(monochrome, color, `${variant} must preserve the color mark's geometry at ${size}px.`);
    }
  }
});

test('avatar, wordmark and README header preserve the canonical mark paths', async t => {
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  const paths = async (file: string) => page.evaluate(svg => {
    const document = new DOMParser().parseFromString(svg, 'image/svg+xml');
    if (document.querySelector('parsererror')) throw new Error('Invalid SVG');
    return [...document.querySelectorAll('path')].map(element => element.getAttribute('d'));
  }, fs.readFileSync(path.join(publicRoot, file), 'utf8'));
  const canonical = await paths('logo.svg');
  assert.equal(canonical.length, 2);
  for (const file of ['brand/avatar.svg', 'brand/wordmark.svg', 'brand/readme-header.svg']) {
    assert.deepEqual(await paths(file), canonical, `${file} must use the canonical geometry.`);
  }
});
