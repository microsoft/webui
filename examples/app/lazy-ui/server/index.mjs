// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createStateServer } from './api.mjs';

const argument = process.argv.indexOf('--port');
const port = Number(argument < 0 ? process.env.PORT ?? 3021 : process.argv[argument + 1]);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error('Pass a valid --port between 1 and 65535.');
}

const server = createStateServer();
server.on('error', error => {
  console.error('Lazy example API failed:', error);
  process.exitCode = 1;
});
server.listen(port, '127.0.0.1', () => {
  console.log(`Lazy example state API: http://127.0.0.1:${port}`);
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    server.close();
    server.closeAllConnections();
  });
}
