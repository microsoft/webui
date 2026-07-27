// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class ErrorDisplay extends WebUIElement {
  @observable errorMessage = '';
  @observable errorPath = '';

  onRetry = (): void => {
    window.navigation.navigate('/');
  };
}
