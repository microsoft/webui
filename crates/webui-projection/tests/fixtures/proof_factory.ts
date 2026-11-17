// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
function mix(Base) {
  class Mixed extends Base { @observable inherited = ''; }
  return Mixed;
}
