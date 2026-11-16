// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package main

import (
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strings"

	webui "github.com/microsoft/webui/go/v3"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	protocol, err := os.ReadFile(os.Getenv("WEBUI_GO_TEST_PROTOCOL"))
	if err != nil {
		return err
	}
	renderer, err := webui.NewRenderer(protocol)
	if err != nil {
		return err
	}
	defer renderer.Close()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return err
	}
	server := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		html, err := renderer.Render([]byte(`{"title":"External Go consumer","message":"Go & Rust"}`),
			webui.RenderOptions{RequestPath: r.URL.Path})
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		if _, err := w.Write(html); err != nil {
			fmt.Fprintln(os.Stderr, err)
		}
	})}
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	defer server.Close()
	for route, marker := range map[string]string{"/": "home-route", "/about": "about-route"} {
		response, err := http.Get("http://" + listener.Addr().String() + route)
		if err != nil {
			return err
		}
		body, readErr := io.ReadAll(response.Body)
		closeErr := response.Body.Close()
		if readErr != nil {
			return readErr
		}
		if closeErr != nil {
			return closeErr
		}
		if response.StatusCode != http.StatusOK ||
			response.Header.Get("Content-Type") != "text/html; charset=utf-8" ||
			!strings.Contains(string(body), "Go &amp; Rust") || !strings.Contains(string(body), marker) {
			return fmt.Errorf("unexpected response for %s: %d %s", route, response.StatusCode, body)
		}
	}
	if err := server.Close(); err != nil {
		return err
	}
	if err := <-done; err != http.ErrServerClosed {
		return fmt.Errorf("HTTP server exit: %v", err)
	}
	fmt.Println("external Go consumer: " + webui.Version)
	return nil
}
