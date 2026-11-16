// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package webui_test

import (
	"bytes"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"runtime"
	"strings"
	"sync"
	"testing"

	webui "github.com/microsoft/webui/go/v3"
)

var testProtocol []byte

func TestMain(m *testing.M) {
	path := os.Getenv("WEBUI_GO_TEST_PROTOCOL")
	if path == "" {
		fmt.Fprintln(os.Stderr, "WEBUI_GO_TEST_PROTOCOL is required: build go/testdata/src with webui build --plugin webui and set it to the resulting protocol.bin")
		os.Exit(1)
	}
	var err error
	testProtocol, err = os.ReadFile(path)
	if err != nil || len(testProtocol) == 0 {
		fmt.Fprintf(os.Stderr, "read WEBUI_GO_TEST_PROTOCOL %q: error=%v, bytes=%d; rebuild go/testdata/src\n", path, err, len(testProtocol))
		os.Exit(1)
	}
	os.Exit(m.Run())
}

func newRenderer(t *testing.T) *webui.Renderer {
	t.Helper()
	r, err := webui.NewRenderer(testProtocol)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := r.Close(); err != nil {
			t.Error(err)
		}
	})
	return r
}

func requireNativeError(t *testing.T, err error, operation, contains string) *webui.NativeError {
	t.Helper()
	var native *webui.NativeError
	if !errors.As(err, &native) {
		t.Fatalf("expected NativeError, got %T: %v", err, err)
	}
	if native.Operation != operation || !strings.Contains(native.Message, contains) || native.Message == "" {
		t.Fatalf("unexpected native error: %#v", native)
	}
	if native.Error() != native.Operation+": "+native.Message {
		t.Fatalf("native diagnostic changed: %q", native.Error())
	}
	return native
}

func TestNewRenderer(t *testing.T) {
	for _, protocol := range [][]byte{nil, {}, {0xff, 0xff}} {
		r, err := webui.NewRenderer(protocol)
		if r != nil || err == nil {
			t.Fatalf("invalid protocol accepted: renderer=%v error=%v", r, err)
		}
		if len(protocol) > 0 {
			requireNativeError(t, err, "webui_protocol_create", "protobuf")
		}
	}
	protocol := bytes.Clone(testProtocol)
	r, err := webui.NewRenderer(protocol)
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	clear(protocol)
	runtime.GC()
	if _, err := r.Render([]byte(`{}`), webui.RenderOptions{}); err != nil {
		t.Fatalf("renderer retained the source protocol: %v", err)
	}
}

func TestRenderDefaultsEscapingAndPaths(t *testing.T) {
	r := newRenderer(t)
	state := []byte(`{"title":"Go café 世界","message":"<script>&\"' café 世界"}`)
	output, err := r.Render(state, webui.RenderOptions{})
	if err != nil {
		t.Fatal(err)
	}
	html := string(output)
	for _, want := range []string{
		"<title>Go café 世界</title>",
		"<h1>&lt;script&gt;&amp;&quot;&#x27; café 世界</h1>",
		"<p>home-route</p>",
		`id="webui-data"`,
	} {
		if !strings.Contains(html, want) {
			t.Errorf("missing %q in %s", want, html)
		}
	}
	if strings.Contains(html, "<h1><script>") || strings.Contains(html, "<p>about-route</p>") {
		t.Fatalf("unsafe escaping or wrong route: %s", html)
	}
	explicit, err := r.Render(state, webui.RenderOptions{Entry: "index.html", RequestPath: "/"})
	if err != nil || !bytes.Equal(output, explicit) {
		t.Fatalf("defaults differ from explicit options: %v", err)
	}
	about, err := r.Render(state, webui.RenderOptions{RequestPath: "/about"})
	if err != nil || !bytes.Contains(about, []byte("<p>about-route</p>")) || bytes.Contains(about, []byte("<p>home-route</p>")) {
		t.Fatalf("request path was not applied: %v, %s", err, about)
	}
	if err := r.Close(); err != nil {
		t.Fatal(err)
	}
	runtime.GC()
	if string(output) != html {
		t.Fatal("returned bytes changed after another render or Close")
	}
}

func TestRenderInvalidInput(t *testing.T) {
	r := newRenderer(t)
	for _, state := range [][]byte{nil, {}, []byte(`{`), []byte(`{"a":}`), {0xff}} {
		output, err := r.Render(state, webui.RenderOptions{})
		if output != nil {
			t.Fatalf("failed render returned bytes: %q", output)
		}
		requireNativeError(t, err, "webui_handler_render", "")
	}
	_, err := r.Render([]byte(`{}`), webui.RenderOptions{Entry: "不存在.html"})
	native := requireNativeError(t, err, "webui_handler_render", "不存在.html")
	saved := native.Message
	if _, err := r.Render([]byte(`{}`), webui.RenderOptions{}); err != nil {
		t.Fatal(err)
	}
	if native.Message != saved {
		t.Fatal("native error did not survive subsequent FFI calls")
	}
	for _, tc := range []struct {
		name    string
		state   []byte
		options webui.RenderOptions
	}{
		{"state", []byte("{}\x00invalid"), webui.RenderOptions{}},
		{"entry", []byte(`{}`), webui.RenderOptions{Entry: "index.html\x00bad"}},
		{"path", []byte(`{}`), webui.RenderOptions{RequestPath: "/\x00bad"}},
		{"nonce", []byte(`{}`), webui.RenderOptions{Nonce: "nonce\x00bad"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			output, err := r.Render(tc.state, tc.options)
			if output != nil || err == nil || !strings.Contains(err.Error(), "NUL") {
				t.Fatalf("embedded NUL accepted: %q, %v", output, err)
			}
		})
	}
	// Escaped JSON NUL is valid; output must be copied by length, not strlen.
	output, err := r.Render([]byte(`{"message":"before\u0000after"}`), webui.RenderOptions{})
	if err != nil || !bytes.Contains(output, []byte("before\x00after")) {
		t.Fatalf("escaped JSON NUL was not preserved: %q, %v", output, err)
	}
	for _, options := range []webui.RenderOptions{
		{Entry: string([]byte{0xff})},
		{RequestPath: string([]byte{0xff})},
	} {
		_, err := r.Render([]byte(`{}`), options)
		requireNativeError(t, err, "webui_handler_render", "UTF-8")
	}
	_, err = r.Render([]byte(`{}`), webui.RenderOptions{Nonce: string([]byte{0xff})})
	requireNativeError(t, err, "webui_handler_render_with_options", "UTF-8")
}

func TestRenderHTTPNonce(t *testing.T) {
	r := newRenderer(t)
	server := http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		nonce := request.URL.Query().Get("nonce")
		html, err := r.Render([]byte(`{}`), webui.RenderOptions{Nonce: nonce})
		if err != nil {
			t.Error(err)
			return
		}
		w.Header().Set("Content-Security-Policy", "script-src 'nonce-"+nonce+"'")
		_, _ = w.Write(html)
	})
	var wg sync.WaitGroup
	for worker := 0; worker < 24; worker++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			nonce := fmt.Sprintf("request-%d", worker)
			for iteration := 0; iteration < 10; iteration++ {
				response := httptest.NewRecorder()
				server.ServeHTTP(response, httptest.NewRequest(http.MethodGet, "/?nonce="+nonce, nil))
				html := response.Body.String()
				if got := response.Header().Get("Content-Security-Policy"); got != "script-src 'nonce-"+nonce+"'" {
					t.Errorf("wrong CSP header: %s", got)
				}
				for _, want := range []string{
					`<script nonce="` + nonce + `">`,
					`<meta name="webui-nonce" content="` + nonce + `">`,
				} {
					if !strings.Contains(html, want) {
						t.Errorf("CSP response missing %q in %s", want, html)
					}
				}
				withoutNonce, err := r.Render([]byte(`{}`), webui.RenderOptions{})
				if err != nil || bytes.Contains(withoutNonce, []byte(`nonce=`)) || bytes.Contains(withoutNonce, []byte(`webui-nonce`)) {
					t.Errorf("request nonce leaked into another render: %s, %v", withoutNonce, err)
				}
			}
		}()
	}
	wg.Wait()
}

func TestConcurrentRendersAndThreadLocalErrors(t *testing.T) {
	r := newRenderer(t)
	var wg sync.WaitGroup
	for worker := 0; worker < 24; worker++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for iteration := 0; iteration < 20; iteration++ {
				id := fmt.Sprintf("missing-世界-%d-%d.html", worker, iteration)
				_, err := r.Render([]byte(`{}`), webui.RenderOptions{Entry: id})
				requireNativeError(t, err, "webui_handler_render", id)
				state := fmt.Sprintf(`{"message":"worker-%d-%d"}`, worker, iteration)
				html, err := r.Render([]byte(state), webui.RenderOptions{})
				if err != nil || !bytes.Contains(html, []byte(fmt.Sprintf("worker-%d-%d", worker, iteration))) {
					t.Errorf("concurrent render: %s, %v", html, err)
					return
				}
			}
		}()
	}
	wg.Wait()
}

func TestCloseCopiesNilAndZero(t *testing.T) {
	var zero webui.Renderer
	for _, r := range []*webui.Renderer{nil, &zero} {
		if err := r.Close(); err != nil {
			t.Fatal(err)
		}
		if _, err := r.Render(nil, webui.RenderOptions{}); !errors.Is(err, webui.ErrClosed) {
			t.Fatalf("uninitialized Render: %v", err)
		}
	}
	r := newRenderer(t)
	copied := *r
	if err := copied.Close(); err != nil {
		t.Fatal(err)
	}
	for _, renderer := range []*webui.Renderer{r, &copied} {
		if _, err := renderer.Render([]byte(`{}`), webui.RenderOptions{}); !errors.Is(err, webui.ErrClosed) {
			t.Fatalf("copy did not share closure: %v", err)
		}
		for i := 0; i < 3; i++ {
			if err := renderer.Close(); err != nil {
				t.Fatal(err)
			}
		}
	}
}

func TestConcurrentCloseAndRender(t *testing.T) {
	for cycle := 0; cycle < 10; cycle++ {
		r := newRenderer(t)
		copied := *r
		start := make(chan struct{})
		var wg sync.WaitGroup
		for worker := 0; worker < 16; worker++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				for iteration := 0; iteration < 10; iteration++ {
					html, err := copied.Render([]byte(`{"message":"lifecycle"}`), webui.RenderOptions{})
					if errors.Is(err, webui.ErrClosed) {
						return
					}
					if err != nil || !bytes.Contains(html, []byte("lifecycle")) {
						t.Errorf("render during Close: %s, %v", html, err)
						return
					}
				}
			}()
		}
		for closer := 0; closer < 4; closer++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				if err := r.Close(); err != nil {
					t.Error(err)
				}
			}()
		}
		close(start)
		wg.Wait()
		if _, err := r.Render([]byte(`{}`), webui.RenderOptions{}); !errors.Is(err, webui.ErrClosed) {
			t.Fatalf("Render after concurrent Close: %v", err)
		}
	}
}
