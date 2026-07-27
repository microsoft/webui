// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Package webui_ffi provides test bindings to the WebUI C shared library.
package webui_ffi

// #cgo LDFLAGS: -L../../../../target/debug -lwebui_ffi
// #cgo darwin LDFLAGS: -framework CoreFoundation -framework Security
// #cgo linux LDFLAGS: -lm -ldl -lpthread
// #include <stdint.h>
// #include <stdlib.h>
//
// extern void  *webui_handler_create();
// extern void   webui_handler_destroy(void *handler_ptr);
// extern void  *webui_protocol_create(const unsigned char *protocol_data,
//                                     uintptr_t protocol_len);
// extern void   webui_protocol_destroy(void *protocol_ptr);
// extern char  *webui_handler_render(void *handler_ptr,
//                                    const void *protocol_ptr,
//                                    const char *data_json,
//                                    const char *entry_id,
//                                    const char *request_path);
// extern void   webui_free(char *string_ptr);
// extern const char *webui_last_error();
import "C"
import "unsafe"

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
	defer C.webui_free(pointer)
	return C.GoString(pointer), nil
}

// RenderRaw invokes the loaded-protocol render function with explicit handles.
func RenderRaw(handler, protocol unsafe.Pointer, stateJSON string) *C.char {
	cState := C.CString(stateJSON)
	defer C.free(unsafe.Pointer(cState))
	cEntry := C.CString("index.html")
	defer C.free(unsafe.Pointer(cEntry))
	cPath := C.CString("/")
	defer C.free(unsafe.Pointer(cPath))
	return C.webui_handler_render(handler, protocol, cState, cEntry, cPath)
}

// Free releases a returned native string.
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
