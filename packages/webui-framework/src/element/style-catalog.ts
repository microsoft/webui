// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** One addressable component stylesheet definition. */
export type ComponentStyleResource = (
  | { kind: 'link'; href: string }
  | { kind: 'style'; css: string }
  | { kind: 'module'; specifier: string; css: string }
) & {
  /** Component resource IDs whose rules this bundled resource covers. */
  members?: string[];
};

/** Versioned stylesheet catalog shared by server and client registration paths. */
export interface ComponentStyles {
  version: 1;
  strategy: 'link' | 'style' | 'module';
  resources: Record<string, ComponentStyleResource>;
  closures: Record<string, string[]>;
}

/** Document-owned definitions shared by bootstrap and stylesheet installation. */
export interface DocumentCatalog {
  readonly resources: Map<string, ComponentStyleResource>;
  readonly closures: Map<string, readonly string[]>;
}

const catalogs = new WeakMap<Document, DocumentCatalog>();

/** Share stylesheet definitions without loading the DOM installation runtime. */
export function catalogFor(document: Document): DocumentCatalog {
  let catalog = catalogs.get(document);
  if (!catalog) {
    catalog = {
      resources: new Map(),
      closures: new Map(),
    };
    catalogs.set(document, catalog);
  }
  return catalog;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Compare prepared resources without allocating serialized copies. */
export function sameComponentStyleResource(
  current: ComponentStyleResource,
  next: ComponentStyleResource,
): boolean {
  if (current === next) return true;
  const currentMembers = current.members;
  const nextMembers = next.members;
  if (
    currentMembers?.length !== nextMembers?.length ||
    currentMembers?.some((member, index) => member !== nextMembers?.[index])
  ) {
    return false;
  }
  switch (current.kind) {
    case 'link':
      return next.kind === 'link' && current.href === next.href;
    case 'style':
      return next.kind === 'style' && current.css === next.css;
    case 'module':
      return next.kind === 'module' &&
        current.specifier === next.specifier &&
        current.css === next.css;
  }
}

/** Compare closures without allocating serialized copies. */
export function sameComponentStyleClosure(
  current: readonly string[],
  next: readonly string[],
): boolean {
  return current === next ||
    (current.length === next.length && current.every((id, index) => id === next[index]));
}

/** Validate and detach a componentStyles payload before any registry mutation. */
export function prepareComponentStyles(value: unknown): ComponentStyles | undefined {
  if (value === undefined) return undefined;
  if (!isObject(value) || value.version !== 1) {
    throw new Error('[WebUI] componentStyles must use version 1.');
  }
  if (value.strategy !== 'link' && value.strategy !== 'style' && value.strategy !== 'module') {
    throw new Error('[WebUI] componentStyles strategy must be "link", "style", or "module".');
  }
  if (!isObject(value.resources) || !isObject(value.closures)) {
    throw new Error('[WebUI] componentStyles resources and closures must be objects.');
  }

  const strategy = value.strategy;
  const resources: Record<string, ComponentStyleResource> = {};
  for (const id of Object.keys(value.resources)) {
    const resource = value.resources[id];
    if (!id || !isObject(resource) || resource.kind !== strategy) {
      throw new Error(`[WebUI] Invalid component style resource "${id}".`);
    }
    let members: string[] | undefined;
    if (resource.members !== undefined) {
      if (
        !Array.isArray(resource.members) ||
        resource.members.length < 2 ||
        resource.members.some(member => typeof member !== 'string' || !member) ||
        new Set(resource.members).size !== resource.members.length
      ) {
        throw new Error(`[WebUI] Invalid component style resource members "${id}".`);
      }
      members = [...resource.members] as string[];
    }
    if (resource.kind === 'link' && typeof resource.href === 'string' && resource.href) {
      resources[id] = members
        ? { kind: 'link', href: resource.href, members }
        : { kind: 'link', href: resource.href };
    } else if (resource.kind === 'style' && typeof resource.css === 'string') {
      resources[id] = members
        ? { kind: 'style', css: resource.css, members }
        : { kind: 'style', css: resource.css };
    } else if (
      resource.kind === 'module' &&
      typeof resource.specifier === 'string' && resource.specifier &&
      typeof resource.css === 'string'
    ) {
      resources[id] = members
        ? { kind: 'module', specifier: resource.specifier, css: resource.css, members }
        : { kind: 'module', specifier: resource.specifier, css: resource.css };
    } else {
      throw new Error(`[WebUI] Invalid component style resource "${id}".`);
    }
  }

  const closures: Record<string, string[]> = {};
  for (const root of Object.keys(value.closures)) {
    const closure = value.closures[root];
    if (!root || !Array.isArray(closure)) {
      throw new Error(`[WebUI] Invalid component style closure "${root}".`);
    }
    closures[root] = closure.map((id) => {
      if (typeof id !== 'string' || !id) {
        throw new Error(`[WebUI] Invalid resource ID in component style closure "${root}".`);
      }
      return id;
    });
  }
  return { version: 1, strategy, resources, closures };
}

/** Validate a required componentStyles payload. */
export function requireComponentStyles(value: unknown): ComponentStyles {
  const styles = prepareComponentStyles(value);
  if (!styles) {
    throw new Error('[WebUI] componentStyles is required.');
  }
  return styles;
}

/** Check a prepared payload for conflicts without publishing any definitions. */
export function validateComponentStylesRegistration(
  styles: ComponentStyles | undefined,
  document: Document = globalThis.document,
): void {
  if (!styles) return;
  const catalog = catalogFor(document);
  for (const id of Object.keys(styles.resources)) {
    const current = catalog.resources.get(id);
    if (current && !sameComponentStyleResource(current, styles.resources[id])) {
      throw new Error(`[WebUI] Conflicting component style resource "${id}".`);
    }
  }
  for (const root of Object.keys(styles.closures)) {
    const current = catalog.closures.get(root);
    const next = styles.closures[root];
    if (current && !sameComponentStyleClosure(current, next)) {
      throw new Error(`[WebUI] Conflicting component style closure "${root}".`);
    }
  }
}

/**
 * Publish a prepared payload after its conflicts have been validated.
 *
 * Internal registration paths use this to avoid cloning the payload again.
 */
export function registerPreparedComponentStyles(
  styles: ComponentStyles,
  document: Document = globalThis.document,
): void {
  const catalog = catalogFor(document);
  for (const id of Object.keys(styles.resources)) {
    if (catalog.resources.has(id)) continue;
    const resource = styles.resources[id];
    catalog.resources.set(id, resource);
  }
  for (const root of Object.keys(styles.closures)) {
    if (!catalog.closures.has(root)) {
      catalog.closures.set(root, styles.closures[root]);
    }
  }
}

/** Validate and publish definitions and ordered closures for one owning Document. */
export function registerComponentStyles(
  value: ComponentStyles | unknown,
  document: Document = globalThis.document,
): void {
  const styles = requireComponentStyles(value);
  validateComponentStylesRegistration(styles, document);
  registerPreparedComponentStyles(styles, document);
}

/** Return whether one owning Document already knows an exact resource ID. */
export function hasRegisteredComponentStyleResource(
  id: string,
  document: Document = globalThis.document,
): boolean {
  return catalogFor(document).resources.has(id);
}

/** Whether one component has registered style resources to install. */
export function hasComponentStyleWork(
  rootId: string,
  document: Document = globalThis.document,
): boolean {
  return (catalogFor(document).closures.get(rootId)?.length ?? 0) > 0;
}
