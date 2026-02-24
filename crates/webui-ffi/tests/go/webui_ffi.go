// Package webui_ffi provides Go bindings to the webui_ffi C shared library.
package webui_ffi

// #cgo LDFLAGS: -L../../../../target/debug -lwebui_ffi
// #cgo darwin LDFLAGS: -framework CoreFoundation -framework Security
// #cgo linux LDFLAGS: -lm -ldl -lpthread
// #include <stdlib.h>
//
// // Forward declarations matching the generated C header.
// extern void  *webui_handler_create();
// extern void   webui_handler_destroy(void *handler_ptr);
// extern char  *webui_handler_render(void *handler_ptr,
//                                     const unsigned char *protocol_data,
//                                     unsigned long protocol_len,
//                                     const char *data_json);
// extern char  *webui_render(const char *html, const char *data_json);
// extern void   webui_free(char *string_ptr);
// extern const char *webui_last_error();
import "C"
import "unsafe"

// Render calls the C webui_render function and returns the result as a Go
// string plus any error. The caller does not need to free anything.
func Render(html, dataJSON string) (string, error) {
	cHTML := C.CString(html)
	defer C.free(unsafe.Pointer(cHTML))

	cJSON := C.CString(dataJSON)
	defer C.free(unsafe.Pointer(cJSON))

	ptr := C.webui_render(cHTML, cJSON)
	if ptr == nil {
		return "", renderError()
	}

	result := C.GoString(ptr)
	C.webui_free(ptr)
	return result, nil
}

// LastError returns the most recent FFI error message, or an empty string.
func LastError() string {
	ptr := C.webui_last_error()
	if ptr == nil {
		return ""
	}
	return C.GoString(ptr)
}

// RenderRaw calls webui_render with raw C string pointers. Used by tests
// that need to pass nil.
func RenderRaw(cHTML, cJSON *C.char) *C.char {
	return C.webui_render(cHTML, cJSON)
}

// Free calls webui_free on the given pointer.
func Free(ptr *C.char) {
	C.webui_free(ptr)
}

// HandlerCreate calls webui_handler_create.
func HandlerCreate() unsafe.Pointer {
	return C.webui_handler_create()
}

// HandlerDestroy calls webui_handler_destroy.
func HandlerDestroy(ptr unsafe.Pointer) {
	C.webui_handler_destroy(ptr)
}

// HandlerRender calls webui_handler_render.
func HandlerRender(handler unsafe.Pointer, protoData *C.uchar, protoLen C.ulong, dataJSON *C.char) *C.char {
	return C.webui_handler_render(handler, protoData, protoLen, dataJSON)
}

// CString wraps C.CString for use from test files.
func CString(s string) *C.char {
	return C.CString(s)
}

// CFree wraps C.free for use from test files.
func CFree(ptr unsafe.Pointer) {
	C.free(ptr)
}

// GoString wraps C.GoString for use from test files.
func GoString(ptr *C.char) string {
	return C.GoString(ptr)
}

func renderError() error {
	msg := LastError()
	if msg == "" {
		msg = "<no error>"
	}
	return &ffiError{msg: msg}
}

type ffiError struct {
	msg string
}

func (e *ffiError) Error() string {
	return "webui_render failed: " + e.msg
}
