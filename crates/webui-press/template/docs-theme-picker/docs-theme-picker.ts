// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";
import { getAppearance, getThemeFamilies, setAppearance, subscribeAppearance } from "@microsoft/webui-press/appearance.js";
import type { Appearance, ThemeFamily } from "@microsoft/webui-press/appearance.js";

export class DocsThemePicker extends WebUIElement {
  @observable appearance!: Appearance & { themes: readonly ThemeFamily[] };
  @observable ready = false;
  @observable error = "";
  private unsubscribe?: () => void;

  override connectedCallback(): void {
    super.connectedCallback();
    if (this.ready) this.follow();
  }

  protected override hydratedCallback(): void {
    this.follow();
    this.ready = true;
  }

  private follow(): void {
    this.unsubscribe?.();
    const themes = getThemeFamilies();
    this.appearance = { ...getAppearance(), themes };
    this.unsubscribe = subscribeAppearance(appearance => { this.appearance = { ...appearance, themes }; });
  }

  override disconnectedCallback(): void {
    this.unsubscribe?.();
    this.unsubscribe = undefined;
    super.disconnectedCallback();
  }

  async change(event: Event): Promise<void> {
    this.error = "";
    try {
      await setAppearance({ theme: (event.target as HTMLSelectElement).value });
    } catch (error) {
      this.error = "Theme could not be updated. Please try again.";
      console.error("[WebUI Press] Theme update failed.", error);
    }
    this.appearance = { ...getAppearance(), themes: getThemeFamilies() };
  }
}

DocsThemePicker.define("docs-theme-picker");
