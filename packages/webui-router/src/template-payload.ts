// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export class TrustedTypesPayloadError extends Error {}

export function validateTemplatePayload(templates: Record<string, unknown> | undefined): boolean {
  if (!templates) return false;
  let hasStrings = false;
  for (const tag of Object.keys(templates)) {
    const template = templates[tag];
    if (typeof template !== 'string') continue;
    if (!template.startsWith('<')) {
      throw new Error(`[Router] Unsupported executable template payload for ${tag}.`);
    }
    hasStrings = true;
  }
  return hasStrings;
}

export function prepareTemplatePayload(
  templates: Record<string, unknown> | undefined,
  functions: Record<string, string> | undefined,
  nonce: string,
): { script?: HTMLScriptElement; content?: DocumentFragment } | undefined {
  const hasStrings = validateTemplatePayload(templates);
  const script = prepareConditionScript(functions, nonce);
  let content: DocumentFragment | undefined;
  if (hasStrings && templates) {
    for (const tag of Object.keys(templates)) {
      const template = templates[tag];
      if (typeof template !== 'string') continue;
      const temp = document.createElement('div');
      try {
        temp.innerHTML = template;
      } catch (cause) {
        if (!(cause instanceof TypeError)) throw cause;
        throw new TrustedTypesPayloadError(
          '[Router] Trusted Types rejected FAST/string templates. Use full document navigation instead of client-side partial navigation.',
          { cause },
        );
      }
      content ??= document.createDocumentFragment();
      while (temp.firstChild) content.appendChild(temp.firstChild);
    }
  }
  return script || content ? { script, content } : undefined;
}

function prepareConditionScript(
  functions: Record<string, string> | undefined,
  nonce: string,
): HTMLScriptElement | undefined {
  if (!functions) return undefined;
  const tags = Object.keys(functions);
  if (tags.length === 0) return undefined;
  let body = 'var w=(window.__webui||(window.__webui={}));var f=w.templateFns||(w.templateFns={});';
  for (const tag of tags) {
    const source = functions[tag];
    if (!source) continue;
    body += 'f[';
    body += JSON.stringify(tag);
    body += ']=';
    body += source;
    body += ';';
  }
  const script = document.createElement('script');
  if (nonce) script.nonce = nonce;
  // Stage the actual native sink before publishing any response resources.
  // Browser support or framework policy creation does not imply enforcement.
  try {
    script.textContent = `(function(){${body}})();`;
  } catch (cause) {
    if (!(cause instanceof TypeError)) throw cause;
    throw new TrustedTypesPayloadError(
      '[Router] Trusted Types rejected condition-source strings. Use full document navigation instead of client-side partial navigation.',
      { cause },
    );
  }
  return script;
}
