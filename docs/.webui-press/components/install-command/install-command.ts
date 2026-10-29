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

const packageManagerKey = "webui-press:tabs:package-manager";

export class InstallCommand extends WebUIElement {
  @observable installOptions: InstallOption[] = [];
  @observable installReady = false;
  @observable copyState: CopyState = "idle";

  private copyGeneration = 0;
  private preferenceStorage?: Storage;

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
    this.restoreManager();
    this.installReady = true;
  }

  private restoreManager(): void {
    let storage: Storage;
    let stored: string | null;
    try {
      const available = this.ownerDocument.defaultView?.localStorage;
      if (!available) {
        console.warn("[WebUI docs] Package-manager preferences require localStorage; selection remains page-local.");
        return;
      }
      storage = available;
      stored = storage.getItem(packageManagerKey);
    } catch (error) {
      console.warn("[WebUI docs] Cannot restore package-manager preferences; selection remains page-local.", error);
      return;
    }
    if (stored !== null && this.installOptions.some(option => option.id === stored)) {
      this.selectManager(stored);
    }
    this.preferenceStorage = storage;
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
    if (this.preferenceStorage) {
      try {
        this.preferenceStorage.setItem(packageManagerKey, id);
      } catch (error) {
        this.preferenceStorage = undefined;
        console.warn("[WebUI docs] Cannot save package-manager preferences; selection remains page-local.", error);
      }
    }
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
