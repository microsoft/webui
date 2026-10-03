// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

interface ActionResult {
  status: string;
  detail: string;
}

export class NativeServicesApp extends WebUIElement {
  @attr status = 'Ready';
  @attr detail = '';
  @attr({ mode: 'boolean' }) busy = false;
  @attr({ mode: 'boolean', attribute: 'has-capture' }) hasCapture = false;
  @attr({ attribute: 'capture-url' }) captureUrl = '';

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.releaseCaptureUrl();
  }

  pickDirectory(): Promise<void> {
    return this.runJsonAction('/api/picker', 'Opening directory picker');
  }

  showError(): Promise<void> {
    return this.runJsonAction('/api/dialog/error', 'Opening native error dialog');
  }

  confirmAction(): Promise<void> {
    return this.runJsonAction('/api/dialog/confirm', 'Opening native confirmation');
  }

  copyCapture(): Promise<void> {
    return this.runJsonAction('/api/clipboard', 'Writing PNG to clipboard');
  }

  async captureView(): Promise<void> {
    if (this.busy) return;
    this.releaseCaptureUrl();
    this.begin('Capturing visible webview');
    try {
      const response = await this.request('/api/capture');
      if (!response.ok) throw await this.responseError(response);
      const blob = await response.blob();
      this.releaseCaptureUrl();
      this.captureUrl = URL.createObjectURL(blob);
      this.hasCapture = true;
      this.status = 'Capture ready';
      this.detail = `${blob.size.toLocaleString()} PNG bytes retained and previewed.`;
    } catch (error) {
      this.fail(error);
    } finally {
      this.busy = false;
    }
  }

  private async runJsonAction(path: string, pending: string): Promise<void> {
    if (this.busy) return;
    this.begin(pending);
    try {
      const response = await this.request(path);
      if (!response.ok) throw await this.responseError(response);
      const result = await response.json() as ActionResult;
      this.status = result.status;
      this.detail = result.detail;
    } catch (error) {
      this.fail(error);
    } finally {
      this.busy = false;
    }
  }

  private begin(status: string): void {
    this.busy = true;
    this.status = status;
    this.detail = 'Waiting for the trusted native host.';
  }

  private request(path: string): Promise<Response> {
    return fetch(path, {
      method: 'POST',
      headers: { 'X-WebUI-Native-Demo': '1' },
    });
  }

  private fail(error: unknown): void {
    this.status = 'Action failed';
    this.detail = error instanceof Error ? error.message : 'The native host returned an unknown error.';
  }

  private async responseError(response: Response): Promise<Error> {
    const result = await response.json().catch(() => undefined) as Partial<ActionResult> | undefined;
    return new Error(result?.detail ?? `Native host returned HTTP ${response.status}.`);
  }

  private releaseCaptureUrl(): void {
    if (this.captureUrl.startsWith('blob:')) URL.revokeObjectURL(this.captureUrl);
    this.captureUrl = '';
    this.hasCapture = false;
  }
}

NativeServicesApp.define('native-services-app');
