// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const proto = fileURLToPath(new URL('../../../crates/webui-desktop/proto/', import.meta.url));
mkdirSync(`${root}src/generated`, { recursive: true });
execFileSync('protoc', [
  `--plugin=protoc-gen-ts_proto=${root}node_modules/.bin/protoc-gen-ts_proto`,
  `--ts_proto_out=${root}src/generated`,
  '--ts_proto_opt=forceLong=bigint,outputServices=none,oneof=unions-value,useOptionals=messages,outputJsonMethods=false,outputPartialMethods=false,useExactTypes=true,esModuleInterop=true',
  `--proto_path=${proto}`, `${proto}webui_desktop.proto`,
], { stdio: 'inherit' });
