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
  searchPage: false,
  suggesting: false,
};

const PACK_PREFIX = '/lazy-ui/_webui/packs/';

/** Serve the page, its build-time packs as static files, and app-owned JSON state. */
export function lazyFixtureHandler(fixture: RenderedFixture | undefined): (context: FixtureRequestContext) => boolean {
  if (!fixture?.render || !fixture.packs) throw new Error('Lazy fixture must retain its renderer and packs.');
  const render = fixture.render;
  const packs = fixture.packs;
  let revision = 0;
  return context => {
    const { url, res, send } = context;
    if (url.pathname === '/lazy-ui/fixture.html') {
      send(200, render(JSON.stringify({
        ...initialState,
        initialOpen: url.searchParams.get('initial') !== '0',
        single: url.searchParams.get('single') === '1',
        dialogPage: url.searchParams.get('dialog') === '1',
        searchPage: url.searchParams.get('search') === '1',
      })), 'text/html; charset=utf-8');
      return true;
    }
    if (url.pathname.startsWith(PACK_PREFIX)) {
      const pack = packs.get(url.pathname.slice(PACK_PREFIX.length));
      if (pack === undefined) send(404, 'Unknown pack');
      else send(200, pack, 'application/json');
      return true;
    }
    if (url.pathname !== '/lazy-ui/state') return false;
    const owner = url.searchParams.get('owner') ?? '';
    const timer = setTimeout(() => {
      if (url.searchParams.get('fail') === '1') {
        send(503, JSON.stringify({ error: 'Fixture data unavailable' }), 'application/json');
        return;
      }
      send(200, JSON.stringify({
        message: `Fresh ${owner} ${++revision}`,
        count: revision,
        items: [{ id: 'fresh', label: 'Fresh row' }],
        expanded: true,
        advanced: url.searchParams.get('advanced') === '1',
      }), 'application/json');
    }, Number(url.searchParams.get('delay') ?? 80));
    res.once('close', () => clearTimeout(timer));
    return true;
  };
}
