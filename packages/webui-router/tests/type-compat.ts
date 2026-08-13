// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { TemplateMeta } from '../../webui-framework/src/index.js';
import '../../webui-framework/src/index.js';
import '../src/index.js';

declare const runtime: NonNullable<Window['__webui']>;

const chain: unknown[] | undefined = runtime.chain;
const templates: Record<string, TemplateMeta> | undefined = runtime.templates;

void chain;
void templates;
