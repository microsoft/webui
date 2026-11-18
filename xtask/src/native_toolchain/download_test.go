// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package main

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestHTTPSOriginAndRedirectPolicy(t *testing.T) {
	for _, raw := range []string{
		"https://github.com/microsoft/TypeScript/releases/download/v7.0.2/typescript-darwin-arm64.tgz",
		"https://release-assets.githubusercontent.com/asset?signature=public-release",
	} {
		address, err := url.Parse(raw)
		if err != nil || allowedURL(address) != nil {
			t.Fatalf("approved URL rejected: %s (%v)", raw, err)
		}
	}
	for _, raw := range []string{
		"http://github.com/asset", "https://example.com/asset",
		"https://user:password@github.com/asset", "https://github.com:444/asset",
		"https://github.com/asset#fragment",
	} {
		address, err := url.Parse(raw)
		if err != nil {
			t.Fatal(err)
		}
		if allowedURL(address) == nil {
			t.Fatalf("unsafe URL accepted: %s", raw)
		}
	}
	address, _ := url.Parse("https://github.com/asset")
	if redirect(&http.Request{URL: address}, make([]*http.Request, 10)) == nil {
		t.Fatal("redirect limit not enforced")
	}
	if acquire("https://github.com/other/repository/asset", filepath.Join(t.TempDir(), "out")) == nil {
		t.Fatal("non-TypeScript initial source was accepted")
	}
}

func TestTLSDownloadBoundsStatusAndPartialCleanup(t *testing.T) {
	for _, test := range []struct {
		name   string
		status int
		body   string
		stream bool
		pass   bool
	}{
		{"valid", 200, "genuine transport fixture", false, true},
		{"not-found", 404, "missing", false, false},
		{"known-large", 200, strings.Repeat("x", 65), false, false},
		{"stream-large", 200, strings.Repeat("x", 65), true, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.WriteHeader(test.status)
				if test.stream {
					w.(http.Flusher).Flush()
				}
				_, _ = w.Write([]byte(test.body))
			}))
			defer server.Close()
			output := filepath.Join(t.TempDir(), "artifact")
			err := download(server.Client(), server.URL, output, 64)
			if (err == nil) != test.pass {
				t.Fatalf("unexpected result: %v", err)
			}
			data, readErr := os.ReadFile(output)
			if test.pass && (readErr != nil || string(data) != test.body) {
				t.Fatalf("download mismatch: %v", readErr)
			}
			if !test.pass && !os.IsNotExist(readErr) {
				t.Fatal("failed download left an output artifact")
			}
		})
	}
}

func TestUntrustedTLSAndExistingOutputAreRejected(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("fixture"))
	}))
	defer server.Close()
	output := filepath.Join(t.TempDir(), "artifact")
	if download(client(), server.URL, output, 64) == nil {
		t.Fatal("untrusted TLS certificate was accepted")
	}
	if err := os.WriteFile(output, []byte("preserve"), 0600); err != nil {
		t.Fatal(err)
	}
	if download(server.Client(), server.URL, output, 64) == nil {
		t.Fatal("existing output was overwritten")
	}
	data, err := os.ReadFile(output)
	if err != nil || string(data) != "preserve" {
		t.Fatalf("existing output changed: %v", err)
	}
}
