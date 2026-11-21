// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';
import { jsonTransport } from '../src/transport.js';

const router = new WebUIRouter();
const loadPage = () => import('./condition-routes-page.js');
let clickAt: number | undefined;
let pendingPage: ReturnType<typeof loadPage> | undefined;
const benchmark = {
  active: () => router.activeComponent,
  cold: undefined as { elapsed: number; rows: number } | undefined,
};

window.addEventListener('click', event => {
  if (!(event.target instanceof Element) || !event.target.closest('#open-comparisons')) return;
  clickAt = performance.now();
  pendingPage = loadPage();
}, { capture: true });

window.addEventListener('webui:route:navigated', () => {
  if (router.activeComponent !== 'bench-comparison-page' || clickAt === undefined || !pendingPage) return;
  const start = clickAt;
  void pendingPage.then(module => module.measureInteraction(start)).then(result => {
    benchmark.cold = result;
  }).catch(reportError);
});

startRuntime();
router.start({
  loaders: {
    'bench-comparison-page': loadPage,
    'bench-comparison-row': loadPage,
  },
  transport: {
    ...jsonTransport,
    async read(response, signal) {
      try {
        const body = await jsonTransport.read(response, signal);
        if (new URL(response.url).pathname === '/comparisons') {
          const module = await (pendingPage ?? loadPage());
          signal.throwIfAborted();
          module.prepare(body.data);
        }
        return body;
      } catch (error) {
        reportError(error);
        throw error;
      }
    },
  },
});

Object.assign(window, { conditionRouteBenchmark: benchmark });
