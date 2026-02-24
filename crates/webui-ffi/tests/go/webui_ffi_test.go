package webui_ffi_test

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
import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"unsafe"
)

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// parseAndRender wraps the C function with proper Go string handling.
func parseAndRender(t *testing.T, html, dataJSON string) string {
	t.Helper()
	cHTML := C.CString(html)
	defer C.free(unsafe.Pointer(cHTML))

	cJSON := C.CString(dataJSON)
	defer C.free(unsafe.Pointer(cJSON))

	ptr := C.webui_render(cHTML, cJSON)
	if ptr == nil {
		errPtr := C.webui_last_error()
		errMsg := "<none>"
		if errPtr != nil {
			errMsg = C.GoString(errPtr)
		}
		t.Fatalf("webui_render returned NULL; error: %s", errMsg)
	}

	result := C.GoString(ptr)
	C.webui_free(ptr)
	return result
}

// lastError returns the last FFI error message, or empty string.
func lastError() string {
	ptr := C.webui_last_error()
	if ptr == nil {
		return ""
	}
	return C.GoString(ptr)
}

// ---------------------------------------------------------------------------
// Tests: happy paths
// ---------------------------------------------------------------------------

func TestSimplePassthrough(t *testing.T) {
	got := parseAndRender(t, "<p>Hello</p>", "{}")
	if got != "<p>Hello</p>" {
		t.Errorf("got %q, want %q", got, "<p>Hello</p>")
	}
}

func TestSignalSubstitution(t *testing.T) {
	got := parseAndRender(t, "Hello, {{name}}!", `{"name":"WebUI"}`)
	if got != "Hello, WebUI!" {
		t.Errorf("got %q, want %q", got, "Hello, WebUI!")
	}
}

func TestForLoop(t *testing.T) {
	html := `<ul><for each="item in items"><li>{{item}}</li></for></ul>`
	got := parseAndRender(t, html, `{"items":["a","b","c"]}`)
	want := "<ul><li>a</li><li>b</li><li>c</li></ul>"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestIfConditionTrue(t *testing.T) {
	html := `<if condition="show"><p>Visible</p></if>`
	got := parseAndRender(t, html, `{"show":true}`)
	if got != "<p>Visible</p>" {
		t.Errorf("got %q, want %q", got, "<p>Visible</p>")
	}
}

func TestIfConditionFalse(t *testing.T) {
	html := `<if condition="show"><p>Hidden</p></if>`
	got := parseAndRender(t, html, `{"show":false}`)
	if got != "" {
		t.Errorf("got %q, want empty string", got)
	}
}

func TestHTMLEscaping(t *testing.T) {
	html := "<div>{{content}}</div>"
	got := parseAndRender(t, html, `{"content":"<script>alert('xss')</script>"}`)
	if strings.Contains(got, "<script>") {
		t.Errorf("signal output must be HTML-escaped, got: %s", got)
	}
	if !strings.Contains(got, "&lt;script&gt;") {
		t.Errorf("expected escaped script tag, got: %s", got)
	}
}

func TestRawSignalUnescaped(t *testing.T) {
	html := "<div>{{{content}}}</div>"
	got := parseAndRender(t, html, `{"content":"<b>bold</b>"}`)
	if got != "<div><b>bold</b></div>" {
		t.Errorf("got %q, want %q", got, "<div><b>bold</b></div>")
	}
}

// ---------------------------------------------------------------------------
// Tests: error cases
// ---------------------------------------------------------------------------

func TestNullHTMLReturnsNull(t *testing.T) {
	cJSON := C.CString("{}")
	defer C.free(unsafe.Pointer(cJSON))

	ptr := C.webui_render(nil, cJSON)
	if ptr != nil {
		C.webui_free(ptr)
		t.Fatal("expected NULL for nil html")
	}

	err := lastError()
	if err == "" {
		t.Fatal("expected error message")
	}
	if !strings.Contains(err, "null") {
		t.Errorf("error should mention null, got: %s", err)
	}
}

func TestInvalidJSON(t *testing.T) {
	cHTML := C.CString("<p>hi</p>")
	defer C.free(unsafe.Pointer(cHTML))
	cJSON := C.CString("NOT JSON")
	defer C.free(unsafe.Pointer(cJSON))

	ptr := C.webui_render(cHTML, cJSON)
	if ptr != nil {
		C.webui_free(ptr)
		t.Fatal("expected NULL for invalid JSON")
	}

	err := lastError()
	if !strings.Contains(err, "JSON") {
		t.Errorf("error should mention JSON, got: %s", err)
	}
}

func TestSuccessfulCallClearsError(t *testing.T) {
	// Trigger error
	cHTML := C.CString("<p>hi</p>")
	cJSON := C.CString("NOT JSON")
	ptr := C.webui_render(cHTML, cJSON)
	if ptr != nil {
		C.webui_free(ptr)
	}
	C.free(unsafe.Pointer(cHTML))
	C.free(unsafe.Pointer(cJSON))

	if lastError() == "" {
		t.Fatal("error should be set")
	}

	// Successful call clears it
	_ = parseAndRender(t, "<p>ok</p>", "{}")
	if lastError() != "" {
		t.Error("error should be cleared after successful call")
	}
}

// ---------------------------------------------------------------------------
// Tests: handler lifecycle
// ---------------------------------------------------------------------------

func TestHandlerCreateAndDestroy(t *testing.T) {
	handler := C.webui_handler_create()
	if handler == nil {
		t.Fatal("handler should not be nil")
	}
	C.webui_handler_destroy(handler)
}

func TestHandlerDestroyNull(t *testing.T) {
	C.webui_handler_destroy(nil) // should not crash
}

func TestFreeStringNull(t *testing.T) {
	C.webui_free(nil) // should not crash
}

// ---------------------------------------------------------------------------
// Tests: fixture file
// ---------------------------------------------------------------------------

func TestFixtureFile(t *testing.T) {
	// Determine fixtures dir relative to this test file
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not determine test file location")
	}
	fixturesDir := filepath.Join(filepath.Dir(filename), "..", "fixtures")

	html, err := os.ReadFile(filepath.Join(fixturesDir, "simple.html"))
	if err != nil {
		t.Fatalf("reading simple.html: %v", err)
	}
	state, err := os.ReadFile(filepath.Join(fixturesDir, "state.json"))
	if err != nil {
		t.Fatalf("reading state.json: %v", err)
	}
	expected, err := os.ReadFile(filepath.Join(fixturesDir, "expected_output.html"))
	if err != nil {
		t.Fatalf("reading expected_output.html: %v", err)
	}

	got := parseAndRender(t, string(html), string(state))
	if got != string(expected) {
		t.Errorf("fixture mismatch:\ngot:  %q\nwant: %q", got, string(expected))
	}
}
