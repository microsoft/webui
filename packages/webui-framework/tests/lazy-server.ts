// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { RenderedFixture } from '@microsoft/webui-test-support/fixture-render';
import type { FixtureRequestContext } from '@microsoft/webui-test-support/fixture-server';

const initialState = {
  initialOpen: true, message: 'Server state', count: 2,
  items: [{ id: 'first', label: 'First row' }],
  expanded: true, advanced: false, loading: false, error: '', chosen: '',
  single: false,
  dialogPage: false,
};

export function lazyFixtureHandler(fixture: RenderedFixture | undefined): (context: FixtureRequestContext) => boolean {
  if (!fixture?.protocol || !fixture.render) throw new Error('Lazy fixture must retain its compiled protocol.');
  const protocol = fixture.protocol;
  const render = fixture.render;
  let revision = 0;
  return context => {
    const { url, req, res, send } = context;
    if (url.pathname === '/lazy-ui/fixture.html') {
      send(200, render(JSON.stringify({
        ...initialState,
        initialOpen: url.searchParams.get('initial') !== '0',
        single: url.searchParams.get('single') === '1',
        dialogPage: url.searchParams.get('dialog') === '1',
      })), 'text/html; charset=utf-8');
      return true;
    }
    if (url.pathname !== '/lazy-ui/resources' && url.pathname !== '/lazy-ui/state') return false;
    const owner = url.searchParams.get('owner') ?? '';
    const id = url.searchParams.get('id') ?? '';
    const header = (name: string): string => {
      const value = req.headers[name];
      if (Array.isArray(value)) throw new Error(`Repeated ${name} header`);
      return value ?? '';
    };
    let options: NonNullable<Parameters<typeof protocol.renderLazyResources>[2]>;
    try {
      options = {
        inventory: header('x-webui-inventory'),
        styleInventory: header('x-webui-style-inventory'),
        buildId: header('x-webui-build'),
      };
      if (url.pathname.endsWith('/resources')) {
        send(200, protocol.renderLazyResources(owner, id, options), 'application/json');
        return true;
      }
    } catch (error) {
      send(400, JSON.stringify({ error: String(error) }), 'application/json');
      return true;
    }
    const controller = new AbortController();
    const cancel = (): void => controller.abort(new DOMException('Client closed', 'AbortError'));
    res.once('close', cancel);
    const state = new Promise<Record<string, unknown>>((resolve, reject) => {
      const timer = setTimeout(() => {
        controller.signal.removeEventListener('abort', abort);
        if (url.searchParams.get('fail') === '1') {
          reject(new Error('Fixture data unavailable'));
          return;
        }
        resolve({
          message: `Fresh ${owner} ${++revision}`,
          count: revision,
          items: [{ id: 'fresh', label: 'Fresh row' }],
          expanded: true,
          advanced: url.searchParams.get('advanced') === '1',
        });
      }, Number(url.searchParams.get('delay') ?? 80));
      const abort = (): void => { clearTimeout(timer); reject(controller.signal.reason); };
      controller.signal.addEventListener('abort', abort, { once: true });
    });
    // State work has already started; an early metadata error still owns cancellation.
    void state.catch(() => {});
    void (async () => {
      try {
        const metadata = protocol.prepareLazyResources(owner, id, options);
        res.writeHead(200, { 'Content-Type': 'application/x-ndjson' });
        res.write(metadata.json + '\n');
        const value = await state;
        if (controller.signal.aborted) return;
        res.end(protocol.renderLazyPartial(value, owner, id, metadata.options) + '\n');
      } catch (error) {
        controller.abort(error);
        if (res.destroyed) return;
        if (!res.headersSent) send(400, JSON.stringify({ error: String(error) }), 'application/json');
        else res.end(JSON.stringify({ error: { code: 'fixture-state-failed', message: String(error) } }) + '\n');
      } finally {
        res.removeListener('close', cancel);
      }
    })();
    return true;
  };
}
