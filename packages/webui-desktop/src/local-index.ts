// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export * from './index.js';
import { NativeFrameCarrier } from './native-carrier.js';
import { createDesktopTransportWithCarrier, type DesktopTransportOptions } from './transport.js';

/** Explicit native-enabled entry; the ordinary package root excludes this carrier. */
export function createNativeDesktopTransport(options: DesktopTransportOptions = {}) {
  return createDesktopTransportWithCarrier(
    (lane, session, ledger) => new NativeFrameCarrier(lane, session, ledger), options,
  );
}
export { createNativeDesktopTransport as createDesktopTransport };
