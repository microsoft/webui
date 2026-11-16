// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package webui

// #cgo !windows LDFLAGS: -lwebui_ffi
// #cgo windows LDFLAGS: -l:webui_ffi.dll.lib
// #include <stdlib.h>
// #include "webui_ffi.h"
import "C"

import (
	"bytes"
	"errors"
	"runtime"
	"strings"
	"sync"
	"unsafe"
)

// ErrClosed is returned by Render on a closed, nil, or zero-value Renderer.
var ErrClosed = errors.New("webui: renderer is closed")

// NativeError preserves the native diagnostic for a failed C ABI operation.
type NativeError struct {
	// Operation is the C ABI function that failed.
	Operation string
	// Message is the unmodified UTF-8 native diagnostic, or empty if absent.
	Message string
}

// Error returns the failing operation and its native diagnostic.
func (e *NativeError) Error() string {
	return e.Operation + ": " + e.Message
}

// RenderOptions selects the document entry, request route, and CSP nonce.
type RenderOptions struct {
	// Entry is the compiled entry identifier; empty means "index.html".
	Entry string
	// RequestPath is the encoded request route; empty means "/".
	// For net/http requests, use request.URL.EscapedPath().
	RequestPath string
	// Nonce is the response's CSP nonce for generated scripts and preload hints.
	// Empty disables nonce emission. Use a fresh, unpredictable nonce per response
	// and include the same value in the Content-Security-Policy header.
	Nonce string
}

// Renderer owns a loaded native protocol and its WebUI hydration handler.
// Render may be called concurrently. Copies share ownership: closing any copy
// closes them all. A nil or zero-value Renderer is already closed.
// A Renderer must be closed explicitly; it has no finalizer.
type Renderer struct {
	state *rendererState
}

type rendererState struct {
	mu       sync.RWMutex
	handler  unsafe.Pointer
	protocol unsafe.Pointer
}

// NewRenderer decodes a compiled protocol once using the default webui plugin.
// The protocol bytes may be reused or modified after this call returns.
func NewRenderer(protocol []byte) (*Renderer, error) {
	if len(protocol) == 0 {
		return nil, errors.New("webui: protocol is empty; compile templates with webui build")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	// SAFETY: Rust reads these bytes only during this call and retains no Go pointer.
	p := C.webui_protocol_create((*C.uint8_t)(unsafe.Pointer(&protocol[0])), C.uintptr_t(len(protocol)))
	if p == nil {
		return nil, nativeError("webui_protocol_create")
	}
	plugin := C.CString("webui")
	defer C.free(unsafe.Pointer(plugin))
	h := C.webui_handler_create_with_plugin(plugin)
	if h == nil {
		err := nativeError("webui_handler_create_with_plugin")
		C.webui_protocol_destroy(p)
		return nil, err
	}
	return &Renderer{state: &rendererState{handler: h, protocol: p}}, nil
}

// Render renders UTF-8 JSON state into Go-owned HTML bytes. The result remains
// valid after later renders and Close. JSON is parsed only by Rust; invalid JSON
// or UTF-8 returns a NativeError. Embedded NUL in state or options is rejected.
// Callers must not mutate state while Render is using it.
func (r *Renderer) Render(state []byte, options RenderOptions) ([]byte, error) {
	if r == nil || r.state == nil {
		return nil, ErrClosed
	}
	s := r.state
	s.mu.RLock()
	defer s.mu.RUnlock()
	if s.handler == nil {
		return nil, ErrClosed
	}
	if bytes.IndexByte(state, 0) >= 0 {
		return nil, errors.New("webui: state contains an embedded NUL; encode JSON strings with \\u0000")
	}
	if strings.IndexByte(options.Entry, 0) >= 0 {
		return nil, errors.New("webui: entry contains an embedded NUL; remove it")
	}
	if strings.IndexByte(options.RequestPath, 0) >= 0 {
		return nil, errors.New("webui: request path contains an embedded NUL; remove it")
	}
	if strings.IndexByte(options.Nonce, 0) >= 0 {
		return nil, errors.New("webui: nonce contains an embedded NUL; remove it")
	}
	if options.Entry == "" {
		options.Entry = "index.html"
	}
	if options.RequestPath == "" {
		options.RequestPath = "/"
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	// SAFETY: Allocate one C-owned, terminated state buffer, with no Go-side clone.
	cState := C.malloc(C.size_t(len(state)) + 1)
	defer C.free(cState)
	copy(unsafe.Slice((*byte)(cState), len(state)), state)
	*(*byte)(unsafe.Add(cState, len(state))) = 0
	cEntry, cPath := C.CString(options.Entry), C.CString(options.RequestPath)
	defer C.free(unsafe.Pointer(cEntry))
	defer C.free(unsafe.Pointer(cPath))
	var result unsafe.Pointer
	operation := "webui_handler_render"
	if options.Nonce == "" {
		result = C.webui_handler_render(s.handler, s.protocol, (*C.char)(cState), cEntry, cPath)
	} else {
		cNonce := C.CString(options.Nonce)
		defer C.free(unsafe.Pointer(cNonce))
		nativeOptions := C.webui_render_options_t{entry_id: cEntry, request_path: cPath, nonce: cNonce}
		operation = "webui_handler_render_with_options"
		// SAFETY: Options contain only C-owned strings, borrowed for this call.
		result = C.webui_handler_render_with_options(s.handler, s.protocol, (*C.char)(cState), &nativeOptions)
	}
	if result == nil {
		return nil, nativeError(operation)
	}
	defer C.webui_render_free(result)
	var length C.uintptr_t
	data := C.webui_render_bytes(result, &length)
	if data == nil {
		return nil, nativeError("webui_render_bytes")
	}
	if uint64(length) > uint64(^uint(0)>>1) {
		return nil, errors.New("webui: rendered document exceeds the Go slice length limit")
	}
	// SAFETY: Copy the borrowed buffer before the single deferred native free.
	output := make([]byte, int(length))
	copy(output, unsafe.Slice((*byte)(unsafe.Pointer(data)), int(length)))
	return output, nil
}

// Close waits for in-flight renders and releases native resources exactly once.
// Repeated calls, including on nil and zero-value renderers, return nil.
func (r *Renderer) Close() error {
	if r == nil || r.state == nil {
		return nil
	}
	s := r.state
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.handler != nil {
		C.webui_handler_destroy(s.handler)
		C.webui_protocol_destroy(s.protocol)
		s.handler, s.protocol = nil, nil
	}
	return nil
}

// Called only on the pinned thread, before any cleanup can replace TLS errors.
func nativeError(operation string) error {
	var message string
	if diagnostic := C.webui_last_error(); diagnostic != nil {
		message = C.GoString(diagnostic)
	}
	return &NativeError{Operation: operation, Message: message}
}
