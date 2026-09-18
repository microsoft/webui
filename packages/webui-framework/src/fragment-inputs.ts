// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Decoded provenance for fragment inputs captured while a response streamed.
 *
 * The server emits one additive tuple per distinct captured value: a root
 * carries its JSON once and every projection of it is a relative reference to
 * an earlier identifier, so decoding is one forward pass with no recursion, no
 * repeated path strings, and no copies. The DOM carries only the integer that
 * selects a value (`<!--wf:7-->`), never the value itself.
 *
 * Four retention scopes exist, deliberately kept separate so that releasing one
 * never pins the others:
 *
 *  - `sources` — the response table, released at the terminal record, on a
 *    stream failure, and on cancellation;
 *  - `pending` — per-host retention of only the identifiers a host that has not
 *    hydrated yet declared it needs, counted by live adopter so an explicit
 *    rebind or a removed invocation drops the captured value immediately
 *    instead of pinning it for the host element's whole lifetime;
 *  - `seeds` — per-marker values pinned during the activation walk, so a range
 *    that hydrates after the response closed still resolves its own markers;
 *  - `retained` — the alias an adopted invocation already resolved, so a
 *    disconnect that preserves DOM rehydrates to the same captured value.
 *
 * A fifth, valueless scope (`rebound`) records the anchors whose capture
 * identity was deliberately given up. Nothing else may treat a closed table as
 * permission to resolve an identifier from current owner state: that would
 * quietly serve the owner's newer value in place of a captured one and hide the
 * skew that produced it.
 */

import { dotWalk } from './element/diff.js';
import { isRawStartMarker } from './element/markers.js';
import type { ScopeFrame } from './element/types.js';
import type { FragmentSourceNode } from './streaming-protocol.js';

let sources: Map<number, unknown> | undefined;
let pending: WeakMap<Element, Map<number, PendingInput>> | undefined;
let retained: WeakMap<Comment, RetainedFragmentInput> | undefined;
let seeds: WeakMap<Comment, unknown> | undefined;
let rebound: WeakSet<Comment> | undefined;

/**
 * One captured value a dormant host reserved, and how many live invocations
 * currently resolve through it.
 *
 * `adopters` is zero both before the first adoption and after the last one is
 * abandoned; only the latter releases the value, so a host that never activated
 * keeps everything it reserved.
 */
interface PendingInput {
  value: unknown;
  adopters: number;
}

/** An adopted invocation can reconnect without recapturing newer owner state. */
export interface RetainedFragmentInput {
  scope: ScopeFrame;
  version: number;
}

function missing(detail: string): never {
  throw new Error(`[WebUI] Invalid captured fragment inputs: ${detail}.`);
}

function source(id: number): unknown {
  if (!sources?.has(id)) missing(`unknown source ${id}`);
  return sources!.get(id);
}

/** Resolve additive source DAG definitions once, independently of host activation. */
export function registerFragmentSources(nodes: FragmentSourceNode[]): void {
  const values = sources ??= new Map();
  for (const node of nodes) {
    const [id, kind] = node;
    if (values.has(id)) missing(`duplicate source ${id}`);
    let value: unknown;
    if (kind === 0) value = node[2];
    else {
      const parent = source(node[2]);
      value = kind === 1
        ? dotWalk(parent, node[3], 0)
        : Array.isArray(parent) ? parent[node[3]] : undefined;
    }
    if (value === undefined) missing(`missing value for source ${id}`);
    values.set(id, value);
  }
}

/**
 * Retain only the distinct resolved inputs one not-yet-hydrated host needs.
 *
 * The response table is released at the terminal record, but a dormant, lazy,
 * or barrier-deferred host activates later and must still resolve the markers
 * in its own range. Retention is bounded by that host element's own
 * reachability rather than by its first hydration, because nested component
 * frames inside it can hydrate on their own schedule.
 */
export function registerFragmentSourceRefs(host: Element, refs: number[]): void {
  const map = pending ??= new WeakMap();
  let values = map.get(host);
  if (!values) map.set(host, values = new Map<number, PendingInput>());
  for (const id of refs) {
    if (!values.has(id)) values.set(id, { value: source(id), adopters: 0 });
  }
}

/**
 * Read the exact input selected by a streamed `wf:ID` invocation marker.
 *
 * Resolution is ordered by narrowness: this marker's own seed, then the
 * identifiers its host reserved, then the open response table, where an unknown
 * identifier is real server/client skew and fails loudly.
 *
 * A closed table is not itself permission to fall back. An invocation that gave
 * up its capture identity — an explicit input write, or removal — resolves its
 * caller path exactly like an uncaptured call. An invocation that still claims
 * a captured value and can no longer produce one is a retention defect, and
 * silently answering it from current owner state would publish the owner's
 * newer value under a captured marker, so it raises instead.
 */
export function fragmentInput(host: Element, id: number, anchor?: Comment): unknown {
  if (anchor && rebound?.has(anchor)) return undefined;
  if (anchor && seeds?.has(anchor)) {
    const value = seeds.get(anchor);
    seeds.delete(anchor);
    return value;
  }
  const entry = pending?.get(host)?.get(id);
  if (entry) return entry.value;
  if (sources) return source(id);
  return releasedInput(id, anchor);
}

/** A capture only stops binding when its own invocation deliberately let it go. */
function releasedInput(id: number, anchor?: Comment): undefined {
  if (anchor && rebound?.has(anchor)) return undefined;
  missing(`unknown source ${id} after the response closed`);
}

/**
 * Resolve an input and record that one more live invocation depends on it.
 *
 * Only reservations can be adopted: the response table and the per-marker seeds
 * are released on their own schedule, so counting them would retain nothing.
 * Every successful adoption must be matched by `releaseFragmentInput` when the
 * invocation is rebound to a caller path or removed outright.
 */
export function adoptFragmentInput(host: Element, id: number, anchor?: Comment): unknown {
  if (anchor && rebound?.has(anchor)) return undefined;
  const entry = pending?.get(host)?.get(id);
  if (anchor && seeds?.has(anchor)) {
    const value = seeds.get(anchor);
    seeds.delete(anchor);
    if (entry) entry.adopters++;
    return value;
  }
  if (entry) {
    entry.adopters++;
    return entry.value;
  }
  if (sources) return source(id);
  return releasedInput(id, anchor);
}

/**
 * Drop one live invocation's claim on a reserved input.
 *
 * The reservation survives while any other invocation still resolves through
 * it, and while it has never been adopted at all — a host whose range is still
 * dormant, lazy, or barrier-deferred has not had its chance to claim anything
 * yet. Losing the last adopter releases the captured value even though the host
 * element itself stays alive.
 */
export function releaseFragmentInput(host: Element, id: number): void {
  const values = pending?.get(host);
  const entry = values?.get(id);
  if (!entry || entry.adopters === 0 || --entry.adopters > 0) return;
  values!.delete(id);
  if (values!.size === 0) pending!.delete(host);
}

/** Decode the canonical response-wide u32 source ID on a streamed call marker. */
export function fragmentSourceId(data: string): number {
  if (data.length === 3) missing('empty invocation source ID');
  let id = 0;
  for (let i = 3; i < data.length; i++) {
    const digit = data.charCodeAt(i) - 48;
    if (digit < 0 || digit > 9) missing('invalid invocation source ID');
    id = id * 10 + digit;
    if (id > 0xffffffff) missing('invocation source ID exceeds u32');
  }
  return id;
}

/** Drop response roots; successful terminal records preserve deferred host inputs. */
export function clearFragmentInputSources(abandon = false): void {
  sources = undefined;
  if (abandon) {
    pending = undefined;
    seeds = undefined;
  }
}

/**
 * Close a buffered page's source table the way a terminal record closes a
 * streamed one.
 *
 * A buffered response has no terminal record, so nothing would ever release the
 * roots decoded out of `#webui-data` and the page would pin every captured
 * value for its whole lifetime. Startup hydration is the equivalent hand-off
 * point: everything that hydrated has already adopted its input, and the only
 * invocations still owed one are the markers of hosts that are dormant, lazy,
 * or not yet defined. Seeding those markers moves retention onto the marker
 * nodes themselves, so removing or rebinding a host releases its captured
 * values instead of keeping the whole table alive.
 *
 * The walk is skipped entirely unless the page actually decoded sources, so an
 * ordinary buffered page pays nothing for this.
 */
export function transferFragmentInputSeeds(root: Node | null | undefined): void {
  const capture = createFragmentSourceCapture();
  if (capture && root) {
    let node: Node | null = root;
    while (node) {
      if (node.nodeType === 8) capture.visit(node as Comment);
      node = nextInBufferedOrder(node, root);
    }
  }
  clearFragmentInputSources();
}

/**
 * Document-order successor that also descends declarative shadow roots.
 *
 * The streaming graph owns an equivalent walk, but that module carries the
 * component-span vocabulary the default entry must never reach, so this one
 * stays local. It is iterative and keeps no stack: descent is a child lookup
 * and ascent walks the parent chain, hopping from a shadow root back to its
 * host's light children the same way a boundary walk does.
 */
function nextInBufferedOrder(node: Node, root: Node): Node | null {
  if (node.nodeType === 1 /* ELEMENT_NODE */) {
    const shadowRoot = (node as Element).shadowRoot;
    if (shadowRoot?.firstChild) return shadowRoot.firstChild;
  }
  if (node.firstChild) return node.firstChild;
  let current: Node | null = node;
  while (current && current !== root) {
    if (current.nodeType === 11 /* DOCUMENT_FRAGMENT_NODE */) {
      const host: Element | null = (current as ShadowRoot).host ?? null;
      if (host) {
        if (host.firstChild) return host.firstChild;
        current = host;
        continue;
      }
    }
    if (current.nextSibling) return current.nextSibling;
    current = current.parentNode;
  }
  return null;
}

/** Capture source markers during the existing streaming walk, without a second DOM scan. */
export class FragmentSourceCapture {
  private raw: string | undefined;
  private rawEnd: string | undefined;
  private rawParent: Node | null = null;
  private depth = 0;

  /** Preserve only owned marker inputs; raw HTML contents remain opaque. */
  visit(node: Comment): void {
    const data = node.data;
    if (this.raw !== undefined) {
      if (node.parentNode !== this.rawParent) return;
      if (data === this.raw) this.depth++;
      else if (data === this.rawEnd && --this.depth === 0) {
        this.raw = undefined;
        this.rawEnd = undefined;
        this.rawParent = null;
      }
      return;
    }
    if (isRawStartMarker(data)) {
      this.raw = data;
      this.rawEnd = `/${data}`;
      this.rawParent = node.parentNode;
      this.depth = 1;
    } else if (data.startsWith('wf:') && !rebound?.has(node)) {
      (seeds ??= new WeakMap()).set(node, source(fragmentSourceId(data)));
    }
  }
}

/** Ordinary streams allocate no marker-capture state. */
export function createFragmentSourceCapture(): FragmentSourceCapture | undefined {
  return sources?.size ? new FragmentSourceCapture() : undefined;
}

/** Retain an adopted alias weakly through teardown that preserves its DOM. */
export function retainFragmentInput(
  anchor: Comment,
  scope: ScopeFrame,
  version: number,
): void {
  (retained ??= new WeakMap()).set(anchor, { scope, version });
}

/** Read a retained alias before restoring an invocation's mutable binding. */
export function retainedFragmentInput(anchor: Comment): RetainedFragmentInput | undefined {
  return retained?.get(anchor);
}

/**
 * Explicit input writes and removed invocations release any retained alias.
 *
 * Giving up the alias also gives up the anchor's capture identity: from here on
 * the invocation resolves its caller path, and a later attempt to read its
 * identifier out of a closed table is a legitimate fallback rather than a lost
 * capture. Only this deliberate hand-off earns that; the mark holds no value
 * and keeps nothing alive.
 */
export function forgetFragmentInput(anchor: Comment): void {
  retained?.delete(anchor);
  seeds?.delete(anchor);
  (rebound ??= new WeakSet()).add(anchor);
}
