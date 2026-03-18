// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Routes example — demonstrates nested routing with FAST-HTML hydration.
 *
 * Route structure:
 *   / → routes-app (shell with nav)
 *     ./sections/:id → section-page (topics list)
 *       ./topics/:topicId → topic-page (lessons list)
 *         ./lessons/:lessonId → lesson-page (lesson content)
 */

performance.mark('routes-hydration-started');

import { TemplateElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';

import './routes-app/routes-app.js';
import './section-page/section-page.js';
import './topic-page/topic-page.js';
import './lesson-page/lesson-page.js';

TemplateElement.options({
  'routes-app': { observerMap: 'all' },
  'section-page': { observerMap: 'all' },
  'topic-page': { observerMap: 'all' },
  'lesson-page': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    performance.measure('routes-hydration-completed', 'routes-hydration-started');
    console.log('Routes example hydration complete!');

    Router.start({
      loaders: {
        'section-page': () => import('./section-page/section-page.js'),
        'topic-page': () => import('./topic-page/topic-page.js'),
        'lesson-page': () => import('./lesson-page/lesson-page.js'),
      },
    });
  },
}).define({
  name: 'f-template',
});
