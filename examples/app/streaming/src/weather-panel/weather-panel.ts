// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

/**
 * A deferred island whose data arrives through its original HTML response.
 *
 * The class contains no client data source. Its boundary is committed as
 * updatable, and the server later projects a forecast state record onto this
 * root whenever backend work completes.
 */
export class WeatherPanel extends WebUIElement {
  @attr location = '';
  @attr temperature = '';
  @attr condition = '';
  /** `loading` in the checkpoint, then `ready` in a server state update. */
  @attr status = 'loading';
}

WeatherPanel.define('weather-panel');
