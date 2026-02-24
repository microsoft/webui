package webui_ffi

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

func mustRender(t *testing.T, html, dataJSON string) string {
	t.Helper()
	result, err := Render(html, dataJSON)
	if err != nil {
		t.Fatalf("Render returned error: %s", err)
	}
	return result
}

// ---------------------------------------------------------------------------
// Tests: happy paths
// ---------------------------------------------------------------------------

func TestSimplePassthrough(t *testing.T) {
	got := mustRender(t, "<p>Hello</p>", "{}")
	if got != "<p>Hello</p>" {
		t.Errorf("got %q, want %q", got, "<p>Hello</p>")
	}
}

func TestSignalSubstitution(t *testing.T) {
	got := mustRender(t, "Hello, {{name}}!", `{"name":"WebUI"}`)
	if got != "Hello, WebUI!" {
		t.Errorf("got %q, want %q", got, "Hello, WebUI!")
	}
}

func TestForLoop(t *testing.T) {
	html := `<ul><for each="item in items"><li>{{item}}</li></for></ul>`
	got := mustRender(t, html, `{"items":["a","b","c"]}`)
	want := "<ul><li>a</li><li>b</li><li>c</li></ul>"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestIfConditionTrue(t *testing.T) {
	html := `<if condition="show"><p>Visible</p></if>`
	got := mustRender(t, html, `{"show":true}`)
	if got != "<p>Visible</p>" {
		t.Errorf("got %q, want %q", got, "<p>Visible</p>")
	}
}

func TestIfConditionFalse(t *testing.T) {
	html := `<if condition="show"><p>Hidden</p></if>`
	got := mustRender(t, html, `{"show":false}`)
	if got != "" {
		t.Errorf("got %q, want empty string", got)
	}
}

func TestHTMLEscaping(t *testing.T) {
	html := "<div>{{content}}</div>"
	got := mustRender(t, html, `{"content":"<script>alert('xss')</script>"}`)
	if strings.Contains(got, "<script>") {
		t.Errorf("signal output must be HTML-escaped, got: %s", got)
	}
	if !strings.Contains(got, "&lt;script&gt;") {
		t.Errorf("expected escaped script tag, got: %s", got)
	}
}

func TestRawSignalUnescaped(t *testing.T) {
	html := "<div>{{{content}}}</div>"
	got := mustRender(t, html, `{"content":"<b>bold</b>"}`)
	if got != "<div><b>bold</b></div>" {
		t.Errorf("got %q, want %q", got, "<div><b>bold</b></div>")
	}
}

// ---------------------------------------------------------------------------
// Tests: error cases
// ---------------------------------------------------------------------------

func TestNullHTMLReturnsNull(t *testing.T) {
	cJSON := CString("{}")
	defer CFree(unsafe.Pointer(cJSON))

	ptr := RenderRaw(nil, cJSON)
	if ptr != nil {
		Free(ptr)
		t.Fatal("expected NULL for nil html")
	}

	err := LastError()
	if err == "" {
		t.Fatal("expected error message")
	}
	if !strings.Contains(err, "null") {
		t.Errorf("error should mention null, got: %s", err)
	}
}

func TestInvalidJSON(t *testing.T) {
	cHTML := CString("<p>hi</p>")
	defer CFree(unsafe.Pointer(cHTML))
	cJSON := CString("NOT JSON")
	defer CFree(unsafe.Pointer(cJSON))

	ptr := RenderRaw(cHTML, cJSON)
	if ptr != nil {
		Free(ptr)
		t.Fatal("expected NULL for invalid JSON")
	}

	err := LastError()
	if !strings.Contains(err, "JSON") {
		t.Errorf("error should mention JSON, got: %s", err)
	}
}

func TestSuccessfulCallClearsError(t *testing.T) {
	// Trigger error
	cHTML := CString("<p>hi</p>")
	cJSON := CString("NOT JSON")
	ptr := RenderRaw(cHTML, cJSON)
	if ptr != nil {
		Free(ptr)
	}
	CFree(unsafe.Pointer(cHTML))
	CFree(unsafe.Pointer(cJSON))

	if LastError() == "" {
		t.Fatal("error should be set")
	}

	// Successful call clears it
	_ = mustRender(t, "<p>ok</p>", "{}")
	if LastError() != "" {
		t.Error("error should be cleared after successful call")
	}
}

// ---------------------------------------------------------------------------
// Tests: handler lifecycle
// ---------------------------------------------------------------------------

func TestHandlerCreateAndDestroy(t *testing.T) {
	handler := HandlerCreate()
	if handler == nil {
		t.Fatal("handler should not be nil")
	}
	HandlerDestroy(handler)
}

func TestHandlerDestroyNull(t *testing.T) {
	HandlerDestroy(nil) // should not crash
}

func TestFreeNull(t *testing.T) {
	Free(nil) // should not crash
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

	got := mustRender(t, string(html), string(state))
	if got != string(expected) {
		t.Errorf("fixture mismatch:\ngot:  %q\nwant: %q", got, string(expected))
	}
}
