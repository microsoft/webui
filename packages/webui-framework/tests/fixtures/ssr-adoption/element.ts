// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, attr, observable } from '../../../src/index.js';

startRuntime();

interface Project {
  id: string;
  name: string;
  status: string;
  description: string;
  owner: string;
  updated: string;
  active: boolean;
  labels: string[];
}

export class AdoptionLibrary extends WebUIElement {
  @observable projects: Project[] = [];
  @observable summary = '';
  @observable details = true;

  add(): void {
    this.projects = [...this.projects, {
      id: `added-${this.projects.length}`, name: 'New project', status: 'Draft',
      description: 'A newly created project.', owner: 'Local user',
      updated: 'Today', active: true, labels: ['New', 'Local'],
    }];
  }

  reverse(): void { this.projects = [...this.projects].reverse(); }
  toggle(): void { this.details = !this.details; }
}

export class AdoptionProject extends WebUIElement {
  @observable project?: Project;
  @observable notes = false;

  toggle(): void { this.notes = !this.notes; }
}

export class AdoptionStatus extends WebUIElement {
  @attr label = '';
}

AdoptionLibrary.define('adoption-library');
AdoptionProject.define('adoption-project');
AdoptionStatus.define('adoption-status');
