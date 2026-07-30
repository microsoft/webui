// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

/** The shape `GET api/weather` returns. */
interface Forecast {
  location: string;
  temperature: string;
  condition: string;
}

/**
 * The weather panel is the counterpart to the streamed boundaries below it:
 * it shows what to do with data that is *not* ready in document order.
 *
 * Native HTML streaming is strictly in-order, so once the response has
 * streamed past `<header>` nothing later in the same response can fill it
 * in. Waiting for the forecast before flushing would therefore hold back the
 * composer — exactly the head-of-line blocking that streaming boundaries
 * exist to remove. So this panel's boundary carries no server data at all:
 * it is the cheapest checkpoint on the page and commits immediately, and the
 * component fetches its own forecast from the client.
 *
 * That split is the whole point, and it is why the panel is a component
 * rather than a static skeleton. Streaming hydration makes this element
 * interactive while the response is still open, so `hydratedCallback` — and
 * with it the forecast request — runs long before `DOMContentLoaded`. The
 * expensive backend work overlaps the rest of the stream instead of queueing
 * behind it, and the forecast typically lands between two feed batches.
 *
 * Ordering is safe in both directions. `@attr` writes reflect to attributes
 * as soon as the element is deferred-ready, and `$shouldApplySSRState` skips
 * any key whose attribute is already present, so a forecast that somehow beat
 * its own boundary commit is preserved rather than reset to `loading`.
 *
 * Filling this header from later in the *same* response is the deferred
 * boundary placement design in DESIGN.md; it is deliberately outside the current design.
 */
export class WeatherPanel extends WebUIElement {
  @attr location = '';
  @attr temperature = '';
  @attr condition = '';
  /** `loading` (server-rendered), then `ready` or `error` from the fetch. */
  @attr status = 'loading';

  protected override hydratedCallback(): void {
    void this.loadForecast();
  }

  private async loadForecast(): Promise<void> {
    try {
      // Relative so the panel keeps working under the demo shell's
      // sub-path mount, where `<base href>` is not `/`.
      const response = await fetch('./api/weather', {
        headers: { Accept: 'application/json' },
      });
      if (!response.ok) {
        throw new Error(`Weather request failed with ${response.status}`);
      }
      const forecast = await response.json() as Forecast;
      this.location = forecast.location;
      this.temperature = forecast.temperature;
      this.condition = forecast.condition;
      this.status = 'ready';
    } catch {
      // A demo backend that is down should degrade to a readable message,
      // never to a skeleton that shimmers forever.
      this.status = 'error';
    }
  }
}

WeatherPanel.define('weather-panel');
