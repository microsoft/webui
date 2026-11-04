// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { enableHydration } from '@microsoft/fast-element/hydration.js';
import { fetchComponentTemplates } from '../../../../packages/webui-delivery/src/resources-entry.js';

class ReviewFastCard extends FASTElement {
  @attr count = 1;
  increment(): void { this.count = Number(this.count) + 1; }
}

enableHydration();
const version = new URL(location.href).searchParams.get('version') ?? '3';
Object.assign(window, {
  async mountFastCard(partial: boolean): Promise<void> {
    // Exercise rematerialization after releasing the unused declarative payload.
    document.querySelector('f-template[name="review-fast-card"]')?.remove();
    const endpoint = `/fast-assets/${partial ? 'partial' : 'templates'}/${version}`;
    await fetchComponentTemplates(['review-fast-card'], '', endpoint, '', () => {});
    await ReviewFastCard.define({ name: 'review-fast-card', template: declarativeTemplate() });
    const card = document.createElement('review-fast-card');
    document.body.appendChild(card);
  },
});
if (document.querySelector('review-fast-card')) {
  await ReviewFastCard.define({ name: 'review-fast-card', template: declarativeTemplate() });
}
