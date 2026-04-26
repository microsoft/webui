// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

export class CodeBlock extends WebUIElement {
  @observable label = "Copy";

  copy(): void {
    const code = this.querySelector("code");
    if (code) {
      navigator.clipboard.writeText(code.textContent || "").then(() => {
        this.label = "Copied!";
        setTimeout(() => {
          this.label = "Copy";
        }, 1500);
      });
    }
  }
}

CodeBlock.define("code-block");
