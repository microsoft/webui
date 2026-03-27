// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class LessonPage extends WebUIElement {
  @observable id = '';
  @observable sectionName = '';
  @observable topicId = '';
  @observable topicName = '';
  @observable lessonId = '';
  @observable lessonName = '';
  @observable lessonContent = '';
}

LessonPage.define('lesson-page');
