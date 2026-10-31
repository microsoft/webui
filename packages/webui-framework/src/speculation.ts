// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Templates that a speculative lazy load registers.
 *
 * Their stylesheets are first prepared while they register, on paths the
 * rendering core drives with only the template in hand. `lazy.js` marks the
 * templates so `styles.js` prepares those stylesheets at low fetch priority,
 * and not before the window has loaded; neither module imports the other.
 */
export const speculativeTemplates = new WeakSet<object>();
