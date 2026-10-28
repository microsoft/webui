// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export interface Item {
  id: string;
  title: string;
  detail: string;
  html: string;
  children: Array<{ id: string; title: string }>;
}

export interface Capture {
  nodes: Array<{ node: Node; parent: ParentNode | null }>;
  probes: Element[];
  repeatNodes: Node[];
  disconnects: string[];
}

const capture: Capture = { nodes: [], probes: [], repeatNodes: [], disconnects: [] };
Reflect.set(window, '__connectivityCapture', capture);

class ConnectivityProbe extends WebUIElement {
  disconnectedCallback(): void {
    capture.disconnects.push(this.getAttribute('data-id') ?? '');
    super.disconnectedCallback();
  }
}

export class ConnectivityList extends WebUIElement {
  @observable items: Item[] = [];
  @observable visible = true;
  @observable selected = '';

  select(id: string): void { this.selected = id; }
}

// Upgrade children before their parent so synchronous hydration moves are observable.
ConnectivityProbe.define('connectivity-probe');
const root = document.querySelector('connectivity-list')!.shadowRoot!;
capture.probes = Array.from(root.querySelectorAll('connectivity-probe'));
const staleMarkers = new Set(['wc', '/wc', 'wi', '/wr']);
const repeatNodes = new Set<Node>();
const stack: Node[] = [root];
while (stack.length > 0) {
  const parent = stack.pop()!;
  for (let node = parent.firstChild; node; node = node.nextSibling) {
    if (node.nodeType !== Node.COMMENT_NODE || !staleMarkers.has((node as Comment).data)) {
      capture.nodes.push({ node, parent: node.parentNode });
    }
    if (node.nodeType === Node.COMMENT_NODE && (node as Comment).data === 'wi') {
      let depth = 0;
      for (let sibling = node.nextSibling; sibling; sibling = sibling.nextSibling) {
        if (sibling.nodeType === Node.COMMENT_NODE) {
          const data = (sibling as Comment).data;
          if (depth === 0 && (data === 'wi' || data === '/wr')) break;
          if (data === 'wr') depth++;
          else if (data === '/wr') depth--;
          if (staleMarkers.has(data)) continue;
        }
        repeatNodes.add(sibling);
      }
    }
    if (node.hasChildNodes()) stack.push(node);
    if (node instanceof Element && node.shadowRoot) stack.push(node.shadowRoot);
  }
}
capture.repeatNodes = Array.from(repeatNodes);
ConnectivityList.define('connectivity-list');
