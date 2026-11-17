// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export function mix<T extends new (...args: any[]) => WebUIElement>(
  Base: T, configuration: { value: string }
) {
  class Mixed extends Base {
    @attr inherited = configuration.value;
    ordinary = 0;
    connectedCallback() { super.connectedCallback(); this.ordinary += 1; }
  }
  return Mixed;
}
