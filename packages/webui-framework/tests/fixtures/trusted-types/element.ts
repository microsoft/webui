// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { configureTrustedTypes } from '../../../src/trusted-types.js';

configureTrustedTypes('review-compiled');
void import('./src/test-trusted-types/test-trusted-types.js');
