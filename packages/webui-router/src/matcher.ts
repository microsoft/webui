// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Iterative path template matching — no regex.
 *
 * Supports `:param`, `:param?` (optional), and `*splat` segments.
 */

/** A single parsed segment from a path template. */
export type Segment =
  | { type: 'literal'; value: string }
  | { type: 'param'; name: string }
  | { type: 'optional'; name: string }
  | { type: 'splat'; name: string };

/** Result of a successful path match. */
export interface PathMatch {
  params: Record<string, string>;
}

/** Parse a path template string into typed segments. */
export function parseTemplate(path: string): Segment[] {
  return path
    .split('/')
    .filter(Boolean)
    .map((seg): Segment => {
      if (seg.startsWith(':')) {
        const raw = seg.slice(1);
        if (raw.endsWith('?')) {
          return { type: 'optional', name: raw.slice(0, -1) };
        }
        return { type: 'param', name: raw };
      }
      if (seg.startsWith('*')) {
        return { type: 'splat', name: seg.slice(1) || 'rest' };
      }
      return { type: 'literal', value: seg };
    });
}

// Module-level cache: template strings are static, avoid re-parsing.
const templateCache = new Map<string, Segment[]>();

function getCachedSegments(template: string): Segment[] {
  let segs = templateCache.get(template);
  if (!segs) {
    segs = parseTemplate(template);
    templateCache.set(template, segs);
  }
  return segs;
}

/**
 * Try to match a request path against a single route template.
 *
 * Returns bound parameters on success, `null` on failure.
 */
export function matchPath(
  template: string,
  requestPath: string,
  exact: boolean,
): PathMatch | null {
  const segs = getCachedSegments(template);
  const parts = requestPath.split('/').filter(Boolean);
  const params: Record<string, string> = {};
  let pi = 0;

  for (let si = 0; si < segs.length; si++) {
    const seg = segs[si];
    switch (seg.type) {
      case 'literal':
        if (pi >= parts.length || parts[pi] !== seg.value) return null;
        pi++;
        break;
      case 'param':
        if (pi >= parts.length) return null;
        params[seg.name] = decodeURIComponent(parts[pi++]);
        break;
      case 'optional':
        if (pi < parts.length) {
          params[seg.name] = decodeURIComponent(parts[pi++]);
        }
        break;
      case 'splat':
        params[seg.name] = parts.slice(pi).map(decodeURIComponent).join('/');
        pi = parts.length;
        break;
    }
  }

  if (exact && pi < parts.length) return null;
  return { params };
}

/**
 * Count the number of literal (non-param) segments in a template.
 * Used to rank matches by specificity.
 */
export function specificity(template: string): number {
  return getCachedSegments(template).filter(s => s.type === 'literal').length;
}
