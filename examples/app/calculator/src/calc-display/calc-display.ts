// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { attributeMap } from '@microsoft/fast-element/attribute-map.js';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';

export class CalcDisplay extends FASTElement {
  @attr expression = '';
  @attr value = '';
  @attr error = '';
}

void CalcDisplay.define({
  name: 'calc-display',
  template: declarativeTemplate(),
}, [attributeMap(), observerMap()]);
