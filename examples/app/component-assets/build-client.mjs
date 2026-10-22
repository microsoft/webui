// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { writeFileSync } from 'node:fs';
import { runWebUIClientBuild } from '../../build-client.mjs';

const result = await runWebUIClientBuild({
  define: { __WEBUI_DEV__: 'false' },
});
writeFileSync(
  'dist/client-metafile.json',
  `${JSON.stringify(result.metafile, null, 2)}\n`,
);
