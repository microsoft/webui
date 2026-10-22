// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

interface InstallOption {
  id: string;
  label: string;
  command: string;
  selected: boolean;
}

type CopyState = "idle" | "copying" | "copied" | "failed" | "unavailable";

export class InstallCommand extends WebUIElement {
  @observable installOptions: InstallOption[] = [];
  @observable installReady = false;
  @observable copyState: CopyState = "idle";

  private copyGeneration = 0;

  protected hydratedCallback(): void {
    let selected = 0;
    for (const option of this.installOptions) {
      if (option.selected && option.command) selected++;
    }
    if (selected !== 1) {
      this.copyState = "unavailable";
      console.error("[WebUI docs] Install commands require exactly one selected command.");
      return;
    }
    this.installReady = true;
  }

  selectManager(id: string): void {
    if (!this.installOptions.some(option => option.id === id)) {
      this.copyState = "unavailable";
      this.installReady = false;
      console.error("[WebUI docs] Unknown install command selection.");
      return;
    }
    this.copyGeneration++;
    this.installOptions = this.installOptions.map(option => ({
      ...option,
      selected: option.id === id,
    }));
    if (this.copyState !== "copying") this.copyState = "idle";
  }

  async copy(): Promise<void> {
    if (!this.installReady || this.copyState === "copying") return;
    const selected = this.installOptions.find(option => option.selected);
    if (!selected) {
      this.copyState = "unavailable";
      console.error("[WebUI docs] No install command is selected.");
      return;
    }
    const generation = ++this.copyGeneration;
    this.copyState = "copying";
    let result: CopyState = "copied";
    try {
      await navigator.clipboard.writeText(selected.command);
    } catch (error) {
      result = "failed";
      if (generation === this.copyGeneration) {
        console.warn("[WebUI docs] Clipboard copy failed; select and copy the command manually.", error);
      }
    }
    this.copyState = generation === this.copyGeneration ? result : "idle";
  }

  disconnectedCallback(): void {
    this.copyGeneration++;
    super.disconnectedCallback();
  }
}

InstallCommand.define("install-command");
