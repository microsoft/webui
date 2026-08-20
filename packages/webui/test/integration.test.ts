// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Integration test for the @microsoft/webui npm package.
 * Uses the built-in Node.js test runner.
 */

import { describe, test, before, after } from 'node:test';
import { strict as assert } from 'node:assert';
import {
  build,
  inspect,
  Protocol,
} from '@microsoft/webui';
import type {
  BoundaryDescriptor,
  ComponentTemplatesResponse,
  StreamStep,
} from '@microsoft/webui';
import { existsSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { createServer, get } from 'node:http';
import type { IncomingMessage, ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { once } from 'node:events';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { analyzeMetafile, type Metafile } from 'esbuild';

let appDir: string;

before(() => {
  const addonName = process.platform === 'win32'
    ? 'webui_node.dll'
    : process.platform === 'darwin'
      ? 'libwebui_node.dylib'
      : 'libwebui_node.so';
  const workspaceAddon = join(process.cwd(), '..', '..', 'target', 'debug', addonName);
  if (!process.env.WEBUI_ADDON_PATH && existsSync(workspaceAddon)) {
    process.env.WEBUI_ADDON_PATH = workspaceAddon;
  }

  appDir = mkdtempSync(join(tmpdir(), 'webui-test-'));

  writeFileSync(join(appDir, 'index.html'), `
<!DOCTYPE html>
<html>
<body>
  <h1>Hello, {{name}}!</h1>
  <for each="item in items">
    <p>{{item}}</p>
  </for>
  <if condition="show">
    <footer>Visible</footer>
  </if>
</body>
</html>
`);

  writeFileSync(join(appDir, 'my-card.html'), '<div class="card">Content</div>');
  writeFileSync(
    join(appDir, 'shadow-card.html'),
    '<template shadowrootmode="open"><div><slot></slot></div></template>',
  );
  writeFileSync(join(appDir, 'my-card.css'), '.card { border: 1px solid #ccc; }');
  writeFileSync(join(appDir, 'index2.html'), '<my-card>Hello</my-card>');
  writeFileSync(join(appDir, 'app-shell.html'), '<div>{{name}}</div>');
  writeFileSync(join(appDir, 'app-shell.ts'), 'export {};');
  writeFileSync(join(appDir, 'lazy-panel.html'), '<p>{{title}}</p>');
  writeFileSync(join(appDir, 'lazy-panel.ts'), 'export {};');
  writeFileSync(join(appDir, 'index3.html'), '<app-shell></app-shell>');

  writeFileSync(join(appDir, 'stream-item.html'), '<p class="item">{{label}}</p>');
  writeFileSync(join(appDir, 'stream-item.ts'), 'export {};');
  writeFileSync(join(appDir, 'index-stream.html'), `
<!DOCTYPE html>
<html>
<head><script type="module" async src="./index.js"></script></head>
<body>
  <boundary name="first">
    <stream-item label="{{firstLabel}}"></stream-item>
  </boundary>
  <p>between</p>
  <boundary name="second">
    <stream-item label="{{secondLabel}}"></stream-item>
  </boundary>
</body>
</html>
`);
  writeFileSync(join(appDir, 'index-stream-keys.html'), `
<!DOCTYPE html>
<html>
<head><script type="module" async src="./index.js"></script></head>
<body>
  <boundary name="string-key" key="{{stringId}}">
    <stream-item label="{{firstLabel}}"></stream-item>
  </boundary>
  <p>between</p>
  <boundary name="number-key" key="{{numberId}}">
    <stream-item label="{{secondLabel}}"></stream-item>
  </boundary>
  <footer>tail</footer>
</body>
</html>
`);
  writeFileSync(join(appDir, 'index-stream-empty.html'), `
<!DOCTYPE html>
<html>
<head><script type="module" async src="./index.js"></script></head>
<body><p>boundary-free</p></body>
</html>
`);
});

after(() => {
  rmSync(appDir, { recursive: true, force: true });
});

describe('build', () => {
  test('returns protocol and stats', () => {
    const result = build({ appDir });
    assert.ok(result.protocol.length > 0);
    assert.ok(result.stats.fragmentCount > 0);
    assert.ok(result.stats.durationMs >= 0);
    assert.ok(result.stats.protocolSizeBytes > 0);
    assert.ok(result.stats.componentCount >= 0);
    assert.ok(Array.isArray(result.warnings));
  });

  test('emits CSS files for used components', () => {
    const result = build({ appDir, entry: 'index2.html', css: 'link' });
    assert.ok(result.stats.componentCount > 0);
    assert.equal(result.cssFiles.length, 2); // [filename, content]
    assert.equal(result.stats.cssFileCount, 1);
  });

  test('defaults to Shadow and supports mixed Light mode', () => {
    const defaultShadow = build({ appDir, entry: 'index2.html' });
    assert.ok(inspect(defaultShadow.protocol).includes('shadowrootmode'));

    const light = build({ appDir, entry: 'index2.html', dom: 'light' });
    assert.ok(!inspect(light.protocol).includes('shadowrootmode'));

    writeFileSync(join(appDir, 'index-shadow.html'), '<shadow-card>slot</shadow-card>');
    const shadow = build({ appDir, entry: 'index-shadow.html', dom: 'light' });
    assert.ok(inspect(shadow.protocol).includes('shadowrootmode'));
  });

  test('BuildOptions exposes the DOM strategy union', () => {
    const options: import('@microsoft/webui').BuildOptions = { appDir, dom: 'light' };
    assert.equal(options.dom, 'light');
    // @ts-expect-error only open Shadow or global Light strategies are supported.
    const invalid: import('@microsoft/webui').BuildOptions = { appDir, dom: 'closed' };
    assert.equal(invalid.appDir, appDir);
  });

  test('emits static component asset files and an analyzable metafile', async () => {
    const result = build({
      appDir,
      entry: 'index3.html',
      plugin: 'webui',
      componentAssetRoots: ['lazy-panel'],
      metafile: true,
    });
    assert.equal(result.componentAssetFiles.length, 2); // [filename, content]
    assert.equal(result.componentAssetFiles[0], 'lazy-panel.webui.js');
    assert.match(result.componentAssetFiles[1], /webui-component-asset/);
    assert.ok(result.metafile);
    const parsedMetafile: Metafile = JSON.parse(result.metafile);
    const analysis = await analyzeMetafile(parsedMetafile);
    assert.match(analysis, /lazy-panel\.webui\.js/);
  });

  test('throws on missing appDir', () => {
    assert.throws(() => build({ appDir: '/nonexistent' }));
  });

  test('throws on invalid css mode', () => {
    assert.throws(() => build({ appDir, css: 'bogus' as 'link' }));
  });
});

describe('inspect', () => {
  test('returns valid JSON with fragments', () => {
    const result = build({ appDir });
    const json = inspect(result.protocol);
    const parsed = JSON.parse(json);
    assert.ok(parsed.fragments);
    assert.ok(parsed.fragments['index.html']);
  });
});

describe('render', () => {
  test('substitutes signals', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const buffer = protocol.render({ name: 'WebUI', items: ['a', 'b'], show: true });
    assert.ok(Buffer.isBuffer(buffer));
    assert.ok(buffer.toString('utf8').includes('Hello, WebUI!'));
  });

  test('expands for-loop', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const html = protocol
      .render({ name: 'X', items: ['a', 'b'], show: false })
      .toString('utf8');
    assert.ok(html.includes('<p>a</p>'));
    assert.ok(html.includes('<p>b</p>'));
  });

  test('includes if-true block', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const html = protocol.render({ name: 'X', items: [], show: true }).toString('utf8');
    assert.ok(html.includes('<footer>Visible</footer>'));
  });

  test('excludes if-false block', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const html = protocol.render({ name: 'X', items: [], show: false }).toString('utf8');
    assert.ok(!html.includes('<footer>'));
  });

  test('reuses one protocol for object and JSON string state', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const objectHtml = protocol
      .render({ name: 'Object', items: [], show: false })
      .toString('utf8');
    const jsonHtml = protocol
      .render(JSON.stringify({ name: 'JSON', items: [], show: false }))
      .toString('utf8');
    assert.ok(objectHtml.includes('Hello, Object!'));
    assert.ok(jsonHtml.includes('Hello, JSON!'));
  });

  test('owns decoded state independently of source buffer mutations', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const state = { name: 'Cache', items: [], show: true };
    const initialHtml = protocol.render(state).toString('utf8');
    assert.ok(initialHtml.includes('<footer>Visible</footer>'));

    const offset = result.protocol.indexOf('Visible');
    assert.ok(offset >= 0);
    result.protocol.write('Altered', offset, 'utf8');

    const existingHtml = protocol.render(state).toString('utf8');
    assert.ok(existingHtml.includes('<footer>Visible</footer>'));

    const updatedHtml = new Protocol(result.protocol).render(state).toString('utf8');
    assert.ok(updatedHtml.includes('<footer>Altered</footer>'));
  });

  test('rejects an unknown protocol plugin at construction', () => {
    const result = build({ appDir });
    assert.throws(
      () => new Protocol(result.protocol, { plugin: '' }),
      /Unknown plugin/,
    );
  });
});

describe('renderStream', () => {
  test('streams chunks via callback', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const chunks: string[] = [];
    protocol.renderStream({ name: 'Stream', items: ['x'], show: false }, (chunk) => {
      chunks.push(chunk);
    });
    assert.ok(chunks.length > 0);
    assert.ok(chunks.join('').includes('Hello, Stream!'));
  });

  test('accepts callback return values', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const chunks: string[] = [];
    protocol.renderStream({ name: 'Return', items: [], show: false }, (chunk) => {
      chunks.push(chunk);
      return false;
    });
    assert.ok(chunks.join('').includes('Hello, Return!'));
  });

  test('propagates callback exceptions', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    assert.throws(
      () =>
        protocol.renderStream({ name: 'Throw', items: [], show: false }, () => {
          throw new Error('chunk callback failed');
        }),
      /chunk callback failed/,
    );
  });
});

describe('streamResponse', () => {
  const streamOptions = { entry: 'index-stream.html', requestPath: '/' };

  function streamingProtocol(entry = 'index-stream.html'): Protocol {
    return new Protocol(
      build({ appDir, entry, plugin: 'webui' }).protocol,
    );
  }

  function boundaryOf(step: StreamStep): BoundaryDescriptor {
    assert.ok(step.boundary);
    return step.boundary;
  }

  test('discovers runtime boundary descriptors from start', () => {
    const session = streamingProtocol().streamResponse(streamOptions);
    const step = session.start({});
    const boundary = boundaryOf(step);

    assert.ok(Buffer.isBuffer(step.bytes));
    assert.equal(step.done, false);
    assert.equal(boundary.instanceId, 0);
    assert.equal(typeof boundary.declarationId, 'number');
    assert.equal(boundary.owner, 'index-stream.html');
    assert.equal(boundary.name, 'first');
    assert.equal(boundary.key, undefined);
  });

  test('resume stops at the checkpoint and advance discovers the next boundary', () => {
    const session = streamingProtocol().streamResponse(streamOptions);
    const start = session.start({});
    const first = boundaryOf(start);
    const resumedFirst = session.resume(first.instanceId, { firstLabel: 'alpha' });
    assert.ok(Buffer.isBuffer(resumedFirst.bytes));
    assert.equal(resumedFirst.done, false);
    assert.equal(resumedFirst.boundary, undefined);
    assert.match(resumedFirst.bytes.toString('utf8'), /alpha/);
    assert.doesNotMatch(resumedFirst.bytes.toString('utf8'), /second/);

    const next = session.advance();
    const second = boundaryOf(next);

    assert.ok(Buffer.isBuffer(next.bytes));
    assert.equal(next.done, false);
    assert.equal(second.instanceId, 1);
    assert.equal(second.name, 'second');
    assert.match(next.bytes.toString('utf8'), /between/);
    assert.doesNotMatch(next.bytes.toString('utf8'), /beta/);

    const resumedSecond = session.resume(second.instanceId, { secondLabel: 'beta' });
    assert.ok(Buffer.isBuffer(resumedSecond.bytes));
    assert.equal(resumedSecond.done, false);
    assert.equal(resumedSecond.boundary, undefined);
    assert.match(resumedSecond.bytes.toString('utf8'), /beta/);
    assert.doesNotMatch(resumedSecond.bytes.toString('utf8'), /<\/html>/);

    const done = session.advance();
    assert.ok(Buffer.isBuffer(done.bytes));
    assert.equal(done.done, true);
    assert.equal(done.boundary, undefined);

    const html = Buffer.concat([
      start.bytes,
      resumedFirst.bytes,
      next.bytes,
      resumedSecond.bytes,
      done.bytes,
    ]).toString('utf8');
    assert.ok(html.includes('<!DOCTYPE html>'));
    assert.ok(html.includes('alpha'));
    assert.ok(html.includes('beta'));
    assert.equal(html.match(/class="item"/g)?.length, 2);
    assert.ok(html.includes('</html>'));
  });

  test('preserves string and number keys on static sibling occurrences', () => {
    const entry = 'index-stream-keys.html';
    const session = streamingProtocol(entry).streamResponse({
      entry,
      requestPath: '/',
    });
    const state = {
      stringId: 'alpha',
      firstLabel: 'first',
      numberId: 20,
      secondLabel: 'second',
    };

    const start = session.start(state);
    const first = boundaryOf(start);
    assert.equal(first.key, 'alpha');

    const resumedFirst = session.resume(first.instanceId, state);
    assert.equal(resumedFirst.done, false);
    assert.equal(resumedFirst.boundary, undefined);
    assert.doesNotMatch(resumedFirst.bytes.toString('utf8'), /between/);

    const next = session.advance();
    const second = boundaryOf(next);
    assert.equal(second.instanceId, 1);
    assert.notEqual(second.declarationId, first.declarationId);
    assert.equal(second.key, 20);
    assert.match(next.bytes.toString('utf8'), /between/);

    const resumedSecond = session.resume(second.instanceId, state);
    assert.equal(resumedSecond.done, false);
    assert.equal(resumedSecond.boundary, undefined);
    assert.doesNotMatch(resumedSecond.bytes.toString('utf8'), /tail/);
    const done = session.advance();
    assert.equal(done.done, true);
    assert.match(done.bytes.toString('utf8'), /tail/);
  });

  test('updates a committed updatable occurrence', () => {
    const session = streamingProtocol().streamResponse(streamOptions);
    const start = session.start({});
    const first = boundaryOf(start);
    const resumed = session.resume(
      first.instanceId,
      { firstLabel: 'alpha' },
      'updatable',
    );
    assert.equal(resumed.done, false);
    assert.equal(resumed.boundary, undefined);
    const update = session.update(first.instanceId, {
      firstLabel: 'alpha-2',
    });

    assert.ok(Buffer.isBuffer(update));
    assert.match(update.toString('utf8'), /alpha-2/);

    const next = session.advance();
    const second = boundaryOf(next);
    const resumedSecond = session.resume(second.instanceId, { secondLabel: 'beta' });
    assert.equal(resumedSecond.done, false);
    assert.equal(resumedSecond.boundary, undefined);
    const done = session.advance();
    assert.equal(done.done, true);
  });

  test('rejects advance before a boundary has been resumed', () => {
    const session = streamingProtocol().streamResponse(streamOptions);
    assert.throws(
      () => session.advance(),
      /start must be called before this operation/,
    );

    const start = session.start({});
    assert.throws(
      () => session.advance(),
      /there is no committed boundary to advance past/,
    );

    const first = boundaryOf(start);
    const resumed = session.resume(first.instanceId, { firstLabel: 'alpha' });
    assert.equal(resumed.done, false);
    assert.equal(resumed.boundary, undefined);
    assert.equal(boundaryOf(session.advance()).name, 'second');
  });

  test('start completes a boundary-free document', () => {
    const entry = 'index-stream-empty.html';
    const session = streamingProtocol(entry).streamResponse({
      entry,
      requestPath: '/',
    });
    const done = session.start({});

    assert.ok(Buffer.isBuffer(done.bytes));
    assert.equal(done.done, true);
    assert.equal(done.boundary, undefined);
    assert.match(done.bytes.toString('utf8'), /boundary-free/);
    assert.ok(done.bytes.toString('utf8').includes('</html>'));
  });

  test('does not expose legacy wrapper members', () => {
    const session = streamingProtocol().streamResponse(streamOptions);
    const members = session as unknown as Record<string, unknown>;
    assert.equal(members.boundary, undefined);
    assert.equal(members.boundaryCount, undefined);
    assert.equal(members.finished, undefined);
    assert.equal(members.writeShell, undefined);
    assert.equal(members.writeBoundary, undefined);
    assert.equal(members.finish, undefined);
  });
});

describe('streamResponse over node:http', () => {
  const streamOptions = { entry: 'index-stream.html', requestPath: '/' };

  /**
   * Proves the property the session API exists for: an in-process Node server
   * can hand bytes to the client while it is still producing the rest of the
   * body.
   *
   * The synchronisation is a barrier rather than a timer, so the test cannot
   * pass by accident. The server blocks after the first boundary until the
   * client confirms it received it; if anything buffered the response, that
   * handshake could never complete and the test would time out instead of
   * silently asserting nothing.
   */
  test('delivers early boundaries to a real client before the response ends', async () => {
    const protocol = new Protocol(
      build({ appDir, entry: 'index-stream.html', plugin: 'webui' }).protocol,
    );

    let releaseServer: () => void = () => {};
    const clientSawFirstBoundary = new Promise<void>((resolvePromise) => {
      releaseServer = resolvePromise;
    });

    let serverError: unknown;
    const server = createServer((_request, response) => {
      void (async () => {
        const session = protocol.streamResponse(streamOptions);

        response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
        let step = session.start({});
        await write(response, step.bytes);
        assert.ok(step.boundary);
        step = session.resume(step.boundary.instanceId, { firstLabel: 'alpha' });
        await write(response, step.bytes);
        assert.equal(step.done, false);
        assert.equal(step.boundary, undefined);

        // Only reached if the client already has the bytes above.
        await clientSawFirstBoundary;

        step = session.advance();
        assert.ok(step.boundary);
        await write(response, step.bytes);
        step = session.resume(step.boundary.instanceId, { secondLabel: 'beta' });
        assert.equal(step.done, false);
        assert.equal(step.boundary, undefined);
        await write(response, step.bytes);
        step = session.advance();
        assert.equal(step.done, true);
        response.end(step.bytes);
      })().catch((error: unknown) => {
        serverError = error;
        response.destroy();
        releaseServer();
      });
    });

    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    const { port } = server.address() as AddressInfo;

    try {
      const response = await new Promise<IncomingMessage>((resolvePromise, rejectPromise) => {
        get({ host: '127.0.0.1', port, path: '/' }, resolvePromise).on('error', rejectPromise);
      });
      response.setEncoding('utf8');

      let received = '';
      let released = false;
      let sawTailBeforeRelease = false;
      for await (const chunk of response) {
        received += chunk;
        if (!released && received.includes('alpha')) {
          // Latched: sampled exactly once, on the read that first sees the head.
          sawTailBeforeRelease = received.includes('beta');
          released = true;
          releaseServer();
        }
      }

      assert.equal(serverError, undefined);
      // The tail arrived only after the client acknowledged the head.
      assert.equal(sawTailBeforeRelease, false);
      assert.ok(received.includes('alpha'));
      assert.ok(received.includes('between'));
      assert.ok(received.includes('beta'));
      assert.ok(received.includes('</html>'));
    } finally {
      server.close();
      await once(server, 'close');
    }
  });

  /** Mirrors the host write helper the Node example documents. */
  async function write(response: ServerResponse, chunk: Buffer): Promise<void> {
    if (response.write(chunk)) return;
    // An aborted client never emits 'drain', and surfaces as 'close', not
    // 'error', so a bare drain wait would hang this test forever.
    await new Promise<void>((ok, fail) => {
      const done = (error?: Error): void => {
        response.off('drain', onDrain);
        response.off('close', onClose);
        if (error) fail(error);
        else ok();
      };
      const onDrain = (): void => done();
      const onClose = (): void => done(new Error('client disconnected'));
      response.once('drain', onDrain);
      response.once('close', onClose);
    });
  }
});

describe('renderComponentTemplates', () => {
  test('returns valid response shape', () => {
    const result = build({ appDir, entry: 'index2.html' });
    const protocol = new Protocol(result.protocol);
    const json = protocol.renderComponentTemplates(['my-card'], '');
    const parsed: ComponentTemplatesResponse = JSON.parse(json);
    assert.equal(typeof parsed.templates, 'object');
    assert.equal(typeof parsed.componentStyles, 'object');
    assert.equal(typeof parsed.templateFunctions, 'object');
    assert.equal(typeof parsed.inventory, 'string');
  });

  test('returns empty template maps for unknown component', () => {
    const result = build({ appDir });
    const protocol = new Protocol(result.protocol);
    const json = protocol.renderComponentTemplates(['nonexistent-widget'], '');
    const parsed: ComponentTemplatesResponse = JSON.parse(json);
    assert.deepEqual(parsed.templates, {});
    assert.deepEqual(parsed.templateFunctions, {});
    assert.deepEqual(parsed.componentStyles.resources, {});
    assert.deepEqual(parsed.componentStyles.closures, {});
  });
});

describe('renderPartial', () => {
  test('preserves full state when projection metadata is absent', () => {
    const result = build({ appDir, entry: 'index3.html', plugin: 'webui' });
    const protocol = new Protocol(result.protocol, { plugin: 'webui' });
    const json = protocol.renderPartial(
      '{"name":"Partial","serverOnly":"drop"}',
      'index3.html',
      '/',
      '',
    );
    const parsed = JSON.parse(json);
    assert.equal(parsed.state.name, 'Partial');
    assert.equal(parsed.state.serverOnly, 'drop');
  });
});
