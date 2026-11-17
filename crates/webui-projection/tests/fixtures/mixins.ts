// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

type ElementConstructor = new (...args: any[]) => WebUIElement;

export function withCounter<TBase extends ElementConstructor>(Base: TBase) {
  class Counter extends Base {
    @observable inheritedValue = 0;
    connectionCount = 0;

    connectedCallback(): void {
      super.connectedCallback();
      this.connectionCount += 1;
    }

    incrementInherited(): void {
      this.inheritedValue += 1;
    }
  }
  return Counter;
}

export function withCaption<TBase extends ElementConstructor>(
  Base: TBase,
  configuration: { caption: string }
) {
  class Caption extends Base {
    @attr inheritedLabel = configuration.caption;
    disconnectionCount = 0;

    disconnectedCallback(): void {
      super.disconnectedCallback();
      this.disconnectionCount += 1;
    }
  }
  return Caption;
}
