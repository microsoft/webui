// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package main

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	webui "__WEBUI_GO_MODULE__"
)

func TestShutdownDrainsActiveRequests(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	entered, release, shutdown := make(chan struct{}), make(chan struct{}), make(chan struct{})
	var once sync.Once
	unblock := func() { once.Do(func() { close(release) }) }
	defer unblock()
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		close(entered)
		<-release
		if _, err := io.WriteString(w, "drained"); err != nil {
			t.Errorf("write active response: %v", err)
		}
	})}
	defer server.Close()
	server.RegisterOnShutdown(func() { close(shutdown) })
	served := make(chan error, 1)
	go func() { served <- serve(ctx, server, listener) }()
	client := &http.Client{Timeout: 5 * time.Second}
	defer client.CloseIdleConnections()
	response := make(chan error, 1)
	go func() {
		result, err := client.Get("http://" + listener.Addr().String())
		if err == nil {
			defer result.Body.Close()
			var body []byte
			body, err = io.ReadAll(result.Body)
			if err == nil && (result.StatusCode != 200 || string(body) != "drained") {
				err = fmt.Errorf("active response lost during shutdown: status %d, body %q", result.StatusCode, body)
			}
		}
		response <- err
	}()
	await := func(event <-chan struct{}, label string) {
		t.Helper()
		select {
		case <-event:
		case <-time.After(5 * time.Second):
			t.Fatalf("timed out waiting for %s", label)
		}
	}
	await(entered, "active request")
	cancel()
	await(shutdown, "shutdown admission close")
	select {
	case err := <-served:
		t.Fatalf("server returned before draining active request: %v", err)
	default:
	}
	if connection, err := net.DialTimeout("tcp", listener.Addr().String(), time.Second); err == nil {
		connection.Close()
		t.Fatal("shutdown still accepts new connections")
	}
	unblock()
	for _, result := range []<-chan error{response, served} {
		select {
		case err := <-result:
			if err != nil {
				t.Fatal(err)
			}
		case <-time.After(5 * time.Second):
			t.Fatal("shutdown did not finish after the active request completed")
		}
	}
}

func TestPublicSnapshot(t *testing.T) {
	directory, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	public := filepath.Join(directory, "public")
	if err := os.MkdirAll(filepath.Join(public, "nested"), 0700); err != nil {
		t.Fatal(err)
	}
	for name, content := range map[string]string{
		"nested/app.js": "export{}", ".private.js": "private",
		"protocol.bin": "private", "source.ts": "private",
	} {
		if err := os.WriteFile(filepath.Join(public, name), []byte(content), 0600); err != nil {
			t.Fatal(err)
		}
	}
	secret := filepath.Join(directory, "secret.js")
	if err := os.WriteFile(secret, []byte("secret"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(secret, filepath.Join(public, "link.js")); err != nil {
		t.Fatal(err)
	}
	assets, err := snapshotAssets(public)
	if err != nil {
		t.Fatal(err)
	}
	if len(assets) != 1 || string(assets["/assets/nested/app.js"].bytes) != "export{}" {
		t.Fatalf("unexpected public snapshot: %#v", assets)
	}
	handler := appHandler(nil, assets)
	for _, path := range []string{
		"/assets/link.js", "/assets/.private.js", "/assets/protocol.bin",
		"/assets/source.ts", "/assets/../secret.js", "/assets/%2e%2e/secret.js",
		"/dist/protocol.bin", "/server/main.go",
	} {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest("GET", path, nil))
		if response.Code != 404 {
			t.Errorf("%s returned %d, want 404", path, response.Code)
		}
	}
	// Published data is immutable even if files are changed or replaced by links.
	if err := os.WriteFile(filepath.Join(public, "nested/app.js"), []byte("changed"), 0600); err != nil {
		t.Fatal(err)
	}
	for _, method := range []string{"GET", "HEAD"} {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest(method, "/assets/nested/app.js", nil))
		if response.Code != 200 || response.Header().Get("Content-Type") != "text/javascript; charset=utf-8" ||
			response.Header().Get("Content-Length") != "8" {
			t.Fatalf("unexpected asset response: %#v", response.Result())
		}
		if method == "HEAD" && response.Body.Len() != 0 {
			t.Fatal("HEAD returned a body")
		}
		if method == "GET" && response.Body.String() != "export{}" {
			t.Fatal("snapshot changed")
		}
	}
}

func TestMethodsAndErrors(t *testing.T) {
	handler := appHandler(nil, nil)
	for _, method := range []string{"POST", "PUT", "DELETE", "OPTIONS"} {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest(method, "/", nil))
		if response.Code != 405 || response.Header().Get("Allow") != "GET, HEAD" {
			t.Fatalf("%s: unexpected response %#v", method, response.Result())
		}
	}
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest("HEAD", "/missing", nil))
	if response.Code != 404 || response.Body.Len() != 0 || response.Header().Get("Content-Length") == "" {
		t.Fatal("HEAD error response must have GET headers and no body")
	}
	stopped := &webui.Renderer{}
	response = httptest.NewRecorder()
	appHandler(stopped, nil).ServeHTTP(response, httptest.NewRequest("GET", "/", nil))
	if response.Code != 500 || !strings.Contains(response.Body.String(), "Internal server error") {
		t.Fatal("render failure did not produce a generic 500")
	}
}

func TestNativeErrorsAndClose(t *testing.T) {
	if _, err := webui.NewRenderer(nil); err == nil {
		t.Fatal("empty protocol accepted")
	}
	// Multiple cgo callers must independently copy thread-local diagnostics.
	var workers sync.WaitGroup
	for range 16 {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for range 20 {
				if _, err := webui.NewRenderer([]byte{0xff}); err == nil || err.Error() == "" {
					t.Error("malformed protocol accepted without an error")
				}
			}
		}()
	}
	workers.Wait()
	r := &webui.Renderer{}
	r.Close()
	r.Close()
	if _, err := r.Render([]byte("{}"), webui.RenderOptions{}); err == nil {
		t.Fatal("render after close succeeded")
	}
}
