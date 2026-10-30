// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";
import { getAppearance, setAppearance, subscribeAppearance } from "@microsoft/webui-press/appearance.js";
import type { Appearance, AppearanceMode } from "@microsoft/webui-press/appearance.js";

interface ControlAppearance extends Appearance {
  selected: Record<AppearanceMode, boolean>;
}

function controlAppearance(appearance: Appearance): ControlAppearance {
  return {
    ...appearance,
    selected: {
      system: appearance.mode === "system",
      light: appearance.mode === "light",
      dark: appearance.mode === "dark",
    },
  };
}

export class DocsThemeToggle extends WebUIElement {
  @observable appearance!: ControlAppearance;
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
    this.appearance = controlAppearance(getAppearance());
    this.unsubscribe = subscribeAppearance(appearance => { this.appearance = controlAppearance(appearance); });
  }

  override disconnectedCallback(): void {
    this.unsubscribe?.();
    this.unsubscribe = undefined;
    super.disconnectedCallback();
  }

  async change(mode: AppearanceMode): Promise<void> {
    this.error = "";
    try {
      await setAppearance({ mode });
    } catch (error) {
      this.error = "Appearance could not be updated. Please try again.";
      console.error("[WebUI Press] Appearance update failed.", error);
    }
    this.appearance = controlAppearance(getAppearance());
  }
}

DocsThemeToggle.define("docs-theme-toggle");
