// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';

const initial = JSON.parse(readFileSync(new URL('../data/state.json', import.meta.url), 'utf8'));

function activityState(revision) {
  return {
    activitySummary: 'A quick look at this sample workspace.',
    activityItems: [
      { title: 'Design notes', description: 'Added a decision about keeping the first view small.', action: 'Edited' },
      { title: 'Release checklist', description: 'Reviewed keyboard access and loading states.', action: 'Reviewed' },
      { title: 'Reading queue', description: 'Saved a reference for a later pass.', action: 'Saved' },
    ],
    activityStamp: `Fresh state · request ${revision}`,
  };
}

function settingsState(revision) {
  return {
    layoutOptions: [
      { value: 'comfortable', label: 'Comfortable' },
      { value: 'compact', label: 'Compact' },
    ],
    settingsNote: 'These preferences affect this page only. The sample API does not store them.',
    settingsStamp: `Fresh state · request ${revision}`,
  };
}

function json(response, status, value) {
  response.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Cache-Control': 'no-store',
  });
  response.end(JSON.stringify(value));
}

/** The API returns state only; the Rust host owns protocol, inventory, and NDJSON. */
export function createStateServer() {
  let revision = 0;
  return createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://127.0.0.1');
    if (request.method !== 'GET') {
      response.setHeader('Allow', 'GET');
      json(response, 405, { error: 'This sample API is read-only.' });
      return;
    }
    if (url.pathname === '/health') {
      json(response, 200, { ready: true });
      return;
    }
    if (url.pathname === '/' || url.pathname === '/index.html') {
      const state = { ...initial };
      const open = url.searchParams.get('open');
      if (open === 'popover') Object.assign(state, activityState(++revision), { activityReady: true });
      if (open === 'dialog') Object.assign(state, settingsState(++revision), { settingsReady: true });
      json(response, 200, { state });
      return;
    }
    if (url.pathname !== '/_webui/deferred/state') {
      json(response, 404, { error: 'Unknown sample state endpoint.' });
      return;
    }
    const region = url.searchParams.get('id');
    const owner = url.searchParams.get('owner');
    const validRegion = region === 'activity'
      ? owner === 'deferred-popover'
      : region === 'settings' && owner === 'deferred-dialog';
    if (!validRegion) {
      json(response, 404, { error: 'Unknown deferred region.' });
      return;
    }
    const controller = new AbortController();
    const disconnect = () => controller.abort();
    response.once('close', disconnect);
    void (async () => {
      try {
        if (url.searchParams.get('slow') === '1') {
          await delay(600, undefined, { signal: controller.signal });
        }
        if (controller.signal.aborted) return;
        const state = region === 'activity' ? activityState(++revision) : settingsState(++revision);
        json(response, 200, { state });
      } catch (error) {
        if (!controller.signal.aborted) {
          console.error('Deferred sample state failed:', error);
          json(response, 500, { error: 'Sample state could not be loaded.' });
        }
      } finally {
        response.removeListener('close', disconnect);
      }
    })();
  });
}
