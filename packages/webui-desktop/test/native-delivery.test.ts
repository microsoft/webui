// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = (path: string) => readFileSync(new URL(`../../../../crates/webui-desktop/src/${path}`, import.meta.url), 'utf8');

test('Linux installs Fetch-visible Content-Type before publishing response headers', () => {
  const response = source('linux/response.rs');
  const mime = response.indexOf('headers.append("Content-Type", &response.content_type)');
  assert(mime >= 0, 'WebKit content-type metadata alone does not populate Fetch response headers');
  assert(mime < response.indexOf('scheme_response.set_http_headers(headers)'));
});

test('Windows queues nonce-bound retirement before resetting navigation and cancelling completions', () => {
  const sourceText = source('windows/ipc.rs');
  const start = sourceText.slice(sourceText.indexOf('fn start(&self,'), sourceText.indexOf('pub(super) fn trusted_source'));
  const publish = start.indexOf('self.publish_retirement(');
  assert(publish >= 0, 'navigation must publish retirement while outgoing proof and generation exist');
  assert(publish < start.indexOf('.start(native_id)'));
  assert(publish < start.indexOf('self.bridge.navigate(navigation)'));
  assert(publish < start.indexOf('old.close()'));
  const control = source('windows/ipc_control.rs');
  const helper = control.slice(control.indexOf('fn publish_retirement('), control.indexOf('pub fn message('));
  assert(helper.includes('crate::native_ipc::control_script(&proof, control)'));
  assert(helper.includes('.ExecuteScript('));
  for (const [begin, end, mutation] of [
    ['pub fn transport_failed(', 'pub(super) fn disconnect_document(', 'document.epoch.committed = false'],
    ['pub fn close(', 'pub(super) fn cancel_hello_deadline(', 'document.epoch.closed = true'],
  ]) {
    const retiring = sourceText.slice(sourceText.indexOf(begin!), sourceText.indexOf(end!));
    assert(retiring.indexOf(mutation!) >= 0, `missing retirement mutation: ${mutation}`);
    assert(retiring.indexOf('self.publish_retirement(') >= 0);
    assert(retiring.indexOf('self.publish_retirement(') < retiring.indexOf(mutation!));
  }
});
