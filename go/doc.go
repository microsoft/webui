// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Package webui renders compiled WebUI protocols through the native Rust C ABI.
// It requires cgo and a version-matched WebUI native SDK. Set CGO_CFLAGS to
// include the SDK headers and CGO_LDFLAGS to locate its libraries.
//
// Compile templates before deployment with the WebUI CLI. NewRenderer loads
// that binary protocol once; Render supplies UTF-8 JSON state without running
// a compiler or JavaScript runtime. Call Close to release native resources.
package webui
