// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import type { RouteLoaderContext } from '@microsoft/webui-router';

export class PageLoader extends WebUIElement {
  @observable source = '';
  @observable loaderMessage = '';

  static async loader(_ctx: RouteLoaderContext): Promise<Record<string, unknown>> {
    return {
      source: 'client-loader',
      loaderMessage: 'Data fetched by static loader',
    };
  }
}
