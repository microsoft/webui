// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { templateNeedsStaticHost } from './template-roots.js';
import type { TemplateMeta } from './template-types.js';

let generation = 0;
let installed = false;
let pending: Promise<void> | undefined;

/** Share one in-flight module request, without keeping its fulfilled promise. */
export function loadTemplateHostRuntime(): Promise<void> | undefined {
  if (installed) return;
  if (!pending) {
    const current = generation;
    pending = import('./static-host.js').then(({ installTemplateElementRuntime }) => {
      if (current !== generation) {
        throw new Error('[WebUI] Template-host activation was abandoned.');
      }
      installTemplateElementRuntime();
      installed = true;
      pending = undefined;
    });
  }
  return pending;
}

/** Join a navigation's existing readiness barrier when it needs dormant hosts. */
export function prepareTemplateDefinitions(
  templates: Record<string, TemplateMeta>,
  names: readonly string[],
): Promise<void> | undefined {
  const exclusions = window.__webui?.templateHostExclusions;
  for (let i = 0; i < names.length; i++) {
    const tag = names[i];
    if (
      templateNeedsStaticHost(templates[tag]) &&
      !exclusions?.has(tag) &&
      !customElements.get(tag)
    ) {
      return loadTemplateHostRuntime();
    }
  }
}

/** Invalidate uncancellable imports when the stream abandons its activation. */
export function invalidateTemplateHostRuntime(): void {
  generation++;
  pending = undefined;
}

/** Restore loader state between isolated pipeline tests. */
export function resetTemplateHostRuntimeForTests(): void {
  invalidateTemplateHostRuntime();
  installed = false;
}
