// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import path from "node:path";

// GNU tar interprets a colon before any slash as a remote host. Keep drive
// letters out of -f, and prefix even a colon-containing basename with ./.
export function tarInvocation(archive, flags, ...entries) {
  const absolute = path.resolve(archive);
  return {
    args: [flags, `./${path.basename(absolute)}`, ...entries],
    cwd: path.dirname(absolute),
  };
}
