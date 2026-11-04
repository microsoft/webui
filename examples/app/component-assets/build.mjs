// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { writeFileSync } from 'node:fs';
import { runWebUIBuild } from '../../build-client.mjs';

const { client } = await runWebUIBuild({
  plugin: 'webui',
  theme: '@microsoft/webui-examples-theme',
  componentAssetRoots: ['lazy-panel', 'secondary-panel'],
  metafilePath: 'dist/component-assets.meta.json',
  clientOptions: { define: { __WEBUI_DEV__: 'false' } },
});
writeFileSync('dist/client-metafile.json', `${JSON.stringify(client.metafile, null, 2)}\n`);
