// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
export class First extends WebUIElement { @observable claimed = ''; }
export class Other extends WebUIElement { @observable actual = ''; }
export function mix(config, Base) {
  class Mixed extends Base { @observable own = ''; }
  return Mixed;
}
