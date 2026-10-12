// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Package webui_ffi provides test bindings to the WebUI C shared library.
package webui_ffi

// #cgo LDFLAGS: -L../../../../target/debug -lwebui_ffi
// #cgo CFLAGS: -I../../include
// #cgo darwin LDFLAGS: -framework CoreFoundation -framework Security
// #cgo linux LDFLAGS: -lm -ldl -lpthread
// #include <stdlib.h>
// #include "webui_ffi.h"
import "C"
import (
	"runtime"
	"unsafe"
)

// HandlerCreate creates a native handler.
func HandlerCreate() unsafe.Pointer {
	return C.webui_handler_create()
}

// HandlerDestroy releases a native handler.
func HandlerDestroy(handler unsafe.Pointer) {
	C.webui_handler_destroy(handler)
}

// ProtocolCreate decodes and owns a protocol byte slice.
func ProtocolCreate(protocolBytes []byte) unsafe.Pointer {
	if len(protocolBytes) == 0 {
		return C.webui_protocol_create(nil, 0)
	}
	return C.webui_protocol_create(
		(*C.uchar)(unsafe.Pointer(&protocolBytes[0])),
		C.uintptr_t(len(protocolBytes)),
	)
}

// ProtocolDestroy releases a loaded protocol.
func ProtocolDestroy(protocol unsafe.Pointer) {
	C.webui_protocol_destroy(protocol)
}

// Render renders with loaded handler and protocol handles.
func Render(handler, protocol unsafe.Pointer, stateJSON string) (string, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	cState := C.CString(stateJSON)
	defer C.free(unsafe.Pointer(cState))
	cEntry := C.CString("index.html")
	defer C.free(unsafe.Pointer(cEntry))
	cPath := C.CString("/")
	defer C.free(unsafe.Pointer(cPath))

	pointer := C.webui_handler_render(handler, protocol, cState, cEntry, cPath)
	if pointer == nil {
		return "", operationError()
	}
	defer C.webui_render_free(pointer)
	var length C.uintptr_t
	bytes := C.webui_render_bytes(pointer, &length)
	if bytes == nil {
		return "", operationError()
	}
	if uint64(length) > uint64(^uint(0)>>1) {
		return "", &ffiError{message: "render result exceeds the Go string size limit"}
	}
	return string(unsafe.Slice((*byte)(unsafe.Pointer(bytes)), int(length))), nil
}

// RenderRaw invokes the loaded-protocol render function with explicit handles.
// The caller must stay on the same OS thread until reading LastError.
func RenderRaw(handler, protocol unsafe.Pointer, stateJSON string) unsafe.Pointer {
	cState := C.CString(stateJSON)
	defer C.free(unsafe.Pointer(cState))
	cEntry := C.CString("index.html")
	defer C.free(unsafe.Pointer(cEntry))
	cPath := C.CString("/")
	defer C.free(unsafe.Pointer(cPath))
	return C.webui_handler_render(handler, protocol, cState, cEntry, cPath)
}

// ResultDestroy releases an owned document.
func ResultDestroy(pointer unsafe.Pointer) {
	C.webui_render_free(pointer)
}

// Free releases a native string, not a document handle.
func Free(pointer *C.char) {
	C.webui_free(pointer)
}

// LastError returns the current thread's FFI error.
func LastError() string {
	pointer := C.webui_last_error()
	if pointer == nil {
		return ""
	}
	return C.GoString(pointer)
}

func operationError() error {
	message := LastError()
	if message == "" {
		message = "<no error>"
	}
	return &ffiError{message: message}
}

type ffiError struct {
	message string
}

func (error *ffiError) Error() string {
	return "webui operation failed: " + error.message
}
