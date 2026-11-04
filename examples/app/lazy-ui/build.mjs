// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { runWebUIBuild } from '../../build-client.mjs';

await runWebUIBuild({
  plugin: 'webui',
  theme: '@microsoft/webui-examples-theme',
});
